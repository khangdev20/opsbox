//! `opsbox proxy` server: HTTP (:80), HTTPS (:443), Unix control socket, persisted
//! routes, and background 30-day ACME renewal.

use std::fs;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nix::unistd::geteuid;
use opsbox_priv::PrivClient;
use opsbox_proto::frame::{self, FrameError};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::crypto::ring;
use tracing::{debug, info, warn};

use crate::VERSION;
use crate::acme::{
    AcmeDirectory, AcmeManager, ChallengeMap, CustomHttpFactory, DynamicCertResolver,
};
use crate::domain::validate_domain;
use crate::proto::{
    DEFAULT_HTTP_ADDR, DEFAULT_HTTPS_ADDR, DEFAULT_SOCKET_PATH, DEFAULT_STATE_DIR, ProxyErrorCode,
    ProxyOp, ProxyReply, ProxyRequest, ProxyResponse, ProxyStatus, Route, read_body, write_frame,
};
use crate::proxy::{
    DEFAULT_MAX_BODY_BYTES, RouteTable, serve_http_connection, serve_https_connection,
};

const MAX_CTRL_CONNS: usize = 16;
const CTRL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const RENEW_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone)]
pub struct ProxyConfig {
    pub listen_http: SocketAddr,
    pub listen_https: SocketAddr,
    pub socket_path: PathBuf,
    pub state_dir: PathBuf,
    pub priv_socket: PathBuf,
    pub allowed_uids: Vec<u32>,
    pub acme_directory: AcmeDirectory,
    pub acme_email: Option<String>,
    pub max_body_bytes: u64,
    pub custom_http: Option<CustomHttpFactory>,
}

impl std::fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("listen_http", &self.listen_http)
            .field("listen_https", &self.listen_https)
            .field("socket_path", &self.socket_path)
            .field("state_dir", &self.state_dir)
            .field("priv_socket", &self.priv_socket)
            .field("allowed_uids", &self.allowed_uids)
            .field("acme_directory", &self.acme_directory)
            .field("acme_email", &self.acme_email)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("custom_http", &self.custom_http.is_some())
            .finish()
    }
}

