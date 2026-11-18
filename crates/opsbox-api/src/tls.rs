//! Self-signed TLS certificate generation, cloud metadata public-IP detection, and TLS server.

use std::fs;
use std::future::Future;
use std::io;
use std::net::{IpAddr, UdpSocket};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ConnectInfo;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use hyper_util::service::TowerToHyperService;
use rcgen::generate_simple_self_signed;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::TcpListener;
pub use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls;
use tower::ServiceExt;

use crate::{AppState, router};

pub const DEFAULT_CERT_PATH: &str = "/etc/opsbox/panel-selfsigned.crt";
pub const DEFAULT_KEY_PATH: &str = "/etc/opsbox/panel-selfsigned.key";

fn metadata_agent() -> ureq::Agent {
    let cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_millis(500)))
        .build();
    cfg.into()
}

fn parse_ip_body(resp: ureq::http::Response<ureq::Body>) -> Option<IpAddr> {
    let text = resp.into_body().read_to_string().ok()?;
    text.trim().parse::<IpAddr>().ok()
}

/// Detect the server's public IP from cloud metadata (AWS IMDSv2, GCP, Azure, DigitalOcean,
/// Hetzner) with a short timeout, falling back to the primary outbound interface address.
pub fn detect_public_ip() -> Option<IpAddr> {
    if std::env::var_os("OPSBOX_NO_IMDS").is_none() {
        let agent = metadata_agent();

        // 1. AWS IMDSv2
        if let Ok(tok_resp) = agent
            .put("http://169.254.169.254/latest/api/token")
            .header("X-aws-ec2-metadata-token-ttl-seconds", "60")
            .send_empty()
            && let Ok(token) = tok_resp.into_body().read_to_string()
            && let Ok(ip_resp) = agent
                .get("http://169.254.169.254/latest/meta-data/public-ipv4")
                .header("X-aws-ec2-metadata-token", token.trim())
                .call()
            && let Some(ip) = parse_ip_body(ip_resp)
        {
            return Some(ip);
        }

        // 2. GCP
        if let Ok(resp) = agent
            .get("http://169.254.169.254/computeMetadata/v1/instance/network-interfaces/0/access-configs/0/external-ip")
            .header("Metadata-Flavor", "Google")
            .call()
            && let Some(ip) = parse_ip_body(resp)
        {
            return Some(ip);
        }

        // 3. Azure
        if let Ok(resp) = agent
            .get("http://169.254.169.254/metadata/instance/network/interface/0/ipv4/ipAddress/0/publicIpAddress?api-version=2021-02-01&format=text")
            .header("Metadata", "true")
            .call()
            && let Some(ip) = parse_ip_body(resp)
        {
            return Some(ip);
        }

        // 4. DigitalOcean
        if let Ok(resp) = agent
            .get("http://169.254.169.254/metadata/v1/interfaces/public/0/ipv4/address")
            .call()
            && let Some(ip) = parse_ip_body(resp)
        {
            return Some(ip);
        }

        // 5. Hetzner
        if let Ok(resp) = agent
            .get("http://169.254.169.254/hetzner/v1/metadata/public-ipv4")
            .call()
            && let Some(ip) = parse_ip_body(resp)
        {
            return Some(ip);
        }
    }

    // Fallback: primary outbound interface IP (connect on UDP does not send packets).
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0")
        && sock.connect("1.1.1.1:80").is_ok()
        && let Ok(addr) = sock.local_addr()
        && !addr.ip().is_loopback()
        && !addr.ip().is_unspecified()
    {
        return Some(addr.ip());
    }
    None
}

/// Generate a self-signed certificate and private key in PEM format for `extra_ips` and `hostname`.
pub fn generate_self_signed(
    extra_ips: &[IpAddr],
    hostname: Option<&str>,
) -> io::Result<(String, String)> {
    let mut sans = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    if let Some(h) = hostname
        && !h.is_empty()
    {
        sans.push(h.to_owned());
    }
    for ip in extra_ips {
        let s = ip.to_string();
        if !sans.contains(&s) {
            sans.push(s);
        }
    }
    let certified = generate_simple_self_signed(sans).map_err(io::Error::other)?;
    Ok((certified.cert.pem(), certified.key_pair.serialize_pem()))
}

