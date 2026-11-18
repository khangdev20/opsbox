//! Independent health probes for `serve`, `priv`, `proxy`, disk space, and SQLite.

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use opsbox_proto::frame;
use opsbox_proto::{PrivOp, PrivReply, PrivRequest, PrivResponse, PrivResult};
use opsbox_store::ReadOnlyStore;
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpStream, UnixStream};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::{ring, verify_tls12_signature, verify_tls13_signature};
use tokio_rustls::rustls::{
    ClientConfig, DigitallySignedStruct, Error as RustlsError, SignatureScheme,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_HTTP_BYTES: u64 = 64 * 1024;
const CRITICAL_AVAIL_BYTES: u64 = 128 * 1024 * 1024;
const WARN_AVAIL_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceHealth {
    Ok { version: String, millis: u128 },
    PortConflict { version: String, millis: u128 },
    Down(String),
    Off,
}

impl ServiceHealth {
    pub fn is_ok(&self) -> bool {
        matches!(
            self,
            Self::Ok { .. } | Self::PortConflict { .. } | Self::Off
        )
    }

    pub fn is_failing(&self) -> bool {
        matches!(self, Self::Down(_))
    }
}

#[derive(Debug, Clone)]
pub struct DiskHealth {
    pub path: String,
    pub total_bytes: u64,
    pub avail_bytes: u64,
    pub used_pct: f64,
    pub warn: bool,
    pub critical: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbHealth {
    Ok,
    Missing,
    Failed(String),
}

#[derive(Debug)]
struct LoopbackSelfSignedVerifier;

impl ServerCertVerifier for LoopbackSelfSignedVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
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
        cert: &CertificateDer<'_>,
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

pub fn loopback_target(addr: SocketAddr) -> SocketAddr {
    if addr.ip().is_unspecified() {
        let ip = if addr.is_ipv6() {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        SocketAddr::new(ip, addr.port())
    } else {
        addr
    }
}

pub async fn probe_serve(listen: SocketAddr) -> ServiceHealth {
    let target = loopback_target(listen);
    let started = Instant::now();
    match probe_serve_tls(target).await {
        Ok((200, Some(version))) => ServiceHealth::Ok {
            version,
            millis: started.elapsed().as_millis(),
        },
        Ok((status, _)) => ServiceHealth::Down(format!("answered /healthz with HTTP {status}")),
        Err(tls_err) => match probe_serve_plain(target).await {
            Ok((200, Some(version))) => ServiceHealth::Ok {
                version,
                millis: started.elapsed().as_millis(),
            },
            Ok((status, _)) => ServiceHealth::Down(format!("answered /healthz with HTTP {status}")),
            Err(_) => ServiceHealth::Down(describe_io_err(&tls_err)),
        },
    }
}

async fn probe_serve_tls(addr: SocketAddr) -> io::Result<(u16, Option<String>)> {
    let provider = Arc::new(ring::default_provider());
    let cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(LoopbackSelfSignedVerifier))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(cfg));
    let server_name = ServerName::try_from("localhost").map_err(io::Error::other)?;

    tokio::time::timeout(PROBE_TIMEOUT, async {
        let tcp = TcpStream::connect(addr).await?;
        let mut tls = connector.connect(server_name, tcp).await?;
        http_healthz_exchange(&mut tls, addr).await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timed out"))?
}

async fn probe_serve_plain(addr: SocketAddr) -> io::Result<(u16, Option<String>)> {
    tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut tcp = TcpStream::connect(addr).await?;
        http_healthz_exchange(&mut tcp, addr).await
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timed out"))?
}

async fn http_healthz_exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    addr: SocketAddr,
) -> io::Result<(u16, Option<String>)> {
    let req = format!(
        "GET /healthz HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;
    let mut raw = Vec::new();
    stream.take(MAX_HTTP_BYTES).read_to_end(&mut raw).await?;
    parse_healthz_http(&raw)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid /healthz HTTP response"))
}

pub fn parse_healthz_http(raw: &[u8]) -> Option<(u16, Option<String>)> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let status_line = head.lines().next()?;
    let mut parts = status_line.split_whitespace();
    if !parts.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let status: u16 = parts.next()?.parse().ok()?;
    let body = &raw[split + 4..];
    let version = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .filter(|v| v["status"].as_str() == Some("ok"))
        .and_then(|v| v["version"].as_str().map(ToOwned::to_owned));
    Some((status, version))
}

pub async fn probe_priv(socket: &Path) -> ServiceHealth {
    let started = Instant::now();
    let res = tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut stream = UnixStream::connect(socket)
            .await
            .map_err(|e| describe_io_err(&e))?;
        let req = PrivRequest {
            id: "watchdog".into(),
            actor: None,
            op: PrivOp::Ping,
        };
        let bytes = frame::encode(&req).map_err(|e| e.to_string())?;
        stream.write_all(&bytes).await.map_err(|e| e.to_string())?;
        stream.flush().await.map_err(|e| e.to_string())?;
        let body = read_unix_body(&mut stream)
            .await
            .map_err(|e| e.to_string())?;
        let resp: PrivResponse = frame::decode(&body).map_err(|e| e.to_string())?;
        match resp.result {
            PrivResult::Success {
                reply: PrivReply::Pong { version },
            } => Ok(version),
            PrivResult::Success { .. } => Err("unexpected reply from priv".to_owned()),
            PrivResult::Failure { error } => Err(error.message),
        }
    })
    .await;

    match res {
        Ok(Ok(version)) => ServiceHealth::Ok {
            version,
            millis: started.elapsed().as_millis(),
        },
        Ok(Err(why)) => ServiceHealth::Down(why),
        Err(_) => ServiceHealth::Down("did not answer in time".into()),
    }
}

