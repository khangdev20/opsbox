//! HTTP (:80) and HTTPS (:443) reverse proxy data plane:
//! - ACME HTTP-01 challenge responses (`/.well-known/acme-challenge/<token>`)
//! - HTTP -> HTTPS and optional `www.` redirects
//! - Dynamic SNI TLS termination (`rustls`)
//! - Upstream forwarding (`http://` or pinned `https://`) with WebSocket upgrades,
//!   HSTS, body size limits, timeouts, and `X-Forwarded-*` headers.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Full, Limited, combinators::BoxBody};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, CONTENT_LENGTH, CONTENT_TYPE, HOST, LOCATION, UPGRADE};
use hyper::http::uri::PathAndQuery;
use hyper::{HeaderMap, Request, Response, StatusCode, Uri};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature};
use tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, RootCertStore, SignatureScheme,
};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::{debug, warn};

use crate::acme::{ChallengeMap, DynamicCertResolver};
use crate::proto::{Route, UpstreamTls};

pub const DEFAULT_MAX_BODY_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2 * 3600);
const CHALLENGE_PREFIX: &str = "/.well-known/acme-challenge/";

pub type ProxyBody = BoxBody<Bytes, hyper::Error>;

#[derive(Clone, Default)]
pub struct RouteTable {
    inner: Arc<RwLock<HashMap<String, Route>>>,
}

impl RouteTable {
    pub fn set(&self, routes: Vec<Route>) {
        let mut map = HashMap::new();
        for mut r in routes {
            r.host = r.host.to_ascii_lowercase();
            map.insert(r.host.clone(), r);
        }
        let mut guard = self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = map;
    }

    pub fn snapshot(&self) -> Vec<Route> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.values().cloned().collect()
    }

    pub fn count(&self) -> usize {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.len()
    }

    fn lookup(&self, host: &str) -> Option<(Route, bool)> {
        let guard = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(r) = guard.get(host) {
            return Some((r.clone(), false));
        }
        if let Some(bare) = host.strip_prefix("www.")
            && let Some(r) = guard.get(bare)
            && r.redirect_www
        {
            return Some((r.clone(), true));
        }
        None
    }
}

fn empty_body() -> ProxyBody {
    Empty::<Bytes>::new()
        .map_err(|never: Infallible| match never {})
        .boxed()
}

fn text_response(status: StatusCode, body: &'static str) -> Response<ProxyBody> {
    let full = Full::new(Bytes::from_static(body.as_bytes()))
        .map_err(|never: Infallible| match never {})
        .boxed();
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(full)
        .unwrap_or_else(|_| Response::new(empty_body()))
}

#[derive(Clone, Copy)]
pub struct ListenerPorts {
    pub http: u16,
    pub https: u16,
}
fn host_with_port(host: &str, port: u16, standard: u16) -> String {
    if port == standard {
        host.to_owned()
    } else {
        format!("{host}:{port}")
    }
}
fn redirect_https_response(
    target_host: &str,
    pq: &str,
    hsts: bool,
    port: u16,
) -> Response<ProxyBody> {
    let target_host = host_with_port(target_host, port, 443);
    let location = format!("https://{target_host}{pq}");
    let mut builder = Response::builder()
        .status(StatusCode::PERMANENT_REDIRECT)
        .header(LOCATION, location);
    if hsts {
        builder = builder.header("strict-transport-security", "max-age=31536000");
    }
    builder
        .body(empty_body())
        .unwrap_or_else(|_| Response::new(empty_body()))
}

/// Extract lowercase hostname without port from the `Host` header (or request URI).
pub fn extract_host(req: &Request<Incoming>) -> Option<String> {
    let raw = req
        .headers()
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            req.uri()
                .authority()
                .map(hyper::http::uri::Authority::as_str)
        })?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bare = if let Some(rest) = trimmed.strip_prefix('[') {
        rest.split(']').next()?
    } else {
        trimmed.split(':').next()?
    };
    (!bare.is_empty()).then(|| bare.to_ascii_lowercase())
}