/// Ensure a self-signed certificate and key exist at `cert_path` and `key_path`.
pub fn ensure_self_signed_cert(
    cert_path: &Path,
    key_path: &Path,
) -> io::Result<(PathBuf, PathBuf)> {
    if cert_path.exists() && key_path.exists() {
        return Ok((cert_path.to_path_buf(), key_path.to_path_buf()));
    }
    let hostname = fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let mut ips = Vec::new();
    if let Some(ip) = detect_public_ip() {
        ips.push(ip);
    }
    let (cert_pem, key_pem) = generate_self_signed(&ips, hostname.as_deref())?;
    if let Some(p) = cert_path.parent() {
        fs::create_dir_all(p)?;
    }
    if let Some(p) = key_path.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(cert_path, cert_pem)?;
    fs::set_permissions(cert_path, fs::Permissions::from_mode(0o644))?;
    fs::write(key_path, key_pem)?;
    fs::set_permissions(key_path, fs::Permissions::from_mode(0o640))?;
    Ok((cert_path.to_path_buf(), key_path.to_path_buf()))
}

/// Compute the lowercase hex SHA-256 fingerprint of the leaf certificate in `cert_path`.
pub fn cert_sha256_hex(cert_path: &Path) -> Option<String> {
    let leaf = CertificateDer::pem_file_iter(cert_path)
        .ok()?
        .next()?
        .ok()?;
    Some(opsbox_proxy::sha256_hex(leaf.as_ref()))
}

/// The TLS settings for the panel: TLS 1.3 preferred, TLS 1.2 the oldest allowed, the
/// provider's default cipher suites (no legacy ones), and ALPN for HTTP/2 and HTTP/1.1.
pub fn server_config(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> io::Result<rustls::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(io::Error::other)?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(cfg)
}

/// Load a `tokio_rustls::TlsAcceptor` from PEM certificate and private key files.
pub fn load_tls_acceptor(cert_path: &Path, key_path: &Path) -> io::Result<TlsAcceptor> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(io::Error::other)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io::Error::other)?;
    if certs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no certificates found in PEM file",
        ));
    }
    let key = PrivateKeyDer::from_pem_file(key_path).map_err(io::Error::other)?;
    Ok(TlsAcceptor::from(Arc::new(server_config(certs, key)?)))
}

/// Accepts any certificate, because the only use is to read which one is presented.
#[derive(Debug)]
struct ReadOnlyVerifier(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for ReadOnlyVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// What a TLS listener presents: the SHA-256 fingerprint of its leaf certificate and the
/// protocol version negotiated. Nothing is sent after the handshake. Used by `opsbox
/// status` so the fingerprint shown is the one the server really serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsProbe {
    pub cert_sha256: String,
    pub version: String,
}

/// Connect to `addr`, finish a TLS handshake without judging the certificate, and report
/// what was presented. Fails when nothing on `addr` speaks TLS.
pub async fn probe_tls(addr: std::net::SocketAddr, timeout: Duration) -> io::Result<TlsProbe> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(ReadOnlyVerifier(provider)))
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(cfg));
    let name = rustls_pki_types::ServerName::IpAddress(addr.ip().into());
    let handshake = async {
        let tcp = tokio::net::TcpStream::connect(addr).await?;
        connector.connect(name, tcp).await
    };
    let stream = tokio::time::timeout(timeout, handshake)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TLS handshake timed out"))??;
    let (_, conn) = stream.get_ref();
    let leaf = conn
        .peer_certificates()
        .and_then(<[_]>::first)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no certificate presented"))?;
    Ok(TlsProbe {
        cert_sha256: opsbox_proxy::sha256_hex(leaf.as_ref()),
        version: match conn.protocol_version() {
            Some(rustls::ProtocolVersion::TLSv1_3) => "TLS 1.3".to_owned(),
            Some(rustls::ProtocolVersion::TLSv1_2) => "TLS 1.2".to_owned(),
            Some(other) => format!("{other:?}"),
            None => "TLS".to_owned(),
        },
    })
}