pub async fn probe_proxy(socket: &Path, expected: bool) -> ServiceHealth {
    let started = Instant::now();
    let res = tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut stream = UnixStream::connect(socket).await?;
        let req = serde_json::json!({
            "id": "watchdog",
            "op": { "type": "status" }
        });
        let bytes = frame::encode(&req).map_err(io::Error::other)?;
        stream.write_all(&bytes).await?;
        stream.flush().await?;
        let body = read_unix_body(&mut stream).await?;
        let resp: serde_json::Value = frame::decode(&body).map_err(io::Error::other)?;
        if resp["result"]["status"].as_str() == Some("ok")
            && resp["result"]["reply"]["type"].as_str() == Some("status")
        {
            let ver = resp["result"]["reply"]["data"]["version"]
                .as_str()
                .unwrap_or("?")
                .to_owned();
            Ok((
                ver,
                resp["result"]["reply"]["data"]["network"]["port_conflict"]
                    .as_bool()
                    .unwrap_or(false),
            ))
        } else {
            let msg = resp["result"]["error"]["message"]
                .as_str()
                .unwrap_or("proxy returned an error")
                .to_owned();
            Err(io::Error::other(msg))
        }
    })
    .await;

    match res {
        Ok(Ok((version, true))) => ServiceHealth::PortConflict {
            version,
            millis: started.elapsed().as_millis(),
        },
        Ok(Ok((version, false))) => ServiceHealth::Ok {
            version,
            millis: started.elapsed().as_millis(),
        },
        Ok(Err(e)) => {
            if !expected
                && matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                )
            {
                ServiceHealth::Off
            } else {
                ServiceHealth::Down(describe_io_err(&e))
            }
        }
        Err(_) => ServiceHealth::Down("did not answer in time".into()),
    }
}

async fn read_unix_body(stream: &mut UnixStream) -> io::Result<Vec<u8>> {
    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    let len = frame::body_len(header).map_err(io::Error::other)?;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

fn describe_io_err(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::NotFound => "not running (socket missing)".into(),
        io::ErrorKind::ConnectionRefused => "not running (connection refused)".into(),
        io::ErrorKind::PermissionDenied => "permission denied".into(),
        io::ErrorKind::TimedOut => "did not answer in time".into(),
        _ => e.to_string(),
    }
}