fn valid_acme_token(token: &str) -> bool {
    !token.is_empty()
        && token.len() <= 128
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Handle one plain HTTP (:80) request: ACME HTTP-01 challenge, HTTPS redirect, or plain HTTP proxy.
pub async fn handle_http(
    req: Request<Incoming>,
    peer: SocketAddr,
    routes: RouteTable,
    challenges: ChallengeMap,
    certs: Arc<DynamicCertResolver>,
    default_max_body: u64,
) -> Result<Response<ProxyBody>, Infallible> {
    handle_http_on_ports(
        req,
        peer,
        routes,
        challenges,
        certs,
        default_max_body,
        ListenerPorts {
            http: 80,
            https: 443,
        },
    )
    .await
}
async fn handle_http_on_ports(
    req: Request<Incoming>,
    peer: SocketAddr,
    routes: RouteTable,
    challenges: ChallengeMap,
    certs: Arc<DynamicCertResolver>,
    default_max_body: u64,
    ports: ListenerPorts,
) -> Result<Response<ProxyBody>, Infallible> {
    let path = req.uri().path();
    if let Some(token) = path.strip_prefix(CHALLENGE_PREFIX) {
        if valid_acme_token(token)
            && let Some(key_auth) = challenges.get(token)
        {
            let body = Full::new(Bytes::from(key_auth))
                .map_err(|never: Infallible| match never {})
                .boxed();
            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/octet-stream")
                .body(body)
                .unwrap_or_else(|_| Response::new(empty_body())));
        }
        return Ok(text_response(StatusCode::NOT_FOUND, "Challenge not found."));
    }

    let Some(host) = extract_host(&req) else {
        return Ok(text_response(
            StatusCode::BAD_REQUEST,
            "Missing Host header.",
        ));
    };
    let Some((route, is_www)) = routes.lookup(&host) else {
        return Ok(text_response(StatusCode::NOT_FOUND, "Unknown host."));
    };
    let pq = req.uri().path_and_query().map_or("/", PathAndQuery::as_str);
    if let Some(ref target) = route.redirect_to {
        if route.redirect_https && (certs.has(&route.host) || certs.has(target)) {
            return Ok(redirect_https_response(target, pq, false, ports.https));
        }
        let target = host_with_port(target, ports.http, 80);
        let location = format!("http://{target}{pq}");
        return Ok(Response::builder()
            .status(StatusCode::MOVED_PERMANENTLY)
            .header(LOCATION, location)
            .body(empty_body())
            .unwrap_or_else(|_| Response::new(empty_body())));
    }
    if route.redirect_https && certs.has(&route.host) {
        return Ok(redirect_https_response(&route.host, pq, false, ports.https));
    }
    if is_www {
        let location = format!(
            "http://{}{}",
            host_with_port(&route.host, ports.http, 80),
            pq
        );
        return Ok(Response::builder()
            .status(StatusCode::MOVED_PERMANENTLY)
            .header(LOCATION, location)
            .body(empty_body())
            .unwrap_or_else(|_| Response::new(empty_body())));
    }
    forward_request(req, peer, &route, "http", default_max_body).await
}

#[derive(Debug)]
struct PinnedCertVerifier {
    expected_sha256: [u8; 32],
}

impl ServerCertVerifier for PinnedCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let digest = Sha256::digest(end_entity.as_ref());
        if digest.as_slice() == self.expected_sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(RustlsError::General(
                "upstream certificate fingerprint does not match expected panel certificate".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls_pki_types::CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls_pki_types::CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

pub fn decode_sha256_hex(hex: &str) -> Option<[u8; 32]> {
    let s = hex.trim();
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        let _ = write!(s, "{b:02x}");
    }
    s
}

enum UpstreamStream {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for UpstreamStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            Self::Tls(s) => std::pin::Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for UpstreamStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        match &mut *self {
            Self::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            Self::Tls(s) => std::pin::Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match &mut *self {
            Self::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            Self::Tls(s) => std::pin::Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match &mut *self {
            Self::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            Self::Tls(s) => std::pin::Pin::new(s).poll_shutdown(cx),
        }
    }
}

async fn connect_upstream(addr: SocketAddr, tls: &UpstreamTls) -> Result<UpstreamStream, String> {
    let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| "upstream connect timed out".to_owned())?
        .map_err(|e| format!("upstream connect failed: {e}"))?;

    match tls {
        UpstreamTls::Plain => Ok(UpstreamStream::Plain(tcp)),
        UpstreamTls::Https { cert_sha256 } => {
            let provider = Arc::new(ring::default_provider());
            let cfg = if let Some(hex) = cert_sha256 {
                let expected = decode_sha256_hex(hex)
                    .ok_or_else(|| "invalid pinned upstream certificate fingerprint".to_owned())?;
                ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(|e| e.to_string())?
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(PinnedCertVerifier {
                        expected_sha256: expected,
                    }))
                    .with_no_client_auth()
            } else {
                let roots = RootCertStore {
                    roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                };
                ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(|e| e.to_string())?
                    .with_root_certificates(roots)
                    .with_no_client_auth()
            };
            let connector = TlsConnector::from(Arc::new(cfg));
            let sni = rustls_pki_types::ServerName::IpAddress(addr.ip().into());
            let tls_stream = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(sni, tcp))
                .await
                .map_err(|_| "upstream TLS handshake timed out".to_owned())?
                .map_err(|e| format!("upstream TLS handshake failed: {e}"))?;
            Ok(UpstreamStream::Tls(Box::new(tls_stream)))
        }
    }
}

