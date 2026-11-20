//! `opsbox status`: is `serve` answering, is `priv` answering?
//!
//! `serve` is probed on its own listen address (`/healthz`, then `/readyz`) over the
//! scheme it really serves: a TLS handshake tells HTTPS (and which certificate) from
//! plain HTTP. `priv` by sending it a `Ping` on its socket. Needs no database access
//! and no root.

use std::fmt::Write as _;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use opsbox_priv::{ClientError, PrivClient};
use opsbox_proto::PrivErrorCode;
use opsbox_proxy::{ProxyClient, ProxyClientError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::VERSION;
use crate::config::Config;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, PartialEq, Eq)]
pub enum ServeState {
    Ready {
        version: String,
    },
    /// Answers, but a dependency is failing.
    Degraded {
        version: String,
        detail: String,
    },
    Down(String),
}

/// The scheme `serve` answers on, found out by connecting, not read from configuration.
#[derive(Debug, PartialEq, Eq)]
pub enum ServeTransport {
    Https(opsbox_api::tls::TlsProbe),
    Http,
    /// Nothing answered, so the scheme is not known.
    Unknown,
}

/// `AB:CD:...`, the form browsers print for a SHA-256 certificate fingerprint.
pub fn colon_fingerprint(hex: &str) -> String {
    hex.as_bytes()
        .chunks(2)
        .map(|c| String::from_utf8_lossy(c).to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join(":")
}

#[derive(Debug, PartialEq, Eq)]
pub enum PrivState {
    Ok { version: String, millis: u128 },
    Down(String),
    Denied(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProxyState {
    Conflict(opsbox_proto::ProxyNetwork),
    Ok {
        version: String,
        routes: usize,
        certs: usize,
    },
    /// Proxy is not installed/configured on this machine (dev/no domain yet).
    NotConfigured,
    Down(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum WatchdogState {
    Ok {
        version: String,
        failed: bool,
        degraded: bool,
    },
    NotConfigured,
    Down(String),
}

#[derive(Debug)]
pub struct Report {
    pub config_file: Option<PathBuf>,
    pub serve_addr: SocketAddr,
    pub serve: ServeState,
    /// What the listener really speaks, learned by connecting to it.
    pub serve_transport: ServeTransport,
    pub priv_socket: PathBuf,
    pub privileged: PrivState,
    pub proxy_socket: PathBuf,
    pub proxy: ProxyState,
    pub watchdog_addr: SocketAddr,
    pub watchdog: WatchdogState,
    pub watchdog_allow_ips: Vec<IpAddr>,
    pub watchdog_cert_sha256: Option<String>,
}

impl Report {
    pub fn healthy(&self) -> bool {
        matches!(self.serve, ServeState::Ready { .. })
            && matches!(&self.privileged, PrivState::Ok { version, .. } if version == VERSION)
            && !matches!(self.proxy, ProxyState::Down(_))
            && !matches!(self.watchdog, WatchdogState::Down(_))
    }

    pub fn render(&self) -> String {
        let mut out = format!("opsbox {VERSION}\n");
        let config = self.config_file.as_deref().map_or_else(
            || "none (built-in defaults)".to_owned(),
            |p| p.display().to_string(),
        );
        let _ = writeln!(out, "config   {config}");

        let (tag, text) = match &self.serve {
            ServeState::Ready { version } => ("ok      ", format!("v{version}, ready")),
            ServeState::Degraded { version, detail } => {
                ("degraded", format!("v{version}, not ready: {detail}"))
            }
            ServeState::Down(why) => ("DOWN    ", why.clone()),
        };
        let url = match &self.serve_transport {
            ServeTransport::Https(_) => format!("https://{}", self.serve_addr),
            ServeTransport::Http => format!("http://{}", self.serve_addr),
            // Nothing answered: say what the configuration asks for rather than guess.
            ServeTransport::Unknown => format!("{}", self.serve_addr),
        };
        let _ = writeln!(out, "serve    {tag} {url}  {text}");
        if let ServeTransport::Https(tls) = &self.serve_transport {
            let _ = writeln!(
                out,
                "         {}, certificate SHA-256 {}",
                tls.version,
                colon_fingerprint(&tls.cert_sha256)
            );
        }

        let (tag, text) = match &self.privileged {
            PrivState::Ok { version, millis } if version == VERSION => {
                ("ok      ", format!("v{version}, answered in {millis} ms"))
            }
            PrivState::Ok { version, millis } => (
                "MISMATCH",
                format!(
                    "v{version}, answered in {millis} ms; this binary is v{VERSION}. Run `sudo systemctl restart opsbox-priv` so both are the same build"
                ),
            ),
            PrivState::Down(why) => ("DOWN    ", why.clone()),
            PrivState::Denied(why) => ("DENIED  ", why.clone()),
        };
        let _ = writeln!(out, "priv     {tag} {}  {text}", self.priv_socket.display());

        let (tag, text) = match &self.proxy {
            ProxyState::Conflict(network) => {
                let holders = network
                    .conflict_ports
                    .iter()
                    .map(|port| {
                        let names = network
                            .holders
                            .iter()
                            .filter(|h| h.port == *port)
                            .map(|h| {
                                format!(
                                    "{}{}",
                                    h.process,
                                    h.container.as_ref().map_or_else(
                                        || h.unit
                                            .as_ref()
                                            .map_or_else(String::new, |u| format!(" ({u})")),
                                        |c| format!(" ({})", c.name)
                                    )
                                )
                            })
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect::<Vec<_>>();
                        format!(
                            ":{port} used by {}",
                            if names.is_empty() {
                                "another process or access restricted".into()
                            } else {
                                names.join(", ")
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                (
                    "conflict",
                    format!(
                        "{holders}, serving on {} / {}. Fix in panel > Settings > Network; run \"Move behind opsbox\" in the panel to keep your sites online",
                        network
                            .listen_http
                            .map_or_else(|| "?".into(), |a| a.to_string()),
                        network
                            .listen_https
                            .map_or_else(|| "?".into(), |a| a.to_string())
                    ),
                )
            }
            ProxyState::Ok {
                version,
                routes,
                certs,
            } => (
                "ok      ",
                format!("v{version}, {routes} route(s), {certs} cert(s)"),
            ),
            ProxyState::NotConfigured => (
                "off     ",
                "not running (no proxy unit/domain configured)".to_owned(),
            ),
            ProxyState::Down(why) => ("DOWN    ", why.clone()),
        };
        let _ = writeln!(
            out,
            "proxy    {tag} {}  {text}",
            self.proxy_socket.display()
        );

        let (tag, text) = match &self.watchdog {
            WatchdogState::Ok {
                version,
                failed,
                degraded,
            } => {
                if *failed {
                    (
                        "degraded",
                        format!("v{version}, service failure; see panel > Recovery"),
                    )
                } else if *degraded {
                    (
                        "warning ",
                        format!("v{version}, warnings reported; see panel > Recovery"),
                    )
                } else {
                    ("ok      ", format!("v{version}, monitoring"))
                }
            }
            WatchdogState::NotConfigured => (
                "off     ",
                "not running (opsbox-watchdog.service not installed)".to_owned(),
            ),
            WatchdogState::Down(why) => ("DOWN    ", why.clone()),
        };
        let _ = writeln!(out, "watchdog {tag} https://{}  {text}", self.watchdog_addr);
        let allowlist = if self.watchdog_allow_ips.is_empty() {
            "open (all source addresses)".to_owned()
        } else {
            format!(
                "restricted to {}",
                self.watchdog_allow_ips
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let _ = writeln!(out, "         allowlist: {allowlist}");
        let fingerprint = self
            .watchdog_cert_sha256
            .as_deref()
            .map(colon_fingerprint)
            .unwrap_or_else(|| "unavailable".into());
        let _ = writeln!(out, "         watchdog certificate SHA-256: {fingerprint}");
        if matches!(
            self.serve,
            ServeState::Down(_) | ServeState::Degraded { .. }
        ) {
            let _ = writeln!(out, "Logs: journalctl -u opsbox -n 50");
        }
        if matches!(self.privileged, PrivState::Down(_) | PrivState::Denied(_))
            || matches!(&self.privileged, PrivState::Ok { version, .. } if version != VERSION)
        {
            let _ = writeln!(out, "Logs: journalctl -u opsbox-priv -n 50");
        }
        if matches!(self.proxy, ProxyState::Down(_)) {
            let _ = writeln!(out, "Logs: journalctl -u opsbox-proxy -n 50");
        }
        if matches!(self.watchdog, WatchdogState::Down(_)) {
            let _ = writeln!(out, "Logs: journalctl -u opsbox-watchdog -n 50");
        }
        out
    }
}

pub fn run(config: &Config) -> bool {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("opsbox: error: cannot start the async runtime: {e}");
            return false;
        }
    };
    let report = runtime.block_on(collect(config));
    print!("{}", report.render());
    match runtime.block_on(PrivClient::new(&config.priv_socket).capacity("cli-status")) {
        Ok(s) => println!(
            "capacity     {} MB allocatable, {} MB reserved, pressure {}",
            s.budget(None).allocatable_mb,
            s.budget(None).reserved_mb
                + s.budget(None).os_mb
                + s.budget(None).opsbox_mb
                + s.budget(None).docker_mb,
            if s.budget(None).pressure {
                "high"
            } else {
                "normal"
            }
        ),
        Err(_) => println!("capacity     unavailable"),
    }
    report.healthy()
}

pub async fn collect(config: &Config) -> Report {
    let serve_addr = config.listen;
    let watchdog_addr = config.watchdog_listen;
    let proxy_expected = Path::new("/etc/systemd/system/opsbox-proxy.service").exists();
    let watchdog_expected = Path::new("/etc/systemd/system/opsbox-watchdog.service").exists();
    let ((mut serve, serve_transport), privileged, proxy, watchdog) = tokio::join!(
        probe_serve(serve_addr),
        probe_priv(&config.priv_socket),
        probe_proxy(&config.proxy_socket, proxy_expected),
        probe_watchdog(config, watchdog_expected),
    );
    if matches!(serve, ServeState::Degraded { ref detail, .. } if detail == "/readyz answered HTTP 403")
        && matches!(privileged, PrivState::Ok { .. })
        && let ServeState::Degraded { version, .. } = serve
    {
        serve = ServeState::Ready { version };
    }
    // Plain HTTP on a public address is refused at config load; if a listener like that is
    // answering anyway (an older opsbox, another program), say so instead of "ready".
    if serve_transport == ServeTransport::Http
        && !serve_addr.ip().to_canonical().is_loopback()
        && let ServeState::Ready { version } | ServeState::Degraded { version, .. } = serve
    {
        serve = ServeState::Degraded {
            version,
            detail: "answers plain HTTP on a public address; set api.listen to 127.0.0.1 or remove api.tls = false, then restart opsbox".into(),
        };
    }
    Report {
        config_file: config.loaded_from.clone(),
        serve_addr,
        serve,
        serve_transport,
        priv_socket: config.priv_socket.clone(),
        privileged,
        proxy_socket: config.proxy_socket.clone(),
        proxy,
        watchdog_addr,
        watchdog,
        watchdog_allow_ips: config.watchdog_allow_ips.clone(),
        watchdog_cert_sha256: opsbox_api::tls::cert_sha256_hex(
            &opsbox_watchdog::WatchdogConfig::default().cert_path,
        ),
    }
}

async fn probe_watchdog(config: &Config, expected: bool) -> WatchdogState {
    let listen = config.watchdog_listen;
    let addr = if listen.ip().is_unspecified() {
        SocketAddr::new(
            if listen.is_ipv6() {
                "::1".parse().unwrap()
            } else {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            },
            listen.port(),
        )
    } else {
        listen
    };
    // A restrictive list may intentionally exclude this machine. Root status
    // (including the installer) can use the watchdog's private, fresh snapshot.
    if !opsbox_watchdog::server::peer_allowed(&config.watchdog_allow_ips, addr.ip()) {
        let snapshot = std::fs::read(
            config
                .watchdog_runtime_dir
                .join(opsbox_watchdog::STATUS_FILE),
        )
        .ok()
        .and_then(|bytes| serde_json::from_slice::<opsbox_proto::WatchdogStatus>(&bytes).ok());
        if let Some(st) = snapshot {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            if st.running && (0..=45).contains(&(now - st.updated_at)) {
                return WatchdogState::Ok {
                    version: st.current_version,
                    failed: st.failed,
                    degraded: st.degraded,
                };
            }
        }
        return WatchdogState::Down("local probe excluded by watchdog.allow_ips; run sudo opsbox status to read the watchdog snapshot".into());
    }
    match http_get(addr, "/healthz").await {
        Ok(r) if r.status == 200 => {
            let version = r
                .json
                .as_ref()
                .and_then(|v| v["version"].as_str())
                .unwrap_or("?")
                .to_owned();
            let failed = r
                .json
                .as_ref()
                .and_then(|v| v["failed"].as_bool())
                .unwrap_or(false);
            let degraded = r
                .json
                .as_ref()
                .and_then(|v| v["degraded"].as_bool())
                .unwrap_or(false);
            WatchdogState::Ok {
                version,
                failed,
                degraded,
            }
        }
        Ok(r) => WatchdogState::Down(format!("answered /healthz with HTTP {}", r.status)),
        Err(e) => {
            if !expected
                && matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::TimedOut
                )
            {
                WatchdogState::NotConfigured
            } else {
                WatchdogState::Down(describe_http_error(&e))
            }
        }
    }
}

async fn probe_proxy(socket: &Path, expected: bool) -> ProxyState {
    let client = ProxyClient::new(socket).with_timeout(PROBE_TIMEOUT);
    match client.status("status").await {
        Ok(st) if st.network.port_conflict => ProxyState::Conflict(*st.network),
        Ok(st) => ProxyState::Ok {
            version: st.version,
            routes: st.routes_count,
            certs: st.certs.len(),
        },
        Err(ProxyClientError::Connect { source, .. }) => match source.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                if expected {
                    ProxyState::Down("not running (no process is listening on the socket)".into())
                } else {
                    ProxyState::NotConfigured
                }
            }
            io::ErrorKind::PermissionDenied => {
                ProxyState::Down("permission denied opening proxy socket".into())
            }
            _ => ProxyState::Down(format!("cannot connect: {source}")),
        },
        Err(ProxyClientError::Timeout) => ProxyState::Down("did not answer in time".into()),
        Err(e) => ProxyState::Down(e.to_string()),
    }
}

// ------------------------------------------------------------------ priv

async fn probe_priv(socket: &Path) -> PrivState {
    let client = PrivClient::new(socket).with_timeout(PROBE_TIMEOUT);
    let started = Instant::now();
    match client.ping("status").await {
        Ok(version) => PrivState::Ok {
            version,
            millis: started.elapsed().as_millis(),
        },
        Err(ClientError::Connect { source, .. }) => match source.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                PrivState::Down("not running (no process is listening on the socket)".into())
            }
            io::ErrorKind::PermissionDenied => PrivState::Denied(
                "the socket exists but this user may not open it (run as root or as a permitted user)".into(),
            ),
            _ => PrivState::Down(format!("cannot connect: {source}")),
        },
        Err(ClientError::Remote(e)) if e.code == PrivErrorCode::Forbidden => PrivState::Denied(
            "reachable, but this user is not on its allow-list (see allowed_users / --allow-uid)".into(),
        ),
        Err(ClientError::Timeout) => PrivState::Down("did not answer in time".into()),
        Err(e) => PrivState::Down(e.to_string()),
    }
}

// ------------------------------------------------------------------ serve

async fn probe_serve(listen: SocketAddr) -> (ServeState, ServeTransport) {
    // A wildcard listen address is reached through loopback.
    let addr = if listen.ip().is_unspecified() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), listen.port())
    } else {
        listen
    };

    let (transport, tls) = match opsbox_api::tls::probe_tls(addr, PROBE_TIMEOUT).await {
        Ok(tls) => (ServeTransport::Https(tls), true),
        Err(_) => (ServeTransport::Http, false),
    };
    let get = |path: &'static str| async move {
        if tls {
            http_get_tls(addr, path).await
        } else {
            http_get_plain(addr, path).await
        }
    };

    let health = match get("/healthz").await {
        Ok(r) => r,
        Err(e) => {
            return (
                ServeState::Down(describe_http_error(&e)),
                ServeTransport::Unknown,
            );
        }
    };
    if health.status != 200 {
        return (
            ServeState::Down(format!("answered /healthz with HTTP {}", health.status)),
            transport,
        );
    }
    let version = health
        .json
        .as_ref()
        .and_then(|v| v["version"].as_str())
        .unwrap_or("?")
        .to_owned();

    let state = match get("/readyz").await {
        Ok(r) if r.status == 200 => ServeState::Ready { version },
        Ok(r) => ServeState::Degraded {
            version,
            detail: failing_checks(r.json.as_ref())
                .unwrap_or_else(|| format!("/readyz answered HTTP {}", r.status)),
        },
        Err(e) => ServeState::Degraded {
            version,
            detail: describe_http_error(&e),
        },
    };
    (state, transport)
}