/// Serve `router(state)` over TLS on `listener` until `shutdown` completes.
pub async fn serve_tls(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    let auth = state.auth.clone();
    let janitor = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(3600));
        loop {
            tick.tick().await;
            if let Err(e) = auth.purge_expired() {
                tracing::warn!(error = %e, "could not purge expired sessions");
            }
        }
    });

    let app = router(state);
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(error = %e, "TLS accept error");
                        continue;
                    }
                };
                let acceptor = acceptor.clone();
                let svc_app = app.clone();
                tokio::spawn(async move {
                    let Ok(tls_stream) = acceptor.accept(stream).await else {
                        return;
                    };
                    let io = TokioIo::new(tls_stream);
                    let svc = tower::service_fn(move |req: axum::http::Request<hyper::body::Incoming>| {
                        let mut req = req.map(axum::body::Body::new);
                        req.extensions_mut().insert(ConnectInfo(peer));
                        svc_app.clone().oneshot(req)
                    });
                    let hyper_svc = TowerToHyperService::new(svc);
                    let _ = ConnBuilder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(io, hyper_svc)
                        .await;
                });
            }
        }
    }

    janitor.abort();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cert() -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let (cert_pem, key_pem) = generate_self_signed(&[], None).unwrap();
        let certs = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).unwrap();
        (certs, key)
    }

    /// A one-connection TLS echo-less server on loopback; returns its address.
    async fn serve_once(cfg: rustls::ServerConfig) -> std::net::SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(cfg));
        tokio::spawn(async move {
            for _ in 0..4 {
                if let Ok((tcp, _)) = listener.accept().await {
                    let _ = acceptor.accept(tcp).await;
                }
            }
        });
        addr
    }

    /// Handshake with a client limited to `versions`, trusting the generated certificate.
    async fn handshake(
        addr: std::net::SocketAddr,
        certs: &[CertificateDer<'static>],
        versions: &[&'static rustls::SupportedProtocolVersion],
        alpn: &[&[u8]],
    ) -> io::Result<(Option<rustls::ProtocolVersion>, Option<Vec<u8>>)> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certs[0].clone()).unwrap();
        let mut cfg = rustls::ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .map_err(io::Error::other)?
            .with_root_certificates(roots)
            .with_no_client_auth();
        cfg.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
        let tcp = tokio::net::TcpStream::connect(addr).await?;
        let name = rustls_pki_types::ServerName::try_from("localhost").unwrap();
        let tls = tokio_rustls::TlsConnector::from(Arc::new(cfg))
            .connect(name, tcp)
            .await?;
        let (_, c) = tls.get_ref();
        Ok((c.protocol_version(), c.alpn_protocol().map(<[u8]>::to_vec)))
    }

    #[tokio::test]
    async fn tls_13_is_preferred_and_tls_12_is_the_floor() {
        let (certs, key) = test_cert();
        let cfg = server_config(certs.clone(), key).unwrap();
        // rustls implements nothing older than TLS 1.2, and `server_config` offers no more.
        let addr = serve_once(cfg).await;
        let (v, _) = handshake(
            addr,
            &certs,
            &[&rustls::version::TLS13, &rustls::version::TLS12],
            &[],
        )
        .await
        .unwrap();
        assert_eq!(v, Some(rustls::ProtocolVersion::TLSv1_3));
        let (v, _) = handshake(addr, &certs, &[&rustls::version::TLS12], &[])
            .await
            .unwrap();
        assert_eq!(v, Some(rustls::ProtocolVersion::TLSv1_2));
    }

    #[tokio::test]
    async fn alpn_offers_h2_then_http_1_1() {
        let (certs, key) = test_cert();
        let addr = serve_once(server_config(certs.clone(), key).unwrap()).await;
        let both = [&rustls::version::TLS13, &rustls::version::TLS12];
        let (_, alpn) = handshake(addr, &certs, &both, &[b"h2", b"http/1.1"])
            .await
            .unwrap();
        assert_eq!(alpn.as_deref(), Some(&b"h2"[..]));
        let (_, alpn) = handshake(addr, &certs, &both, &[b"http/1.1"])
            .await
            .unwrap();
        assert_eq!(alpn.as_deref(), Some(&b"http/1.1"[..]));
    }

    #[tokio::test]
    async fn the_probe_reports_the_certificate_the_server_presents() {
        let (certs, key) = test_cert();
        let want = opsbox_proxy::sha256_hex(certs[0].as_ref());
        let addr = serve_once(server_config(certs, key).unwrap()).await;
        let probe = probe_tls(addr, Duration::from_secs(5)).await.unwrap();
        assert_eq!(probe.cert_sha256, want);
        assert_eq!(probe.version, "TLS 1.3");
    }

    #[tokio::test]
    async fn the_probe_fails_on_a_plain_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            if let Ok((mut s, _)) = listener.accept().await {
                let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
            }
        });
        assert!(probe_tls(addr, Duration::from_secs(5)).await.is_err());
    }
}