fn is_hop_by_hop(name: &str, connection_tokens: &[String]) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
    ) || connection_tokens.iter().any(|t| t == name)
}

fn connection_header_tokens(headers: &HeaderMap) -> Vec<String> {
    headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Handle one HTTPS (:443) request and reverse-proxy it to the matched upstream.
pub async fn handle_https(
    req: Request<Incoming>,
    peer: SocketAddr,
    routes: RouteTable,
    default_max_body: u64,
) -> Result<Response<ProxyBody>, Infallible> {
    handle_https_on_port(req, peer, routes, default_max_body, 443).await
}
async fn handle_https_on_port(
    req: Request<Incoming>,
    peer: SocketAddr,
    routes: RouteTable,
    default_max_body: u64,
    https_port: u16,
) -> Result<Response<ProxyBody>, Infallible> {
    let Some(host) = extract_host(&req) else {
        return Ok(text_response(
            StatusCode::BAD_REQUEST,
            "Missing Host header.",
        ));
    };
    let Some((route, is_www)) = routes.lookup(&host) else {
        return Ok(text_response(StatusCode::NOT_FOUND, "Unknown host."));
    };
    if let Some(ref target) = route.redirect_to {
        let pq = req.uri().path_and_query().map_or("/", PathAndQuery::as_str);
        return Ok(redirect_https_response(target, pq, route.hsts, https_port));
    }
    if is_www {
        let pq = req.uri().path_and_query().map_or("/", PathAndQuery::as_str);
        return Ok(redirect_https_response(
            &route.host,
            pq,
            route.hsts,
            https_port,
        ));
    }
    forward_request(req, peer, &route, "https", default_max_body).await
}

async fn forward_request(
    mut req: Request<Incoming>,
    peer: SocketAddr,
    route: &Route,
    proto: &str,
    default_max_body: u64,
) -> Result<Response<ProxyBody>, Infallible> {
    let max_body = if route.max_body_bytes > 0 {
        route.max_body_bytes
    } else {
        default_max_body
    };
    if let Some(cl) = req
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        && cl > max_body
    {
        return Ok(text_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Request body exceeds the proxy limit.",
        ));
    }

    let upstream_addr: SocketAddr = match route.upstream.parse() {
        Ok(a) => a,
        Err(_) => {
            return Ok(text_response(
                StatusCode::BAD_GATEWAY,
                "Invalid upstream address.",
            ));
        }
    };

    let conn_tokens = connection_header_tokens(req.headers());
    let is_upgrade =
        conn_tokens.iter().any(|t| t == "upgrade") && req.headers().contains_key(UPGRADE);
    let client_upgrade = is_upgrade.then(|| hyper::upgrade::on(&mut req));

    let original_host = req
        .headers()
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned);
    let existing_xff = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let (mut parts, incoming_body) = req.into_parts();
    let pq = parts
        .uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| PathAndQuery::from_static("/"));
    parts.uri = Uri::from(pq);
    parts.version = hyper::Version::HTTP_11;

    let mut clean_headers = HeaderMap::new();
    for (name, value) in &parts.headers {
        let lname = name.as_str().to_ascii_lowercase();
        if lname == "x-forwarded-for" || lname == "x-forwarded-proto" || lname == "x-forwarded-host"
        {
            continue;
        }
        if is_hop_by_hop(&lname, &conn_tokens) {
            if is_upgrade && (lname == "connection" || lname == "upgrade") {
                clean_headers.append(name.clone(), value.clone());
            }
            continue;
        }
        clean_headers.append(name.clone(), value.clone());
    }
    let peer_ip = peer.ip().to_string();
    let xff_val = match existing_xff {
        Some(prev) => format!("{prev}, {peer_ip}"),
        None => peer_ip,
    };
    if let Ok(v) = xff_val.parse() {
        clean_headers.insert("x-forwarded-for", v);
    }
    clean_headers.insert(
        "x-forwarded-proto",
        proto
            .parse()
            .unwrap_or_else(|_| "https".parse().expect("valid")),
    );
    if let Some(ref h) = original_host
        && let Ok(v) = h.parse()
    {
        clean_headers.insert("x-forwarded-host", v);
    }
    parts.headers = clean_headers;

    let max_usize = usize::try_from(max_body).unwrap_or(usize::MAX);
    let limited_body = Limited::new(incoming_body, max_usize).boxed();
    let upstream_req = Request::from_parts(parts, limited_body);

    let stream = match connect_upstream(upstream_addr, &route.upstream_tls).await {
        Ok(s) => s,
        Err(e) => {
            warn!(upstream = %upstream_addr, error = %e, "proxy upstream connect failed");
            return Ok(text_response(
                StatusCode::BAD_GATEWAY,
                "The upstream service is unavailable.",
            ));
        }
    };

    let (mut sender, conn) = match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await
    {
        Ok(parts) => parts,
        Err(e) => {
            warn!(upstream = %upstream_addr, error = %e, "upstream HTTP handshake failed");
            return Ok(text_response(
                StatusCode::BAD_GATEWAY,
                "Upstream HTTP handshake failed.",
            ));
        }
    };
    tokio::spawn(async move {
        if let Err(e) = conn.with_upgrades().await {
            debug!(error = %e, "upstream connection ended");
        }
    });

    let mut upstream_resp =
        match tokio::time::timeout(TRANSFER_TIMEOUT, sender.send_request(upstream_req)).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                warn!(upstream = %upstream_addr, error = %e, "upstream request failed");
                return Ok(text_response(
                    StatusCode::BAD_GATEWAY,
                    "Upstream request failed.",
                ));
            }
            Err(_) => {
                return Ok(text_response(
                    StatusCode::GATEWAY_TIMEOUT,
                    "Upstream request timed out.",
                ));
            }
        };

    if upstream_resp.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(on_client) = client_upgrade
    {
        let on_upstream = hyper::upgrade::on(&mut upstream_resp);
        tokio::spawn(async move {
            match tokio::try_join!(on_client, on_upstream) {
                Ok((client_io, upstream_io)) => {
                    let mut c = TokioIo::new(client_io);
                    let mut u = TokioIo::new(upstream_io);
                    let _ = tokio::io::copy_bidirectional(&mut c, &mut u).await;
                }
                Err(e) => {
                    debug!(error = %e, "websocket tunnel upgrade failed");
                }
            }
        });
        let (mut resp_parts, _) = upstream_resp.into_parts();
        if proto == "https"
            && route.hsts
            && !resp_parts.headers.contains_key("strict-transport-security")
        {
            resp_parts.headers.insert(
                "strict-transport-security",
                "max-age=31536000".parse().expect("valid"),
            );
        }
        return Ok(Response::from_parts(resp_parts, empty_body()));
    }

    let resp_conn_tokens = connection_header_tokens(upstream_resp.headers());
    let (mut resp_parts, resp_body) = upstream_resp.into_parts();
    let keys: Vec<_> = resp_parts
        .headers
        .keys()
        .filter(|k| is_hop_by_hop(&k.as_str().to_ascii_lowercase(), &resp_conn_tokens))
        .cloned()
        .collect();
    for k in keys {
        resp_parts.headers.remove(&k);
    }
    if proto == "https"
        && route.hsts
        && !resp_parts.headers.contains_key("strict-transport-security")
    {
        resp_parts.headers.insert(
            "strict-transport-security",
            "max-age=31536000".parse().expect("valid"),
        );
    }
    Ok(Response::from_parts(resp_parts, resp_body.boxed()))
}