pub fn probe_disk(path: &Path) -> DiskHealth {
    let label = path.display().to_string();
    let target = if path.exists() {
        path
    } else {
        path.parent()
            .filter(|p| p.exists())
            .unwrap_or_else(|| Path::new("/"))
    };
    match nix::sys::statvfs::statvfs(target) {
        #[allow(clippy::useless_conversion)]
        Ok(s) => {
            let total = u64::from(s.blocks()).saturating_mul(u64::from(s.fragment_size()));
            let avail =
                u64::from(s.blocks_available()).saturating_mul(u64::from(s.fragment_size()));
            disk_health_from_bytes(label, total, avail)
        }
        Err(e) => DiskHealth {
            path: label,
            total_bytes: 0,
            avail_bytes: 0,
            used_pct: 100.0,
            warn: true,
            critical: false,
            error: Some(format!("cannot read filesystem stats: {e}")),
        },
    }
}

pub fn disk_health_from_bytes(path: String, total_bytes: u64, avail_bytes: u64) -> DiskHealth {
    let used_pct = if total_bytes == 0 {
        100.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        {
            100.0 * (1.0 - avail_bytes as f64 / total_bytes as f64)
        }
    };
    let critical = total_bytes == 0 || avail_bytes < CRITICAL_AVAIL_BYTES || used_pct >= 98.0;
    let warn = !critical && (avail_bytes < WARN_AVAIL_BYTES || used_pct > 85.0);
    DiskHealth {
        path,
        total_bytes,
        avail_bytes,
        used_pct,
        warn,
        critical,
        error: None,
    }
}

pub fn check_db_integrity(db_path: &Path) -> DbHealth {
    if !db_path.exists() {
        return DbHealth::Missing;
    }
    match ReadOnlyStore::open(db_path).and_then(|s| {
        s.check_integrity()
            .map_err(|e| opsbox_store::StoreError::Io(io::Error::other(e)))
    }) {
        Ok(()) => DbHealth::Ok,
        Err(e) => DbHealth::Failed(e.to_string()),
    }
}

pub fn human_bytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    if b >= (1u64 << 30) as f64 {
        format!("{:.1} GB", b / (1u64 << 30) as f64)
    } else if b >= (1u64 << 20) as f64 {
        format!("{:.1} MB", b / (1u64 << 20) as f64)
    } else if b >= 1024.0 {
        format!("{:.0} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_and_invalid_healthz_responses() {
        let ok = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"status\":\"ok\",\"version\":\"0.1.4\"}";
        assert_eq!(parse_healthz_http(ok), Some((200, Some("0.1.4".into()))));

        let err_status = b"HTTP/1.1 503 Service Unavailable\r\n\r\n{}";
        assert_eq!(parse_healthz_http(err_status), Some((503, None)));

        assert_eq!(parse_healthz_http(b"not http"), None);
    }

    #[test]
    fn disk_thresholds_mark_warn_and_critical() {
        let healthy = disk_health_from_bytes("/".into(), 20 << 30, 10 << 30);
        assert!(!healthy.warn && !healthy.critical);

        let warn_bytes = disk_health_from_bytes("/".into(), 2 << 30, 300 << 20);
        assert!(warn_bytes.warn && !warn_bytes.critical);

        let warn_pct = disk_health_from_bytes("/".into(), 20 << 30, 2 << 30);
        assert!(warn_pct.warn && !warn_pct.critical);

        let critical_bytes = disk_health_from_bytes("/".into(), 20 << 30, 64 << 20);
        assert!(critical_bytes.critical);

        let critical_pct = disk_health_from_bytes("/".into(), 100 << 30, 1 << 30);
        assert!(critical_pct.critical);
    }
}