fn failing_checks(body: Option<&serde_json::Value>) -> Option<String> {
    let checks = body?.get("checks")?.as_object()?;
    let failing: Vec<String> = checks
        .iter()
        .filter(|(_, v)| v.as_str() != Some("ok"))
        .map(|(k, _)| k.replace('_', " "))
        .collect();
    (!failing.is_empty()).then(|| format!("{} failing", failing.join(", ")))
}

fn describe_http_error(e: &io::Error) -> String {
    match e.kind() {
        io::ErrorKind::ConnectionRefused => "not running (connection refused)".into(),
        io::ErrorKind::TimedOut => "did not answer in time".into(),
        _ => format!("cannot connect: {e}"),
    }
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    json: Option<serde_json::Value>,
}

async fn http_get(addr: SocketAddr, path: &str) -> io::Result<HttpResponse> {
    match http_get_tls(addr, path).await {
        Ok(r) => Ok(r),
        Err(_) => http_get_plain(addr, path).await,
    }
}

async fn http_get_tls(addr: SocketAddr, path: &str) -> io::Result<HttpResponse> {
    let url = format!("https://{addr}{path}");
    tokio::task::spawn_blocking(move || {
        let cfg = ureq::Agent::config_builder()
            .timeout_global(Some(PROBE_TIMEOUT))
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .disable_verification(true)
                    .build(),
            )
            .build();
        let agent: ureq::Agent = cfg.into();
        let resp = agent
            .get(&url)
            .header("Accept", "application/json")
            .call()?;
        let status = resp.status().as_u16();
        let body = resp.into_body().read_to_string().ok();
        let json = body.as_deref().and_then(|s| serde_json::from_str(s).ok());
        Ok::<_, ureq::Error>(HttpResponse { status, json })
    })
    .await
    .map_err(io::Error::other)?
    .map_err(io::Error::other)
}