pub async fn serve_http_connection(
    stream: TcpStream,
    peer: SocketAddr,
    routes: RouteTable,
    challenges: ChallengeMap,
    certs: Arc<DynamicCertResolver>,
    default_max_body: u64,
    ports: ListenerPorts,
) {
    let io = TokioIo::new(stream);
    let svc = hyper::service::service_fn(move |req| {
        handle_http_on_ports(
            req,
            peer,
            routes.clone(),
            challenges.clone(),
            Arc::clone(&certs),
            default_max_body,
            ports,
        )
    });
    let _ = ConnBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(io, svc)
        .await;
}

pub async fn serve_https_connection(
    stream: TcpStream,
    peer: SocketAddr,
    acceptor: TlsAcceptor,
    routes: RouteTable,
    default_max_body: u64,
    https_port: u16,
) {
    let Ok(tls_stream) = acceptor.accept(stream).await else {
        return;
    };
    let io = TokioIo::new(tls_stream);
    let svc = hyper::service::service_fn(move |req| {
        handle_https_on_port(req, peer, routes.clone(), default_max_body, https_port)
    });
    let _ = ConnBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(io, svc)
        .await;
}

#[cfg(test)]
mod listener_port_tests {
    use super::*;
    #[test]
    fn redirects_use_serving_ports() {
        let response = redirect_https_response("app.example.com", "/test?a=1", false, 8445);
        assert_eq!(
            response.headers()[LOCATION],
            "https://app.example.com:8445/test?a=1"
        );
        assert_eq!(
            host_with_port("app.example.com", 8081, 80),
            "app.example.com:8081"
        );
        assert_eq!(
            host_with_port("app.example.com", 443, 443),
            "app.example.com"
        );
    }
}
