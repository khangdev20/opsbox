//! End-to-end integration test for `opsbox-proxy`:
//! - Real `PrivServer` encrypting/decrypting ACME account & TLS keys via `master.key`
//! - Real `ProxyServer` (`:80` HTTP + `:443` HTTPS + Unix control socket)
//! - In-process RFC 8555 ACME server verifying HTTP-01 over `ProxyServer`'s HTTP listener
//!   and signing real CA-backed certificates (short-lived 20d then 90d renewal!)
//! - HTTPS routing to a TLS upstream with pinned SHA-256 certificate, HTTP->HTTPS
//!   redirect, HSTS, X-Forwarded-* headers, preserved Host, WebSocket upgrade,
//!   30-day renewal, and encrypted persistence across proxy restarts.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::os::unix::fs::MetadataExt;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{CONTENT_TYPE, LOCATION};
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use instant_acme::{BodyWrapper, BytesResponse, HttpClient};
use opsbox_priv::{PrivConfig, PrivServer};
use opsbox_proxy::{
    AcmeDirectory, CertState, ProxyClient, ProxyConfig, ProxyServer, Route, UpstreamTls, sha256_hex,
};
use rcgen::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;
use serde_json::{Value, json};
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, oneshot};
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::{PrivateKeyDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore, ServerConfig};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_tungstenite::tungstenite::Message;

struct MockAcmeState {
    ca_cert_pem: String,
    ca_issuer: Issuer<'static, KeyPair>,
    proxy_http_addr: RwLock<Option<SocketAddr>>,
    validity_days: Mutex<i64>,
    orders: Mutex<HashMap<String, MockOrder>>,
    nonces: Mutex<u64>,
    chal_checks: Mutex<u32>,
}

#[derive(Clone)]
struct MockOrder {
    domain: String,
    token: String,
    ready: bool,
    cert_pem: Option<String>,
}

impl MockAcmeState {
    fn new(validity_days: i64) -> Arc<Self> {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        let mut dn = DistinguishedName::new();
        dn.push(rcgen::DnType::CommonName, "Opsbox Test ACME CA");
        ca_params.distinguished_name = dn;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let ca_cert_pem = ca_cert.pem();
        let ca_issuer = Issuer::new(ca_params, ca_key);
        Arc::new(Self {
            ca_cert_pem,
            ca_issuer,
            proxy_http_addr: RwLock::new(None),
            validity_days: Mutex::new(validity_days),
            orders: Mutex::new(HashMap::new()),
            nonces: Mutex::new(0),
            chal_checks: Mutex::new(0),
        })
    }

    fn next_nonce(&self) -> String {
        let mut g = self.nonces.lock().unwrap();
        *g += 1;
        format!("nonce-{}", *g)
    }
}

struct MockAcmeClient {
    state: Arc<MockAcmeState>,
}

impl HttpClient for MockAcmeClient {
    fn request(
        &self,
        req: Request<BodyWrapper<Bytes>>,
    ) -> Pin<Box<dyn Future<Output = Result<BytesResponse, instant_acme::Error>> + Send>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let uri = req.uri().to_string();
            let method = req.method().clone();
            let body_wrapper = req.into_body();
            let raw_body = body_wrapper
                .collect()
                .await
                .unwrap_or_else(|never| match never {})
                .to_bytes();

            let jws_payload: Option<Value> = if raw_body.is_empty() {
                None
            } else {
                let jws: Value = serde_json::from_slice(&raw_body)?;
                let payload_b64 = jws["payload"].as_str().unwrap_or("");
                if payload_b64.is_empty() {
                    None
                } else {
                    let decoded = B64
                        .decode(payload_b64)
                        .map_err(|e| instant_acme::Error::Other(Box::new(e)))?;
                    Some(serde_json::from_slice(&decoded)?)
                }
            };