impl ProxyConfig {
    pub fn new(priv_socket: impl Into<PathBuf>) -> Self {
        Self {
            listen_http: DEFAULT_HTTP_ADDR.parse().expect("valid default"),
            listen_https: DEFAULT_HTTPS_ADDR.parse().expect("valid default"),
            socket_path: PathBuf::from(DEFAULT_SOCKET_PATH),
            state_dir: PathBuf::from(DEFAULT_STATE_DIR),
            priv_socket: priv_socket.into(),
            allowed_uids: vec![geteuid().as_raw(), 0],
            acme_directory: AcmeDirectory::Production,
            acme_email: None,
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            custom_http: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProxyServerError {
    #[error("cannot prepare proxy socket {path}: {source}")]
    PrepareSocket { path: String, source: io::Error },
    #[error("another opsbox proxy is already listening on {0}")]
    AlreadyRunning(String),
    #[error("{0} exists and is not a socket; refusing to remove it")]
    NotASocket(String),
    #[error("cannot listen on {addr}: {source}")]
    Bind { addr: SocketAddr, source: io::Error },
    #[error("TLS configuration error: {0}")]
    Tls(String),
}

pub struct ProxyServer {
    http_listener: TcpListener,
    https_listener: TcpListener,
    ctrl_listener: UnixListener,
    http_addr: SocketAddr,
    https_addr: SocketAddr,
    config: ProxyConfig,
    routes: RouteTable,
    challenges: ChallengeMap,
    resolver: Arc<DynamicCertResolver>,
    acme: Arc<AcmeManager>,
}

impl ProxyServer {
    pub async fn bind(config: ProxyConfig) -> Result<Self, ProxyServerError> {
        let _ = ring::default_provider().install_default();

        let ctrl_listener = bind_control_socket(&config.socket_path)?;
        let http_listener = TcpListener::bind(config.listen_http)
            .await
            .map_err(|source| ProxyServerError::Bind {
                addr: config.listen_http,
                source,
            })?;
        let https_listener = TcpListener::bind(config.listen_https)
            .await
            .map_err(|source| ProxyServerError::Bind {
                addr: config.listen_https,
                source,
            })?;

        let http_addr = http_listener
            .local_addr()
            .map_err(|source| ProxyServerError::Bind {
                addr: config.listen_http,
                source,
            })?;
        let https_addr = https_listener
            .local_addr()
            .map_err(|source| ProxyServerError::Bind {
                addr: config.listen_https,
                source,
            })?;

        let routes = RouteTable::default();
        if let Some(persisted_routes) = load_routes(&config.state_dir) {
            routes.set(persisted_routes);
        }

        let challenges = ChallengeMap::default();
        let resolver = Arc::new(DynamicCertResolver::new());
        let priv_client =
            PrivClient::new(&config.priv_socket).with_timeout(Duration::from_secs(15));
        let mut acme = AcmeManager::new(
            &config.state_dir,
            config.acme_directory.clone(),
            config.acme_email.clone(),
            priv_client,
            resolver.clone(),
            challenges.clone(),
        );
        if let Some(ref factory) = config.custom_http {
            acme = acme.with_custom_http(factory.clone());
        }
        let acme = Arc::new(acme);

        Ok(Self {
            http_listener,
            https_listener,
            ctrl_listener,
            http_addr,
            https_addr,
            config,
            routes,
            challenges,
            resolver,
            acme,
        })
    }

    pub fn http_addr(&self) -> SocketAddr {
        self.http_addr
    }

    pub fn https_addr(&self) -> SocketAddr {
        self.https_addr
    }

    pub fn socket_path(&self) -> &Path {
        &self.config.socket_path
    }

    pub fn acme(&self) -> Arc<AcmeManager> {
        Arc::clone(&self.acme)
    }

    pub fn routes(&self) -> RouteTable {
        self.routes.clone()
    }

    pub async fn run(self, shutdown: impl Future<Output = ()>) -> Result<(), ProxyServerError> {
        let _ = fs::create_dir_all(&self.config.state_dir);
        let _ = fs::set_permissions(&self.config.state_dir, fs::Permissions::from_mode(0o700));

        if let Err(e) = self.acme.load_persisted().await {
            warn!(error = %e, "could not read persisted proxy certificates");
        }

        let provider = Arc::new(ring::default_provider());
        let tls_cfg = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| ProxyServerError::Tls(e.to_string()))?
            .with_no_client_auth()
            .with_cert_resolver(self.resolver.clone());
        let acceptor = TlsAcceptor::from(Arc::new(tls_cfg));

        info!(
            http = %self.http_addr,
            https = %self.https_addr,
            socket = %self.config.socket_path.display(),
            staging = self.config.acme_directory.is_staging(),
            "opsbox proxy started"
        );

        let acme_bg = Arc::clone(&self.acme);
        let renew_task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(RENEW_CHECK_INTERVAL);
            loop {
                tick.tick().await;
                acme_bg.renew_due_certs().await;
            }
        });

        let permits = Arc::new(Semaphore::new(MAX_CTRL_CONNS));
        let mut allowed_uids = self.config.allowed_uids.clone();
        allowed_uids.sort_unstable();
        allowed_uids.dedup();
        let allowed_uids: Arc<[u32]> = allowed_uids.into();

        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                () = &mut shutdown => break,
                accepted = self.http_listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => {
                            let routes = self.routes.clone();
                            let challenges = self.challenges.clone();
                            let max_body = self.config.max_body_bytes;
                            tokio::spawn(serve_http_connection(stream, peer, routes, challenges, max_body));
                        }
                        Err(e) => {
                            warn!(error = %e, "HTTP accept failed");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
                accepted = self.https_listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => {
                            let acceptor = acceptor.clone();
                            let routes = self.routes.clone();
                            let max_body = self.config.max_body_bytes;
                            tokio::spawn(serve_https_connection(stream, peer, acceptor, routes, max_body));
                        }
                        Err(e) => {
                            warn!(error = %e, "HTTPS accept failed");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
                accepted = self.ctrl_listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            let permits = Arc::clone(&permits);
                            let allowed = Arc::clone(&allowed_uids);
                            let routes = self.routes.clone();
                            let acme = Arc::clone(&self.acme);
                            let state_dir = self.config.state_dir.clone();
                            let staging = self.config.acme_directory.is_staging();
                            tokio::spawn(async move {
                                let Ok(_permit) = permits.acquire_owned().await else { return };
                                handle_ctrl_conn(stream, &allowed, routes, acme, &state_dir, staging).await;
                            });
                        }
                        Err(e) => {
                            warn!(error = %e, "proxy control accept failed");
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                    }
                }
            }
        }

        renew_task.abort();
        info!("opsbox proxy stopped");
        if let Err(e) = fs::remove_file(&self.config.socket_path) {
            debug!(error = %e, "could not remove proxy socket file");
        }
        Ok(())
    }
}

fn bind_control_socket(path: &Path) -> Result<UnixListener, ProxyServerError> {
    let prep = |source| ProxyServerError::PrepareSocket {
        path: path.display().to_string(),
        source,
    };
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).map_err(prep)?;
    }
    if let Ok(meta) = fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            return Err(ProxyServerError::NotASocket(path.display().to_string()));
        }
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Err(ProxyServerError::AlreadyRunning(path.display().to_string()));
        }
        fs::remove_file(path).map_err(prep)?;
    }
    let std_listener = std::os::unix::net::UnixListener::bind(path).map_err(prep)?;
    std_listener.set_nonblocking(true).map_err(prep)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660)).map_err(prep)?;
    UnixListener::from_std(std_listener).map_err(prep)
}

