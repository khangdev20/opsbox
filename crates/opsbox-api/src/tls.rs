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
use tokio_rustls::{TlsAcceptor, rustls};
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
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(io::Error::other)?;
    Ok(TlsAcceptor::from(Arc::new(cfg)))
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