            let nonce = state.next_nonce();
            let make_json = |status: StatusCode, val: Value, loc: Option<&str>| {
                let mut builder = Response::builder()
                    .status(status)
                    .header("replay-nonce", &nonce)
                    .header(CONTENT_TYPE, "application/json");
                if let Some(l) = loc {
                    builder = builder.header(LOCATION, l);
                }
                let body = Full::new(Bytes::from(val.to_string()));
                Ok(BytesResponse::from(builder.body(body).unwrap()))
            };

            if uri == "https://acme.test/dir" && method == hyper::Method::GET {
                return make_json(
                    StatusCode::OK,
                    json!({
                        "newNonce": "https://acme.test/new-nonce",
                        "newAccount": "https://acme.test/new-acct",
                        "newOrder": "https://acme.test/new-order"
                    }),
                    None,
                );
            }
            if uri == "https://acme.test/new-nonce" {
                let resp = Response::builder()
                    .status(StatusCode::OK)
                    .header("replay-nonce", &nonce)
                    .body(Full::new(Bytes::new()))
                    .unwrap();
                return Ok(BytesResponse::from(resp));
            }
            if uri == "https://acme.test/new-acct" {
                return make_json(
                    StatusCode::CREATED,
                    json!({"status": "valid"}),
                    Some("https://acme.test/acct/1"),
                );
            }
            if uri == "https://acme.test/new-order" {
                let payload = jws_payload.unwrap();
                let domain = payload["identifiers"][0]["value"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let oid = format!("ord-{}", state.orders.lock().unwrap().len() + 1);
                let token = format!("tok-{oid}");
                state.orders.lock().unwrap().insert(
                    oid.clone(),
                    MockOrder {
                        domain: domain.clone(),
                        token,
                        ready: false,
                        cert_pem: None,
                    },
                );
                return make_json(
                    StatusCode::CREATED,
                    json!({
                        "status": "pending",
                        "identifiers": [{"type": "dns", "value": domain}],
                        "authorizations": [format!("https://acme.test/authz/{oid}")],
                        "finalize": format!("https://acme.test/finalize/{oid}")
                    }),
                    Some(&format!("https://acme.test/order/{oid}")),
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/authz/") {
                let ord = state.orders.lock().unwrap().get(oid).unwrap().clone();
                let status = if ord.ready { "valid" } else { "pending" };
                return make_json(
                    StatusCode::OK,
                    json!({
                        "identifier": {"type": "dns", "value": ord.domain},
                        "status": status,
                        "challenges": [{
                            "type": "http-01",
                            "url": format!("https://acme.test/chal/{oid}"),
                            "status": status,
                            "token": ord.token
                        }]
                    }),
                    None,
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/chal/") {
                let (domain, token) = {
                    let g = state.orders.lock().unwrap();
                    let o = g.get(oid).unwrap();
                    (o.domain.clone(), o.token.clone())
                };
                // Perform a REAL HTTP-01 GET against ProxyServer's HTTP listener!
                let http_addr = state.proxy_http_addr.read().await.unwrap();
                let mut stream = TcpStream::connect(http_addr).await.unwrap();
                let req_str = format!(
                    "GET /.well-known/acme-challenge/{token} HTTP/1.1\r\nHost: {domain}\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(req_str.as_bytes()).await.unwrap();
                let mut resp_bytes = Vec::new();
                stream.read_to_end(&mut resp_bytes).await.unwrap();
                let resp_text = String::from_utf8_lossy(&resp_bytes);
                assert!(resp_text.starts_with("HTTP/1.1 200 OK"), "{resp_text}");
                let body = resp_text.split("\r\n\r\n").nth(1).unwrap();
                assert!(body.starts_with(&format!("{token}.")), "{body}");
                *state.chal_checks.lock().unwrap() += 1;

                state.orders.lock().unwrap().get_mut(oid).unwrap().ready = true;
                return make_json(
                    StatusCode::OK,
                    json!({
                        "type": "http-01",
                        "url": format!("https://acme.test/chal/{oid}"),
                        "status": "valid",
                        "token": token
                    }),
                    None,
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/order/") {
                let ord = state.orders.lock().unwrap().get(oid).unwrap().clone();
                let status = if ord.cert_pem.is_some() {
                    "valid"
                } else if ord.ready {
                    "ready"
                } else {
                    "pending"
                };
                return make_json(
                    StatusCode::OK,
                    json!({
                        "status": status,
                        "identifiers": [{"type": "dns", "value": ord.domain}],
                        "authorizations": [format!("https://acme.test/authz/{oid}")],
                        "finalize": format!("https://acme.test/finalize/{oid}"),
                        "certificate": ord.cert_pem.as_ref().map(|_| format!("https://acme.test/cert/{oid}"))
                    }),
                    None,
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/finalize/") {
                let payload = jws_payload.unwrap();
                let csr_b64 = payload["csr"].as_str().unwrap();
                let csr_der = B64.decode(csr_b64).unwrap();
                let mut csr_params =
                    CertificateSigningRequestParams::from_der(&csr_der.into()).unwrap();
                let days = *state.validity_days.lock().unwrap();
                let now = OffsetDateTime::now_utc();
                csr_params.params.not_before = now - TimeDuration::hours(1);
                csr_params.params.not_after = now + TimeDuration::days(days);
                let leaf = csr_params.signed_by(&state.ca_issuer).unwrap();
                let chain_pem = format!("{}{}", leaf.pem(), state.ca_cert_pem);

                let mut g = state.orders.lock().unwrap();
                let ord = g.get_mut(oid).unwrap();
                ord.cert_pem = Some(chain_pem);
                return make_json(
                    StatusCode::OK,
                    json!({
                        "status": "valid",
                        "identifiers": [{"type": "dns", "value": ord.domain}],
                        "authorizations": [format!("https://acme.test/authz/{oid}")],
                        "finalize": format!("https://acme.test/finalize/{oid}"),
                        "certificate": format!("https://acme.test/cert/{oid}")
                    }),
                    None,
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/cert/") {
                let pem = state
                    .orders
                    .lock()
                    .unwrap()
                    .get(oid)
                    .unwrap()
                    .cert_pem
                    .clone()
                    .unwrap();
                let resp = Response::builder()
                    .status(StatusCode::OK)
                    .header("replay-nonce", &nonce)
                    .header(CONTENT_TYPE, "application/pem-certificate-chain")
                    .body(Full::new(Bytes::from(pem)))
                    .unwrap();
                return Ok(BytesResponse::from(resp));
            }
            panic!("unexpected ACME call: {method} {uri}");
        })
    }
}

/// Start a TLS upstream simulating `opsbox serve`: echoes headers on HTTP and echoes
/// text frames on WebSocket `/ws`. Returns `(addr, leaf_cert_sha256)`.
async fn spawn_tls_upstream() -> (SocketAddr, String) {
    let mut params =
        CertificateParams::new(vec!["127.0.0.1".to_owned(), "localhost".to_owned()]).unwrap();
    params.distinguished_name = DistinguishedName::new();
    let key = KeyPair::generate().unwrap();
    let cert = params.self_signed(&key).unwrap();
    let cert_sha256 = sha256_hex(cert.der());

    let certs = vec![cert.der().clone()];
    let key_der = PrivateKeyDer::from_pem_slice(key.serialize_pem().as_bytes()).unwrap();
    let server_cfg = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(certs, key_der)
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(stream).await else {
                    return;
                };
                let io = TokioIo::new(tls);
                let svc = hyper::service::service_fn(|mut req: Request<Incoming>| async move {
                    if req.uri().path() == "/ws"
                        && let Some(key_hdr) = req.headers().get("sec-websocket-key").cloned()
                    {
                        let accept = tokio_tungstenite::tungstenite::handshake::derive_accept_key(
                            key_hdr.as_bytes(),
                        );
                        let upgrade = hyper::upgrade::on(&mut req);
                        tokio::spawn(async move {
                            if let Ok(upgraded) = upgrade.await {
                                let ws = tokio_tungstenite::WebSocketStream::from_raw_socket(
                                    TokioIo::new(upgraded),
                                    tokio_tungstenite::tungstenite::protocol::Role::Server,
                                    None,
                                )
                                .await;
                                let (mut tx, mut rx) = ws.split();
                                while let Some(Ok(msg)) = rx.next().await {
                                    if msg.is_text() || msg.is_binary() {
                                        let _ = tx.send(msg).await;
                                    } else if msg.is_close() {
                                        break;
                                    }
                                }
                            }
                        });
                        return Ok::<_, Infallible>(
                            Response::builder()
                                .status(StatusCode::SWITCHING_PROTOCOLS)
                                .header("connection", "Upgrade")
                                .header("upgrade", "websocket")
                                .header("sec-websocket-accept", accept)
                                .body(Full::new(Bytes::new()))
                                .unwrap(),
                        );
                    }
                    let info = json!({
                        "host": req.headers().get("host").and_then(|v| v.to_str().ok()),
                        "xff": req.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok()),
                        "xfp": req.headers().get("x-forwarded-proto").and_then(|v| v.to_str().ok()),
                        "xfh": req.headers().get("x-forwarded-host").and_then(|v| v.to_str().ok()),
                        "path": req.uri().path(),
                    });
                    Ok::<_, Infallible>(
                        Response::builder()
                            .status(StatusCode::OK)
                            .header(CONTENT_TYPE, "application/json")
                            .body(Full::new(Bytes::from(info.to_string())))
                            .unwrap(),
                    )
                });
                let _ = ConnBuilder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(io, svc)
                    .await;
            });
        }
    });

    (addr, cert_sha256)
}