/// Plain HTTP/1.1, for a loopback listener configured with `api.tls = false` (and tests).
async fn http_get_plain(addr: SocketAddr, path: &str) -> io::Result<HttpResponse> {
    tokio::time::timeout(PROBE_TIMEOUT, async {
        let mut stream = TcpStream::connect(addr).await?;
        let request = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: application/json\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await?;
        let mut raw = Vec::new();
        // Cap what we are willing to read from whatever is on that port.
        stream.take(64 * 1024).read_to_end(&mut raw).await?;
        parse_response(&raw).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an HTTP response"))
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timed out"))?
}

fn parse_response(raw: &[u8]) -> Option<HttpResponse> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..split]).ok()?;
    let status_line = head.lines().next()?;
    let mut parts = status_line.split_whitespace();
    if !parts.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let status = parts.next()?.parse().ok()?;
    let json = serde_json::from_slice(&raw[split + 4..]).ok();
    Some(HttpResponse { status, json })
}

/// Full, machine-readable-numbered policy table without HTTP authentication.
pub fn capacity(config: &Config) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let s = runtime.block_on(PrivClient::new(&config.priv_socket).capacity("cli-capacity"))?;
    let b = s.budget(None);
    println!(
        "{} vCPU ({}m quota), {} MB RAM, {} MB swap, cgroup v2: {}",
        s.profile.online_cpus,
        s.profile.cpu_millis,
        s.profile.memory_mb,
        s.profile.swap_mb,
        s.profile.cgroup_v2
    );
    println!(
        "OS {} MB; opsbox {} MB; Docker {} MB; workloads {} MB; allocatable {} MB (databases {} MB); pressure {}",
        b.os_mb,
        b.opsbox_mb,
        b.docker_mb,
        b.reserved_mb,
        b.allocatable_mb,
        b.database_allocatable_mb,
        b.pressure
    );
    for d in &s.profile.disks {
        println!(
            "{}: {} MB free / {} MB total",
            d.path, d.free_mb, d.total_mb
        );
    }
    for (feature, min, recommended, _) in opsbox_capacity::requirements::FEATURES {
        let fit = s.admit(&opsbox_capacity::Request {
            feature: *feature,
            ..Default::default()
        });
        println!(
            "{feature:?}: {} (min {min} MB, recommended {recommended} MB) {}",
            match fit {
                opsbox_capacity::Fit::Ok => "ok",
                opsbox_capacity::Fit::Tight { .. } => "tight",
                opsbox_capacity::Fit::Blocked { .. } => "blocked",
            },
            fit.reasons().join(" ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_response() {
        let raw = b"HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\n\r\n{\"status\":\"not_ready\",\"checks\":{\"database\":\"ok\",\"privileged_helper\":\"failing\"}}";
        let r = parse_response(raw).unwrap();
        assert_eq!(r.status, 503);
        assert_eq!(
            failing_checks(r.json.as_ref()).unwrap(),
            "privileged helper failing"
        );
    }

    #[test]
    fn rejects_things_that_are_not_http() {
        assert!(parse_response(b"SSH-2.0-OpenSSH_9.6\r\n\r\n").is_none());
        assert!(parse_response(b"garbage").is_none());
        assert!(parse_response(b"HTTP/1.1 abc OK\r\n\r\n").is_none());
    }

    #[test]
    fn non_json_bodies_are_tolerated() {
        let r = parse_response(b"HTTP/1.1 200 OK\r\n\r\nhello").unwrap();
        assert_eq!((r.status, r.json), (200, None));
    }

    /// Start a real `serve` (in-process, ephemeral port) and, optionally, a real helper.
    async fn live(
        with_priv: bool,
    ) -> (
        Config,
        Vec<tokio::sync::oneshot::Sender<()>>,
        tempfile::TempDir,
    ) {
        use opsbox_api::AppState;
        use opsbox_priv::{PrivConfig, PrivServer};
        use opsbox_store::Store;
        use std::os::unix::fs::MetadataExt;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let uid = {
            let probe = dir.path().join("probe");
            std::fs::write(&probe, b"").unwrap();
            std::fs::metadata(&probe).unwrap().uid()
        };
        let socket = dir.path().join("priv.sock");
        let mut stops = Vec::new();
        if with_priv {
            let server = PrivServer::bind(PrivConfig::new(&socket, vec![uid])).unwrap();
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            tokio::spawn(server.run(async {
                let _ = rx.await;
            }));
            stops.push(tx);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen = listener.local_addr().unwrap();
        let state = AppState::new(
            Arc::new(Store::open_in_memory().unwrap()),
            PrivClient::new(&socket),
            VERSION,
            opsbox_api::SecureCookies::Auto,
            opsbox_auth::AuthConfig::default(),
        );
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(opsbox_api::serve(listener, state, async {
            let _ = rx.await;
        }));
        stops.push(tx);

        let config = Config::load(
            Some(&write_config(dir.path(), listen, &socket)),
            crate::config::Overrides::default(),
        )
        .unwrap();
        (config, stops, dir)
    }

    fn write_config(dir: &Path, listen: SocketAddr, socket: &Path) -> PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            format!(
                "[api]\nlisten = \"{listen}\"\n[priv]\nsocket = \"{}\"\n",
                socket.display()
            ),
        )
        .unwrap();
        path
    }

    #[tokio::test]
    async fn reports_healthy_when_serve_and_priv_answer() {
        let (config, _stops, _dir) = live(true).await;
        let report = collect(&config).await;
        assert!(report.healthy(), "{}", report.render());
        assert!(matches!(report.serve, ServeState::Ready { .. }));
        assert!(matches!(report.privileged, PrivState::Ok { .. }));
    }

    #[tokio::test]
    async fn reports_degraded_serve_and_down_priv_when_the_helper_is_missing() {
        let (config, _stops, _dir) = live(false).await;
        let report = collect(&config).await;
        assert!(!report.healthy());
        match &report.serve {
            ServeState::Degraded { detail, .. } => {
                assert!(detail.contains("privileged helper"), "{detail}")
            }
            other => panic!("expected degraded, got {other:?}"),
        }
        assert!(
            matches!(report.privileged, PrivState::Down(_)),
            "{}",
            report.render()
        );
    }

    #[tokio::test]
    async fn reports_serve_down_when_nothing_listens() {
        let dir = tempfile::tempdir().unwrap();
        // Reserve a port, then free it, so nothing is listening there.
        let free = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let config = Config::load(
            Some(&write_config(
                dir.path(),
                free,
                &dir.path().join("none.sock"),
            )),
            crate::config::Overrides::default(),
        )
        .unwrap();
        let report = collect(&config).await;
        assert!(
            matches!(report.serve, ServeState::Down(_)),
            "{}",
            report.render()
        );
        assert!(!report.healthy());
    }

    fn bare_state(dir: &Path) -> opsbox_api::AppState {
        opsbox_api::AppState::new(
            std::sync::Arc::new(opsbox_store::Store::open_in_memory().unwrap()),
            PrivClient::new(dir.join("priv.sock")),
            VERSION,
            opsbox_api::SecureCookies::Auto,
            opsbox_auth::AuthConfig::default(),
        )
    }

    #[tokio::test]
    async fn reports_https_and_the_certificate_fingerprint_of_a_tls_listener() {
        let dir = tempfile::tempdir().unwrap();
        let (cert, key) = opsbox_api::tls::generate_self_signed(&[], None).unwrap();
        let (cert_path, key_path) = (dir.path().join("c.crt"), dir.path().join("c.key"));
        std::fs::write(&cert_path, cert).unwrap();
        std::fs::write(&key_path, key).unwrap();
        let want = opsbox_api::tls::cert_sha256_hex(&cert_path).unwrap();
        let acceptor = opsbox_api::tls::load_tls_acceptor(&cert_path, &key_path).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen = listener.local_addr().unwrap();
        let state = bare_state(dir.path()).with_tls(true);
        tokio::spawn(opsbox_api::tls::serve_tls(
            listener,
            acceptor,
            state,
            std::future::pending(),
        ));
        let config = Config::load(
            Some(&write_config(
                dir.path(),
                listen,
                &dir.path().join("priv.sock"),
            )),
            crate::config::Overrides::default(),
        )
        .unwrap();
        let report = collect(&config).await;
        let ServeTransport::Https(tls) = &report.serve_transport else {
            panic!("expected https: {}", report.render());
        };
        assert_eq!(tls.cert_sha256, want);
        let text = report.render();
        assert!(text.contains(&format!("https://{listen}")), "{text}");
        assert!(!text.contains(&format!("http://{listen}")), "{text}");
        assert!(text.contains(&colon_fingerprint(&want)), "{text}");
        assert!(text.contains("TLS 1."), "{text}");
    }

    #[tokio::test]
    async fn reports_plain_http_only_as_what_it_is_on_loopback() {
        let (config, _stops, _dir) = live(true).await;
        let report = collect(&config).await;
        assert_eq!(report.serve_transport, ServeTransport::Http);
        let text = report.render();
        assert!(
            text.contains(&format!("http://{}", config.listen)),
            "{text}"
        );
        assert!(!text.contains("certificate"), "{text}");
    }

    #[tokio::test]
    async fn plain_http_on_a_public_address_is_never_reported_as_ready() {
        let dir = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let listen = listener.local_addr().unwrap();
        tokio::spawn(opsbox_api::serve(
            listener,
            bare_state(dir.path()),
            std::future::pending(),
        ));
        let config = Config::load(
            Some(&write_config(
                dir.path(),
                listen,
                &dir.path().join("priv.sock"),
            )),
            crate::config::Overrides::default(),
        )
        .unwrap();
        let report = collect(&config).await;
        assert!(!report.healthy(), "{}", report.render());
        match &report.serve {
            ServeState::Degraded { detail, .. } => {
                assert!(
                    detail.contains("plain HTTP on a public address"),
                    "{detail}"
                )
            }
            other => panic!("expected degraded, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn watchdog_status_reads_fresh_snapshot_when_local_probe_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            format!(
                "[watchdog]\nallow_ips = [\"203.0.113.7\"]\nruntime_dir = \"{}\"\n",
                dir.path().display()
            ),
        )
        .unwrap();
        let config = Config::load(Some(&path), crate::config::Overrides::default()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let status_path = dir.path().join(opsbox_watchdog::STATUS_FILE);
        for (age, running, good) in [(0, true, true), (46, true, false), (0, false, false)] {
            let snapshot = opsbox_proto::WatchdogStatus {
                running,
                updated_at: now - age,
                current_version: VERSION.into(),
                ..Default::default()
            };
            std::fs::write(&status_path, serde_json::to_vec(&snapshot).unwrap()).unwrap();
            let state = probe_watchdog(&config, true).await;
            assert_eq!(matches!(state, WatchdogState::Ok { .. }), good);
        }
    }

    #[test]
    fn fingerprints_are_printed_the_way_browsers_print_them() {
        assert_eq!(colon_fingerprint("0aff10"), "0A:FF:10");
    }

    #[test]
    fn conflict_is_healthy_with_warning_and_does_not_claim_safe_mode() {
        let report = Report {
            config_file: None,
            serve_addr: "127.0.0.1:8443".parse().unwrap(),
            serve: ServeState::Ready {
                version: VERSION.into(),
            },
            serve_transport: ServeTransport::Http,
            priv_socket: "/run/opsbox/priv.sock".into(),
            privileged: PrivState::Ok {
                version: VERSION.into(),
                millis: 1,
            },
            proxy_socket: "/run/opsbox-proxy/proxy.sock".into(),
            proxy: ProxyState::Conflict(opsbox_proto::ProxyNetwork {
                running: true,
                port_conflict: true,
                conflict_ports: vec![80],
                listen_http: Some("0.0.0.0:8080".parse().unwrap()),
                listen_https: Some("0.0.0.0:443".parse().unwrap()),
                ..Default::default()
            }),
            watchdog_addr: "127.0.0.1:8444".parse().unwrap(),
            watchdog_allow_ips: Vec::new(),
            watchdog_cert_sha256: None,
            watchdog: WatchdogState::Ok {
                version: VERSION.into(),
                failed: false,
                degraded: true,
            },
        };
        assert!(report.healthy());
        let text = report.render();
        assert!(
            text.contains("conflict")
                && text.contains(":8080")
                && text.contains("Settings > Network")
        );
        assert!(text.contains("warning") && !text.contains("safe mode"));
        assert!(!text.contains("journalctl"));
    }

    #[test]
    fn rendering_marks_unhealthy_parts() {
        let report = Report {
            config_file: None,
            serve_addr: "127.0.0.1:8443".parse().unwrap(),
            serve: ServeState::Down("not running (connection refused)".into()),
            serve_transport: ServeTransport::Unknown,
            priv_socket: "/run/opsbox/priv.sock".into(),
            privileged: PrivState::Ok {
                version: VERSION.into(),
                millis: 1,
            },
            proxy_socket: "/run/opsbox-proxy/proxy.sock".into(),
            proxy: ProxyState::NotConfigured,
            watchdog_addr: "127.0.0.1:8444".parse().unwrap(),
            watchdog_allow_ips: Vec::new(),
            watchdog_cert_sha256: None,
            watchdog: WatchdogState::NotConfigured,
        };
        assert!(!report.healthy());
        let text = report.render();
        assert!(text.contains("DOWN"), "{text}");
        assert!(text.contains("connection refused"), "{text}");
        assert!(text.contains("none (built-in defaults)"), "{text}");
        assert!(
            text.contains("allowlist: open (all source addresses)"),
            "{text}"
        );
        assert!(
            text.contains("watchdog certificate SHA-256: unavailable"),
            "{text}"
        );
        let mut restricted = report;
        restricted.watchdog_allow_ips = vec!["203.0.113.7".parse().unwrap()];
        restricted.watchdog_cert_sha256 = Some("aabbcc".into());
        let text = restricted.render();
        assert!(
            text.contains("allowlist: restricted to 203.0.113.7"),
            "{text}"
        );
        assert!(
            text.contains("watchdog certificate SHA-256: AA:BB:CC"),
            "{text}"
        );
    }

    #[test]
    fn a_helper_from_another_build_is_not_healthy() {
        let mut report = Report {
            config_file: None,
            serve_addr: "127.0.0.1:8443".parse().unwrap(),
            serve: ServeState::Ready {
                version: VERSION.into(),
            },
            serve_transport: ServeTransport::Unknown,
            priv_socket: "/run/opsbox/priv.sock".into(),
            privileged: PrivState::Ok {
                version: "0.0.1-old".into(),
                millis: 1,
            },
            proxy_socket: "/run/opsbox-proxy/proxy.sock".into(),
            proxy: ProxyState::NotConfigured,
            watchdog_addr: "127.0.0.1:8444".parse().unwrap(),
            watchdog_allow_ips: Vec::new(),
            watchdog_cert_sha256: None,
            watchdog: WatchdogState::NotConfigured,
        };
        assert!(!report.healthy());
        let text = report.render();
        assert!(
            text.contains("MISMATCH") && text.contains("0.0.1-old"),
            "{text}"
        );
        assert!(text.contains("systemctl restart opsbox-priv"), "{text}");
        report.privileged = PrivState::Ok {
            version: VERSION.into(),
            millis: 1,
        };
        assert!(report.healthy());
    }
}
