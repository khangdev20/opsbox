//! `opsbox status`: is `serve` answering, is `priv` answering?
//!
//! `serve` is probed over HTTP on its own listen address (`/healthz`, then
//! `/readyz`); `priv` by sending it a `Ping` on its socket. Needs no database access
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

#[derive(Debug, PartialEq, Eq)]
pub enum PrivState {
    Ok { version: String, millis: u128 },
    Down(String),
    Denied(String),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProxyState {
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
    pub priv_socket: PathBuf,
    pub privileged: PrivState,
    pub proxy_socket: PathBuf,
    pub proxy: ProxyState,
    pub watchdog_addr: SocketAddr,
    pub watchdog: WatchdogState,
}

impl Report {
    pub fn healthy(&self) -> bool {
        matches!(self.serve, ServeState::Ready { .. })
            && matches!(self.privileged, PrivState::Ok { .. })
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
        let _ = writeln!(out, "serve    {tag} http://{}  {text}", self.serve_addr);

        let (tag, text) = match &self.privileged {
            PrivState::Ok { version, millis } => {
                let note = if version == VERSION {
                    String::new()
                } else {
                    format!(" (this binary is v{VERSION}; restart opsbox-priv after an update)")
                };
                (
                    "ok      ",
                    format!("v{version}, answered in {millis} ms{note}"),
                )
            }
            PrivState::Down(why) => ("DOWN    ", why.clone()),
            PrivState::Denied(why) => ("DENIED  ", why.clone()),
        };
        let _ = writeln!(out, "priv     {tag} {}  {text}", self.priv_socket.display());

        let (tag, text) = match &self.proxy {
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
                if *failed || *degraded {
                    ("degraded", format!("v{version}, safe mode alert active"))
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
    report.healthy()
}

pub async fn collect(config: &Config) -> Report {
    let serve_addr = config.listen;
    let watchdog_addr = config.watchdog_listen;
    let proxy_expected = Path::new("/etc/systemd/system/opsbox-proxy.service").exists();
    let watchdog_expected = Path::new("/etc/systemd/system/opsbox-watchdog.service").exists();
    let (mut serve, privileged, proxy, watchdog) = tokio::join!(
        probe_serve(serve_addr),
        probe_priv(&config.priv_socket),
        probe_proxy(&config.proxy_socket, proxy_expected),
        probe_watchdog(watchdog_addr, watchdog_expected),
    );
    if matches!(serve, ServeState::Degraded { ref detail, .. } if detail == "/readyz answered HTTP 403")
        && matches!(privileged, PrivState::Ok { .. })
        && let ServeState::Degraded { version, .. } = serve
    {
        serve = ServeState::Ready { version };
    }
    Report {
        config_file: config.loaded_from.clone(),
        serve_addr,
        serve,
        priv_socket: config.priv_socket.clone(),
        privileged,
        proxy_socket: config.proxy_socket.clone(),
        proxy,
        watchdog_addr,
        watchdog,
    }
}

async fn probe_watchdog(listen: SocketAddr, expected: bool) -> WatchdogState {
    let addr = if listen.ip().is_unspecified() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), listen.port())
    } else {
        listen
    };
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

async fn probe_serve(listen: SocketAddr) -> ServeState {
    // A wildcard listen address is reached through loopback.
    let addr = if listen.ip().is_unspecified() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), listen.port())
    } else {
        listen
    };

    let health = match http_get(addr, "/healthz").await {
        Ok(r) => r,
        Err(e) => return ServeState::Down(describe_http_error(&e)),
    };
    if health.status != 200 {
        return ServeState::Down(format!("answered /healthz with HTTP {}", health.status));
    }
    let version = health
        .json
        .as_ref()
        .and_then(|v| v["version"].as_str())
        .unwrap_or("?")
        .to_owned();

    match http_get(addr, "/readyz").await {
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
    }
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
    let url = format!("https://{addr}{path}");
    let tls_res = tokio::task::spawn_blocking(move || {
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
    .map_err(io::Error::other)?;
    match tls_res {
        Ok(r) => Ok(r),
        Err(_) => http_get_plain(addr, path).await,
    }
}

/// Plain HTTP/1.1 fallback for tests that bind a plain TCP listener.
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
            opsbox_api::SecureCookies::Never,
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

    #[test]
    fn rendering_marks_unhealthy_parts() {
        let report = Report {
            config_file: None,
            serve_addr: "127.0.0.1:8443".parse().unwrap(),
            serve: ServeState::Down("not running (connection refused)".into()),
            priv_socket: "/run/opsbox/priv.sock".into(),
            privileged: PrivState::Ok {
                version: VERSION.into(),
                millis: 1,
            },
            proxy_socket: "/run/opsbox-proxy/proxy.sock".into(),
            proxy: ProxyState::NotConfigured,
            watchdog_addr: "127.0.0.1:8444".parse().unwrap(),
            watchdog: WatchdogState::NotConfigured,
        };
        assert!(!report.healthy());
        let text = report.render();
        assert!(text.contains("DOWN"), "{text}");
        assert!(text.contains("connection refused"), "{text}");
        assert!(text.contains("none (built-in defaults)"), "{text}");
    }
}