#[tokio::test]
async fn acme_http01_encryption_https_websocket_and_30d_renewal_e2e() {
    let _ = ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    std::fs::write(&probe, b"").unwrap();
    let uid = std::fs::metadata(&probe).unwrap().uid();

    // 1. Start real `PrivServer`.
    let priv_sock = dir.path().join("priv.sock");
    let mut priv_cfg = PrivConfig::new(&priv_sock, vec![uid]);
    priv_cfg.master_key_path = dir.path().join("master.key");
    let priv_server = PrivServer::bind(priv_cfg).unwrap();
    let (priv_stop, priv_stopped) = oneshot::channel::<()>();
    let priv_task = tokio::spawn(priv_server.run(async {
        let _ = priv_stopped.await;
    }));

    // 2. Start TLS upstream (`opsbox serve` analogue) and mock ACME server (20-day cert!).
    let (upstream_addr, upstream_sha256) = spawn_tls_upstream().await;
    let acme_state = MockAcmeState::new(20);
    let acme_for_factory = Arc::clone(&acme_state);

    let state_dir = dir.path().join("proxy-state");
    let proxy_sock = dir.path().join("proxy.sock");
    let mut cfg = ProxyConfig::new(&priv_sock);
    cfg.listen_http = "127.0.0.1:0".parse().unwrap();
    cfg.listen_https = "127.0.0.1:0".parse().unwrap();
    cfg.socket_path = proxy_sock.clone();
    cfg.state_dir = state_dir.clone();
    cfg.acme_directory = AcmeDirectory::Custom("https://acme.test/dir".into());
    cfg.custom_http = Some(Arc::new(move || {
        Box::new(MockAcmeClient {
            state: Arc::clone(&acme_for_factory),
        })
    }));

    let server = ProxyServer::bind(cfg.clone()).await.unwrap();
    let http_addr = server.http_addr();
    let https_addr = server.https_addr();
    *acme_state.proxy_http_addr.write().await = Some(http_addr);
    let acme_mgr = server.acme();

    let (proxy_stop, proxy_stopped) = oneshot::channel::<()>();
    let proxy_task = tokio::spawn(async move {
        server
            .run(async {
                let _ = proxy_stopped.await;
            })
            .await
            .unwrap();
    });

    let client = ProxyClient::new(&proxy_sock).with_timeout(Duration::from_secs(30));
    assert!(!client.ping("p1").await.unwrap().is_empty());

    // 3. Push route for panel.example.com -> TLS upstream (pinned SHA-256).
    let route = Route {
        host: "panel.example.com".into(),
        upstream: upstream_addr.to_string(),
        upstream_tls: UpstreamTls::Https {
            cert_sha256: Some(upstream_sha256),
        },
        redirect_www: false,
        redirect_https: true,
        hsts: true,
        max_body_bytes: 1024 * 1024,
        is_panel: true,
    };
    client.set_routes("r1", vec![route]).await.unwrap();

    // 4. Issue ACME HTTP-01 cert (verifies HTTP-01 challenge on ProxyServer's HTTP port!).
    let cert_info = client.issue_cert("i1", "panel.example.com").await.unwrap();
    assert_eq!(cert_info.domain, "panel.example.com");
    assert_eq!(cert_info.status, CertState::Expiring); // 20 days <= 30d window
    assert_eq!(*acme_state.chal_checks.lock().unwrap(), 1);

    // 5. Verify keys on disk are encrypted at rest (never plaintext PEM/JWK!).
    let acct_raw = std::fs::read_to_string(state_dir.join("acme-account-custom.json")).unwrap();
    assert!(acct_raw.contains("enc:v1:"), "{acct_raw}");
    assert!(!acct_raw.contains("key_pkcs8"));
    let cert_raw = std::fs::read_to_string(state_dir.join("certs/panel.example.com.json")).unwrap();
    assert!(cert_raw.contains("enc:v1:"), "{cert_raw}");
    assert!(!cert_raw.contains("BEGIN PRIVATE KEY"));

    // 6. Verify HTTP -> HTTPS 308 redirect on :80 listener.
    let mut http_conn = TcpStream::connect(http_addr).await.unwrap();
    http_conn
        .write_all(
            b"GET /hello?a=1 HTTP/1.1\r\nHost: panel.example.com\r\nConnection: close\r\n\r\n",
        )
        .await
        .unwrap();
    let mut http_resp = Vec::new();
    http_conn.read_to_end(&mut http_resp).await.unwrap();
    let http_text = String::from_utf8_lossy(&http_resp);
    assert!(http_text.starts_with("HTTP/1.1 308"), "{http_text}");
    assert!(
        http_text.contains("location: https://panel.example.com/hello?a=1"),
        "{http_text}"
    );

    // 7. Connect over HTTPS (:443 listener) trusting the test CA; verify proxying, HSTS, headers.
    let mut roots = RootCertStore::empty();
    for der in CertificateDer::pem_slice_iter(acme_state.ca_cert_pem.as_bytes()) {
        roots.add(der.unwrap()).unwrap();
    }
    let tls_client_cfg = Arc::new(
        ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let connector = TlsConnector::from(tls_client_cfg.clone());
    let tcp = TcpStream::connect(https_addr).await.unwrap();
    let tls = connector
        .connect(ServerName::try_from("panel.example.com").unwrap(), tcp)
        .await
        .unwrap();
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .unwrap();
    tokio::spawn(conn);
    let resp = sender
        .send_request(
            Request::get("/api/v1/host")
                .header("host", "panel.example.com")
                .header("x-forwarded-for", "198.51.100.1")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("strict-transport-security")
            .unwrap()
            .to_str()
            .unwrap(),
        "max-age=31536000"
    );
    let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let echoed: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(echoed["host"], "panel.example.com");
    assert_eq!(echoed["xfp"], "https");
    assert_eq!(echoed["xfh"], "panel.example.com");
    assert_eq!(echoed["xff"], "198.51.100.1, 127.0.0.1");

    // 8. Verify WebSocket upgrade (wss://) through proxy to TLS upstream.
    let ws_tcp = TcpStream::connect(https_addr).await.unwrap();
    let ws_tls = connector
        .connect(ServerName::try_from("panel.example.com").unwrap(), ws_tcp)
        .await
        .unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async("wss://panel.example.com/ws", ws_tls)
        .await
        .unwrap();
    ws.send(Message::Text("ping-ws".into())).await.unwrap();
    let echoed_ws = ws.next().await.unwrap().unwrap();
    assert_eq!(echoed_ws.into_text().unwrap(), "ping-ws");

    // 9. Verify 30-day renewal: bump CA validity to 90 days and trigger renew_due_certs().
    *acme_state.validity_days.lock().unwrap() = 90;
    acme_mgr.renew_due_certs().await;
    assert_eq!(*acme_state.chal_checks.lock().unwrap(), 2);
    let status = client.status("s1").await.unwrap();
    assert_eq!(status.routes_count, 1);
    assert_eq!(status.certs.len(), 1);
    assert_eq!(status.certs[0].status, CertState::Valid);
    assert!(status.certs[0].days_remaining.unwrap() >= 85);

    // 10. Verify persisted encrypted cert & routes survive a ProxyServer restart.
    let _ = proxy_stop.send(());
    proxy_task.await.unwrap();

    let server2 = ProxyServer::bind(cfg).await.unwrap();
    let https2 = server2.https_addr();
    let (proxy_stop2, proxy_stopped2) = oneshot::channel::<()>();
    let proxy_task2 = tokio::spawn(async move {
        server2
            .run(async {
                let _ = proxy_stopped2.await;
            })
            .await
            .unwrap();
    });
    let tcp2 = TcpStream::connect(https2).await.unwrap();
    let tls2 = connector
        .connect(ServerName::try_from("panel.example.com").unwrap(), tcp2)
        .await
        .unwrap();
    let (mut sender2, conn2) = hyper::client::conn::http1::handshake(TokioIo::new(tls2))
        .await
        .unwrap();
    tokio::spawn(conn2);
    let resp2 = sender2
        .send_request(
            Request::get("/healthz")
                .header("host", "panel.example.com")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);

    // 11. Oversized request body returns 413 Payload Too Large; unknown host returns 404.
    let big_tcp = TcpStream::connect(https2).await.unwrap();
    let big_tls = connector
        .connect(ServerName::try_from("panel.example.com").unwrap(), big_tcp)
        .await
        .unwrap();
    let (mut big_sender, big_conn) = hyper::client::conn::http1::handshake(TokioIo::new(big_tls))
        .await
        .unwrap();
    tokio::spawn(big_conn);
    let big_resp = big_sender
        .send_request(
            Request::post("/api/v1/host")
                .header("host", "panel.example.com")
                .header("content-length", "99999999")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(big_resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let miss_tcp = TcpStream::connect(https2).await.unwrap();
    let miss_tls = connector
        .connect(ServerName::try_from("panel.example.com").unwrap(), miss_tcp)
        .await
        .unwrap();
    let (mut miss_sender, miss_conn) =
        hyper::client::conn::http1::handshake(TokioIo::new(miss_tls))
            .await
            .unwrap();
    tokio::spawn(miss_conn);
    let miss_resp = miss_sender
        .send_request(
            Request::get("/")
                .header("host", "other.example.com")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(miss_resp.status(), StatusCode::NOT_FOUND);

    // 12. Staging directory URL check.
    assert!(AcmeDirectory::Staging.is_staging());
    assert!(AcmeDirectory::Staging.url().contains("staging"));
    assert!(!AcmeDirectory::Production.is_staging());

    let _ = proxy_stop2.send(());
    proxy_task2.await.unwrap();
    let _ = priv_stop.send(());
    priv_task.await.unwrap();
}
