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
use opsbox_proto::{ListenerAction, PrivOp, PrivReply, ProxyNetwork};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::{RwLock, Semaphore, mpsc, oneshot};
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
    DEFAULT_MAX_BODY_BYTES, ListenerPorts, RouteTable, serve_http_connection,
    serve_https_connection,
};

const MAX_CTRL_CONNS: usize = 16;
const CTRL_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const RENEW_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone)]
pub struct ProxyConfig {
    pub listen_http: SocketAddr,
    pub listen_https: SocketAddr,
    pub fallback_http: u16,
    pub fallback_https: u16,
    pub retry_interval: Duration,
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
            fallback_http: 8080,
            fallback_https: 8445,
            retry_interval: Duration::from_secs(30),
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
    network: Arc<RwLock<ProxyNetwork>>,
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
        let (http_listener, http_conflict) =
            bind_or_fallback(config.listen_http, config.fallback_http, 8080..=8099).await?;
        let (https_listener, https_conflict) =
            bind_or_fallback(config.listen_https, config.fallback_https, 8445..=8464).await?;
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

        if let Ok(raw) = fs::read(config.state_dir.join("adopted.json")) {
            let adopted =
                serde_json::from_slice(&raw).map_err(|e| ProxyServerError::Tls(e.to_string()))?;
            routes.set_adopted(adopted);
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
            network: Arc::new(RwLock::new(ProxyNetwork {
                running: true,
                port_conflict: http_conflict || https_conflict,
                listen_http: Some(http_addr),
                listen_https: Some(https_addr),
                primary_http: Some(config.listen_http),
                primary_https: Some(config.listen_https),
                conflict_ports: [
                    (http_conflict, config.listen_http.port()),
                    (https_conflict, config.listen_https.port()),
                ]
                .into_iter()
                .filter_map(|(conflict, port)| conflict.then_some(port))
                .collect(),
                holders: Vec::new(),
            })),
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

    async fn change_listeners(&mut self, action: ListenerAction) -> Result<(), ProxyServerError> {
        let mut network = self.network.write().await;
        if action == ListenerAction::ClaimPrimary {
            // Nginx reload returns before draining workers release their accept FDs.
            // Try immediately, then briefly await that handoff (never the 30s retry).
            let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
            let http = if network.listen_http != Some(self.config.listen_http) {
                Some(bind_cutover(self.config.listen_http, deadline).await?)
            } else {
                None
            };
            let https = if network.listen_https != Some(self.config.listen_https) {
                Some(bind_cutover(self.config.listen_https, deadline).await?)
            } else {
                None
            };
            if let Some(new) = http {
                network.listen_http = new.local_addr().ok();
                self.http_listener = new;
            }
            if let Some(new) = https {
                network.listen_https = new.local_addr().ok();
                self.https_listener = new;
            }
        } else if action == ListenerAction::KeepFallback {
            self.config.listen_http =
                self.http_listener
                    .local_addr()
                    .map_err(|source| ProxyServerError::Bind {
                        addr: self.config.listen_http,
                        source,
                    })?;
            self.config.listen_https =
                self.https_listener
                    .local_addr()
                    .map_err(|source| ProxyServerError::Bind {
                        addr: self.config.listen_https,
                        source,
                    })?;
        } else {
            // Bind both replacements before releasing either primary port.
            let http = bind_fallback(
                self.config.listen_http,
                self.config.fallback_http,
                8080..=8099,
                network.listen_http,
            )
            .await?;
            let https = bind_fallback(
                self.config.listen_https,
                self.config.fallback_https,
                8445..=8464,
                network.listen_https,
            )
            .await?;
            if let Some(new) = http {
                network.listen_http = new.local_addr().ok();
                self.http_listener = new;
            }
            if let Some(new) = https {
                network.listen_https = new.local_addr().ok();
                self.https_listener = new;
            }
        }
        refresh_conflicts(&mut network, &self.config);
        Ok(())
    }

    pub async fn run(mut self, shutdown: impl Future<Output = ()>) -> Result<(), ProxyServerError> {
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
        let traffic_permits = Arc::new(Semaphore::new(1024));
        let mut allowed_uids = self.config.allowed_uids.clone();
        allowed_uids.sort_unstable();
        allowed_uids.dedup();
        let allowed_uids: Arc<[u32]> = allowed_uids.into();

        let (commands, mut command_rx) = mpsc::channel::<ListenerCommand>(4);
        let mut retry = tokio::time::interval(self.config.retry_interval);
        retry.tick().await;
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                () = &mut shutdown => break,
                _ = retry.tick() => {
                    // Replacing only the accept sockets lets already accepted requests drain.
                    let mut network = self.network.write().await;
                    for https in [false, true] {
                        let (listener, primary, actual) = if https {
                            (&mut self.https_listener, &mut self.config.listen_https, &mut network.listen_https)
                        } else {
                            (&mut self.http_listener, &mut self.config.listen_http, &mut network.listen_http)
                        };
                        if actual.is_some_and(|a| a != *primary && primary.port() != 0) {
                            match TcpListener::bind(*primary).await {
                                Ok(new) => { *actual = new.local_addr().ok(); *listener = new; }
                                Err(e) if is_port_conflict(&e) => {},
                                Err(source) => return Err(ProxyServerError::Bind {addr: *primary, source}),
                            }
                        }
                    }
                    refresh_conflicts(&mut network, &self.config);
                }
                Some(command) = command_rx.recv() => {
                    let result = self.change_listeners(command.action).await;
                    retry.reset();
                    let _ = command.reply.send(result.map_err(|e| e.to_string()));
                }
                accepted = self.http_listener.accept() => {
                    match accepted {
                        Ok((stream, peer)) => {
                            let Ok(permit) = traffic_permits.clone().try_acquire_owned() else { continue; };
                            let routes = self.routes.clone();
                            let challenges = self.challenges.clone();
                            let certs = Arc::clone(&self.resolver);
                            let max_body = self.config.max_body_bytes;
                            let ports = ListenerPorts { http: if self.config.listen_http.port() == 0 {80} else {self.http_listener.local_addr().map_or(80, |a| a.port())}, https: if self.config.listen_https.port() == 0 {443} else {self.https_listener.local_addr().map_or(443, |a| a.port())} };
                            tokio::spawn(async move { let _permit = permit; serve_http_connection(
                                stream, peer, routes, challenges, certs, max_body,
                                ports,
                            ).await; });
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
                            let Ok(permit) = traffic_permits.clone().try_acquire_owned() else { continue; };
                            let acceptor = acceptor.clone();
                            let routes = self.routes.clone();
                            let max_body = self.config.max_body_bytes;
                            let port = if self.config.listen_https.port() == 0 {443} else {self.https_listener.local_addr().map_or(443, |a| a.port())};
                            tokio::spawn(async move { let _permit = permit; serve_https_connection(stream, peer, acceptor, routes, max_body, port).await; });
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
                            let network = self.network.clone();
                            let commands = commands.clone();
                            let priv_client = PrivClient::new(&self.config.priv_socket).with_timeout(Duration::from_millis(500));
                            tokio::spawn(async move {
                                let Ok(_permit) = permits.acquire_owned().await else { return };
                                handle_ctrl_conn(stream, &allowed, routes, acme, &state_dir, staging, ListenerControl { network, commands, priv_client }).await;
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
    // Panel and proxy have separate groups. Kernel UID checks below authenticate
    // control clients before parsing; granting connect does not grant operations.
    fs::set_permissions(path, fs::Permissions::from_mode(0o666)).map_err(prep)?;
    UnixListener::from_std(std_listener).map_err(prep)
}

async fn handle_ctrl_conn(
    mut stream: UnixStream,
    allowed_uids: &[u32],
    routes: RouteTable,
    acme: Arc<AcmeManager>,
    state_dir: &Path,
    staging: bool,
    control: ListenerControl,
) {
    let ListenerControl {
        network,
        commands,
        priv_client,
    } = control;
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

        let resp = if let ProxyOp::Listeners { action } = req.op {
            // System changes are root-only; the ordinary control allow-list cannot take over ports.
            if cred.uid() != 0 {
                ProxyResponse::failure(
                    req.id,
                    ProxyErrorCode::Forbidden,
                    "Listener changes require the privileged helper.",
                )
            } else {
                let (reply, answer) = oneshot::channel();
                if commands
                    .send(ListenerCommand { action, reply })
                    .await
                    .is_err()
                {
                    return;
                }
                match answer.await {
                    Ok(Ok(())) => ProxyResponse::success(req.id, ProxyReply::Ok),
                    _ => ProxyResponse::failure(
                        req.id,
                        ProxyErrorCode::Internal,
                        "Cannot change proxy listeners.",
                    ),
                }
            }
        } else if matches!(
            req.op,
            ProxyOp::SetAdopted { .. } | ProxyOp::ReloadCerts | ProxyOp::ImportCert { .. }
        ) && cred.uid() != 0
        {
            ProxyResponse::failure(
                req.id,
                ProxyErrorCode::Forbidden,
                "Adoption changes require the privileged helper.",
            )
        } else {
            let mut snapshot = network.read().await.clone();
            if matches!(req.op, ProxyOp::Status)
                && !snapshot.conflict_ports.is_empty()
                && let Ok(PrivReply::PortHolders(holders)) = priv_client
                    .call(
                        &req.id,
                        None,
                        PrivOp::PortHolders {
                            ports: snapshot.conflict_ports.clone(),
                        },
                    )
                    .await
            {
                snapshot.holders = holders;
            }
            dispatch_ctrl(req, &routes, &acme, state_dir, staging, snapshot).await
        };
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
    network: ProxyNetwork,
) -> ProxyResponse {
    let id = req.id.clone();
    match req.op {
        ProxyOp::Listeners { .. } => unreachable!("handled by root-only control dispatcher"),
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
                network: Box::new(network),
                adopted: routes.adopted(),
            }),
        ),
        ProxyOp::ImportCert {
            domain,
            cert_pem,
            key_sealed,
        } => match acme
            .import_sealed(&id, &domain, &cert_pem, &key_sealed)
            .await
        {
            Ok(()) => ProxyResponse::success(id, ProxyReply::Ok),
            Err(_) => ProxyResponse::failure(
                id,
                ProxyErrorCode::Invalid,
                "Cannot import the existing certificate.",
            ),
        },
        ProxyOp::ReloadCerts => match acme.load_persisted().await {
            Ok(_) => ProxyResponse::success(id, ProxyReply::Ok),
            Err(_) => {
                ProxyResponse::failure(id, ProxyErrorCode::Internal, "Cannot load certificate.")
            }
        },
        ProxyOp::SetAdopted { routes: adopted } => {
            if adopted.domains.len() > 256
                || adopted
                    .domains
                    .iter()
                    .any(|d| validate_domain(&d.domain).is_err())
                || (!adopted.domains.is_empty() || adopted.everything_else)
                    && (adopted.http_port < 1024
                        || adopted.https_port < 1024
                        || adopted.http_port == adopted.https_port)
            {
                return ProxyResponse::failure(
                    id,
                    ProxyErrorCode::Invalid,
                    "Invalid adopted routes.",
                );
            }
            if fs::create_dir_all(state_dir)
                .and_then(|()| {
                    let tmp = state_dir.join("adopted.json.tmp");
                    fs::write(
                        &tmp,
                        serde_json::to_vec(&adopted).map_err(io::Error::other)?,
                    )?;
                    fs::rename(tmp, state_dir.join("adopted.json"))
                })
                .is_err()
            {
                return ProxyResponse::failure(
                    id,
                    ProxyErrorCode::Internal,
                    "Cannot persist adopted routes.",
                );
            }
            routes.set_adopted(adopted);
            ProxyResponse::success(id, ProxyReply::Ok)
        }
        ProxyOp::SetRoutes { routes: new_routes } => {
            for r in &new_routes {
                if let Err(e) = validate_domain(&r.host) {
                    return ProxyResponse::failure(id, ProxyErrorCode::Invalid, e.0);
                }
                if let Some(ref target) = r.redirect_to {
                    if let Err(e) = validate_domain(target) {
                        return ProxyResponse::failure(id, ProxyErrorCode::Invalid, e.0);
                    }
                    if target.eq_ignore_ascii_case(&r.host) {
                        return ProxyResponse::failure(
                            id,
                            ProxyErrorCode::Invalid,
                            format!("Domain '{}' cannot redirect to itself.", r.host),
                        );
                    }
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

struct ListenerCommand {
    action: ListenerAction,
    reply: oneshot::Sender<Result<(), String>>,
}
fn is_port_conflict(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::AddrInUse | io::ErrorKind::PermissionDenied
    )
}
fn refresh_conflicts(network: &mut ProxyNetwork, config: &ProxyConfig) {
    network.primary_http = Some(config.listen_http);
    network.primary_https = Some(config.listen_https);
    network.conflict_ports = [
        (network.listen_http, config.listen_http),
        (network.listen_https, config.listen_https),
    ]
    .into_iter()
    .filter_map(|(actual, primary)| {
        (primary.port() != 0 && actual != Some(primary)).then_some(primary.port())
    })
    .collect();
    network.port_conflict = !network.conflict_ports.is_empty();
}
async fn bind_cutover(
    addr: SocketAddr,
    deadline: tokio::time::Instant,
) -> Result<TcpListener, ProxyServerError> {
    loop {
        match TcpListener::bind(addr).await {
            Ok(listener) => return Ok(listener),
            Err(source)
                if source.kind() == io::ErrorKind::AddrInUse
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await
            }
            Err(source) => return Err(ProxyServerError::Bind { addr, source }),
        }
    }
}
async fn bind_or_fallback(
    primary: SocketAddr,
    preferred: u16,
    range: std::ops::RangeInclusive<u16>,
) -> Result<(TcpListener, bool), ProxyServerError> {
    match TcpListener::bind(primary).await {
        Ok(listener) => Ok((listener, false)),
        Err(e) if is_port_conflict(&e) => Ok((
            bind_fallback(primary, preferred, range, None)
                .await?
                .expect("no current listener"),
            true,
        )),
        Err(source) => Err(ProxyServerError::Bind {
            addr: primary,
            source,
        }),
    }
}
async fn bind_fallback(
    primary: SocketAddr,
    preferred: u16,
    range: std::ops::RangeInclusive<u16>,
    current: Option<SocketAddr>,
) -> Result<Option<TcpListener>, ProxyServerError> {
    if current.is_some_and(|addr| addr != primary && addr.port() != 0) {
        return Ok(None);
    }
    let mut last = io::Error::new(io::ErrorKind::AddrInUse, "All fallback ports are busy");
    for port in std::iter::once(preferred).chain(range.filter(|p| *p != preferred)) {
        if port == 0 || port == primary.port() {
            continue;
        }
        let addr = SocketAddr::new(primary.ip(), port);
        match TcpListener::bind(addr).await {
            Ok(listener) => return Ok(Some(listener)),
            Err(e) if is_port_conflict(&e) => last = e,
            Err(source) => return Err(ProxyServerError::Bind { addr, source }),
        }
    }
    Err(ProxyServerError::Bind {
        addr: SocketAddr::new(primary.ip(), preferred),
        source: last,
    })
}

struct ListenerControl {
    network: Arc<RwLock<ProxyNetwork>>,
    commands: mpsc::Sender<ListenerCommand>,
    priv_client: PrivClient,
}

#[cfg(test)]
mod cutover_bind_tests {
    use super::*;
    #[tokio::test]
    async fn immediate_handoff_waits_only_for_draining_accept_fds() {
        let old = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = old.local_addr().unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            drop(old);
        });
        let new = bind_cutover(addr, tokio::time::Instant::now() + Duration::from_secs(1))
            .await
            .unwrap();
        assert_eq!(new.local_addr().unwrap(), addr);
        release.await.unwrap();
        assert!(
            bind_cutover(
                addr,
                tokio::time::Instant::now() + Duration::from_millis(20)
            )
            .await
            .is_err()
        );
    }
}