async fn handle_ctrl_conn(
    mut stream: UnixStream,
    allowed_uids: &[u32],
    routes: RouteTable,
    acme: Arc<AcmeManager>,
    state_dir: &Path,
    staging: bool,
) {
    let Ok(cred) = stream.peer_cred() else {
        let _ = write_frame(
            &mut stream,
            &ProxyResponse::failure("", ProxyErrorCode::Forbidden, "Could not verify peer."),
        )
        .await;
        return;
    };
    if !allowed_uids.contains(&cred.uid()) {
        warn!(uid = cred.uid(), "rejected proxy control connection");
        let _ = write_frame(
            &mut stream,
            &ProxyResponse::failure(
                "",
                ProxyErrorCode::Forbidden,
                "This user may not control opsbox-proxy.",
            ),
        )
        .await;
        return;
    }

    loop {
        let body = match tokio::time::timeout(CTRL_IDLE_TIMEOUT, read_body(&mut stream)).await {
            Ok(Ok(Some(b))) => b,
            Ok(Ok(None)) | Err(_) => return,
            Ok(Err(crate::proto::ReadError::Frame(e @ FrameError::TooLarge(_)))) => {
                let _ = write_frame(
                    &mut stream,
                    &ProxyResponse::failure("", ProxyErrorCode::BadRequest, e.to_string()),
                )
                .await;
                return;
            }
            Ok(Err(_)) => return,
        };

        let req = match parse_ctrl_request(&body) {
            Ok(r) => r,
            Err(resp) => {
                if write_frame(&mut stream, &resp).await.is_err() {
                    return;
                }
                continue;
            }
        };

        let resp = dispatch_ctrl(req, &routes, &acme, state_dir, staging).await;
        if write_frame(&mut stream, &resp).await.is_err() {
            return;
        }
    }
}

fn parse_ctrl_request(body: &[u8]) -> Result<ProxyRequest, Box<ProxyResponse>> {
    let val: serde_json::Value = frame::decode(body).map_err(|e| {
        Box::new(ProxyResponse::failure(
            "",
            ProxyErrorCode::BadRequest,
            e.to_string(),
        ))
    })?;
    let id = val
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    serde_json::from_value(val).map_err(|_| {
        Box::new(ProxyResponse::failure(
            id,
            ProxyErrorCode::BadRequest,
            "Unknown proxy control operation.",
        ))
    })
}

async fn dispatch_ctrl(
    req: ProxyRequest,
    routes: &RouteTable,
    acme: &Arc<AcmeManager>,
    state_dir: &Path,
    staging: bool,
) -> ProxyResponse {
    let id = req.id.clone();
    match req.op {
        ProxyOp::Ping => ProxyResponse::success(
            id,
            ProxyReply::Pong {
                version: VERSION.to_owned(),
            },
        ),
        ProxyOp::Status => ProxyResponse::success(
            id,
            ProxyReply::Status(ProxyStatus {
                version: VERSION.to_owned(),
                routes_count: routes.count(),
                certs: acme.list_certs(),
                acme_staging: staging,
            }),
        ),
        ProxyOp::SetRoutes { routes: new_routes } => {
            for r in &new_routes {
                if let Err(e) = validate_domain(&r.host) {
                    return ProxyResponse::failure(id, ProxyErrorCode::Invalid, e.0);
                }
                if r.upstream.parse::<SocketAddr>().is_err() {
                    return ProxyResponse::failure(
                        id,
                        ProxyErrorCode::Invalid,
                        format!("Invalid upstream socket address '{}'.", r.upstream),
                    );
                }
            }
            if let Err(e) = save_routes(state_dir, &new_routes) {
                warn!(error = %e, "could not persist proxy routes");
            }
            routes.set(new_routes);
            ProxyResponse::success(id, ProxyReply::Ok)
        }
        ProxyOp::IssueCert { domain } => match acme.issue_cert(&id, &domain).await {
            Ok(info) => ProxyResponse::success(id, ProxyReply::Cert(info)),
            Err(e) => {
                let code = match e {
                    crate::acme::AcmeError::Invalid(_) => ProxyErrorCode::Invalid,
                    _ => ProxyErrorCode::Internal,
                };
                ProxyResponse::failure(id, code, e.to_string())
            }
        },
        ProxyOp::RemoveCert { domain } => match acme.remove_cert(&domain).await {
            Ok(()) => ProxyResponse::success(id, ProxyReply::Ok),
            Err(e) => ProxyResponse::failure(id, ProxyErrorCode::Invalid, e.to_string()),
        },
    }
}

fn routes_path(state_dir: &Path) -> PathBuf {
    state_dir.join("routes.json")
}

fn load_routes(state_dir: &Path) -> Option<Vec<Route>> {
    let raw = fs::read_to_string(routes_path(state_dir)).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_routes(state_dir: &Path, routes: &[Route]) -> io::Result<()> {
    fs::create_dir_all(state_dir)?;
    let _ = fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700));
    let path = routes_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_vec_pretty(routes).map_err(io::Error::other)?,
    )?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp, path)?;
    Ok(())
}
