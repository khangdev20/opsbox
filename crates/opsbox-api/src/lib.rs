//! The HTTP server: router, request ids, the error format, health endpoints and the
//! embedded web UI.
//!
//! Phase 1 authenticates machine APIs and delegates system access to `priv`.
//! Listen on loopback or use a trusted HTTPS proxy; native TLS arrives later.

mod access;
mod allowlist;
mod auth;
mod cleanup;
pub mod deploy;
mod docker;
mod domains;
mod error;
mod files;
mod github;
mod hardening;
mod health;
mod host;
mod json;
mod limits;
mod metrics;
pub mod panel;
mod recovery;
mod request_id;
mod security;
pub mod session;
mod setup;
mod system;
mod tasks;
mod terminal;
pub mod tls;
mod tokens;
mod ui;
mod update;
mod users;

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use opsbox_auth::{Auth, AuthConfig};
use opsbox_metrics::Metrics;
use opsbox_priv::PrivClient;
use opsbox_store::Store;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

pub use deploy::{DeployHub, DeploySettings};
pub use error::HttpError;
pub use files::FilesSettings;
pub use panel::sync_proxy_routes;
pub use request_id::RequestId;
pub use session::SecureCookies;
pub use terminal::{DEFAULT_IDLE as DEFAULT_TERMINAL_IDLE, TerminalSettings};

/// Request bodies above this are rejected (docs/BACKEND_SPEC.md section 6.1). The
/// streamed upload and editor endpoints have separate limits below.
const BODY_LIMIT: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The editor sends whole files: 5 MB of text can be more than 10 MB once JSON-escaped.
const EDITOR_BODY_LIMIT: usize = 12 * 1024 * 1024;
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2 * 3600);

pub type PublicIpCache =
    Arc<tokio::sync::Mutex<Option<(std::time::Instant, Vec<std::net::IpAddr>)>>>;

/// Shared by every handler. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub priv_client: PrivClient,
    pub auth: Auth,
    pub metrics: Metrics,
    pub tasks: tasks::Tasks,
    pub files: FilesSettings,
    pub terminal: TerminalSettings,
    pub(crate) limits: limits::RateLimits,
    pub(crate) stream_slots: Arc<tokio::sync::Semaphore>,
    /// The panel's own address, for the Security checks.
    pub listen: std::net::SocketAddr,
    pub allow: allowlist::AllowList,
    pub secure_cookies: SecureCookies,
    pub tls: bool,
    pub tls_cert_sha256: Option<String>,
    pub proxy: Option<opsbox_proxy::ProxyClient>,
    pub public_ips: Vec<std::net::IpAddr>,
    pub dns_nameservers: Option<Vec<std::net::SocketAddr>>,
    pub(crate) public_ip_cache: PublicIpCache,
    pub version: &'static str,
    pub updater: update::Updater,
    pub github: github::GithubService,
    pub deploy: DeploySettings,
    pub deploy_hub: DeployHub,
}

impl AppState {
    pub fn new(
        store: Arc<Store>,
        priv_client: PrivClient,
        version: &'static str,
        secure_cookies: SecureCookies,
        auth: AuthConfig,
    ) -> Self {
        let auth = Auth::new(store.clone(), priv_client.clone(), auth);
        let metrics = Metrics::spawn(priv_client.clone(), store.clone());
        let allow = allowlist::AllowList::default();
        allow.set(security::stored_allow_list(&store));
        let files = FilesSettings::default();
        let updater = update::Updater::spawn(
            store.clone(),
            priv_client.clone(),
            files.xfer_dir.clone(),
            version,
        );
        Self {
            store,
            priv_client,
            auth,
            metrics,
            tasks: tasks::Tasks::default(),
            files,
            terminal: TerminalSettings::default(),
            limits: limits::RateLimits::default(),
            stream_slots: Arc::new(tokio::sync::Semaphore::new(64)),
            listen: std::net::SocketAddr::from(([127, 0, 0, 1], 8443)),
            allow,
            secure_cookies,
            tls: false,
            tls_cert_sha256: None,
            proxy: None,
            public_ips: Vec::new(),
            dns_nameservers: None,
            public_ip_cache: Arc::new(tokio::sync::Mutex::new(None)),
            version,
            updater,
            github: github::GithubService::default(),
            deploy: DeploySettings::default(),
            deploy_hub: DeployHub::default(),
        }
    }
}

/// Check that the embedded web UI is complete. Used by `opsbox selftest`; returns the
/// number of embedded files.
pub fn check_embedded_ui() -> Result<usize, String> {
    ui::check_embedded()
}

impl AppState {
    #[must_use]
    pub fn with_deploy(mut self, deploy: DeploySettings) -> Self {
        self.deploy = deploy;
        self
    }
    /// Mark whether `serve` is terminating native TLS (forces `Secure` cookies in Auto mode).
    #[must_use]
    pub fn with_tls(mut self, tls: bool) -> Self {
        self.tls = tls;
        if tls && self.secure_cookies == SecureCookies::Auto {
            self.secure_cookies = SecureCookies::Always;
        }
        self
    }
    /// Use these file-transfer settings instead of the defaults.
    #[must_use]
    pub fn with_files(mut self, files: FilesSettings) -> Self {
        self.files = files;
        self
    }

    /// The allowed-address list as text (empty when there is none).
    pub fn allow_list_text(&self) -> String {
        allowlist::render(&self.allow.get())
    }

    /// The address the panel listens on (for the Security checks).
    #[must_use]
    pub fn with_listen(mut self, listen: std::net::SocketAddr) -> Self {
        self.listen = listen;
        self
    }

    /// Close a terminal nobody has typed in for this long.
    #[must_use]
    pub fn with_terminal_idle(mut self, idle: Duration) -> Self {
        self.terminal = TerminalSettings::new(idle);
        self
    }

    /// Connect `serve` to `opsbox-proxy` for panel domain routing and TLS certificates.
    #[must_use]
    pub fn with_proxy(
        mut self,
        client: opsbox_proxy::ProxyClient,
        tls_cert_sha256: Option<String>,
        public_ips: Vec<std::net::IpAddr>,
    ) -> Self {
        self.proxy = Some(client);
        self.tls_cert_sha256 = tls_cert_sha256;
        self.public_ips = public_ips;
        self
    }

    /// Override GitHub API/web base URLs (used in tests).
    #[must_use]
    pub fn with_github(mut self, config: opsbox_deploy::GitHubConfig) -> Self {
        self.github = github::GithubService::new(config);
        self
    }

    /// Override DNS resolver nameservers (used in hermetic tests).
    #[must_use]
    pub fn with_dns_nameservers(mut self, servers: Vec<std::net::SocketAddr>) -> Self {
        self.dns_nameservers = Some(servers);
        self
    }
}

/// Build the full application: API routes, the embedded UI as the fallback, and the
/// middleware stack.
pub fn router(state: AppState) -> Router {
    let allow_guard_state = state.allow.clone();
    let limits = state.limits.clone();
    let max_upload = usize::try_from(state.files.max_upload).unwrap_or(usize::MAX);

    // Ordinary API calls: small bodies, short time.
    let ordinary = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/api/v1/host", get(host::get_host))
        .route("/api/v1/tasks/{id}", get(system::get_task))
        .route("/api/v1/system/updates", get(system::get_updates))
        .route(
            "/api/v1/system/updates/install",
            post(system::install_updates),
        )
        .route("/api/v1/system/updates/auto", put(system::set_auto_updates))
        .route("/api/v1/system/reboot", post(system::reboot))
        .route("/api/v1/system/reboot/cancel", post(system::reboot_cancel))
        .route("/api/v1/system/units", get(system::get_units))
        .route(
            "/api/v1/system/units/{name}/{action}",
            post(system::unit_action),
        )
        .route("/api/v1/system/journal", get(system::get_journal))
        .route("/api/v1/stream/journal", get(system::stream_journal))
        .route("/api/v1/jobs", get(system::get_jobs).post(system::add_job))
        .route("/api/v1/jobs/{id}/run", post(system::run_job))
        .route("/api/v1/jobs/{id}", delete(system::remove_job))
        .route("/api/v1/firewall", get(system::get_firewall))
        .route("/api/v1/firewall/rules", post(system::add_rule))
        .route("/api/v1/firewall/rules/{id}", delete(system::remove_rule))
        .route("/api/v1/firewall/confirm", post(system::confirm_firewall))
        .route("/api/v1/ssh/emergency", post(system::ssh_emergency))
        .route("/api/v1/system/swap", put(system::set_swap))
        .route("/api/v1/system/hostname", put(system::set_hostname))
        .route("/api/v1/system/timezone", put(system::set_timezone))
        .route("/api/v1/metrics", get(metrics::get_metrics))
        .route("/api/v1/attention", get(metrics::get_attention))
        .route("/api/v1/stream/metrics", get(metrics::stream))
        .route("/api/v1/settings", get(tokens::get_settings))
        .route("/api/v1/panel", get(panel::status))
        .route("/api/v1/panel/dns-check", post(panel::check_dns))
        .route("/api/v1/panel/certificate", post(panel::issue_cert))
        .route("/api/v1/panel/domain", delete(panel::remove_domain))
        .route("/api/v1/panel/local-only", post(panel::set_local_only))
        .route("/api/v1/updates/self", get(update::get_status))
        .route("/api/v1/updates/check", post(update::check))
        .route("/api/v1/updates/apply", post(update::apply))
        .route("/api/v1/updates/rollback", post(update::rollback))
        .route("/api/v1/updates/auto", put(update::set_auto))
        .route("/api/v1/recovery", get(recovery::get))
        .route("/api/v1/tokens", post(tokens::create))
        .route("/api/v1/tokens/{id}", delete(tokens::revoke))
        .route("/api/v1/security", get(security::get))
        .route("/api/v1/security/allow-ips", put(security::set_allow_ips))
        .route("/api/v1/security/checks/{id}/fix", post(security::fix))
        .route("/api/v1/users/invites", post(users::invite))
        .route("/api/v1/users/{name}", delete(users::remove))
        .route("/api/v1/invites/claim", post(users::claim))
        .route("/api/v1/invites/{token}", get(users::info))
        .route("/api/v1/access", get(access::get))
        .route("/api/v1/access/keys", post(access::add_key))
        .route("/api/v1/access/keys/remove", post(access::remove_key))
        .route("/api/v1/access/users/{name}/lock", post(access::lock_user))
        .route("/api/v1/cleanup/scan", get(cleanup::scan))
        .route("/api/v1/cleanup/volumes", get(cleanup::volume_sizes))
        .route("/api/v1/cleanup/run", post(cleanup::run))
        .route("/api/v1/cleanup/auto", put(cleanup::set_auto))
        .route("/api/v1/docker/status", get(docker::status))
        .route("/api/v1/docker/install", post(docker::install))
        .route("/api/v1/docker/containers", get(docker::list_containers))
        .route(
            "/api/v1/docker/containers/{id}",
            get(docker::inspect_container),
        )
        .route(
            "/api/v1/docker/containers/{id}/{action}",
            post(docker::container_action),
        )
        .route(
            "/api/v1/docker/containers/{id}/logs",
            get(docker::container_logs),
        )
        .route(
            "/api/v1/stream/containers/{id}/logs",
            get(docker::stream_logs),
        )
        .route("/api/v1/docker/images", get(docker::list_images))
        .route("/api/v1/docker/volumes", get(docker::list_volumes))
        .route("/api/v1/docker/networks", get(docker::list_networks))
        .route(
            "/api/v1/github",
            get(github::get_status).delete(github::disconnect),
        )
        .route("/api/v1/github/manifest", post(github::start_manifest))
        .route("/api/v1/github/callback", get(github::manifest_callback))
        .route("/api/v1/github/setup", get(github::setup_callback))
        .route("/api/v1/github/complete", post(github::complete_manifest))
        .route(
            "/api/v1/github/installation",
            post(github::sync_installation),
        )
        .route("/api/v1/github/webhook/sync", post(github::sync_webhook))
        .route("/api/v1/github/webhook", post(github::webhook))
        .route("/api/v1/github/repos", get(github::list_repos))
        .route(
            "/api/v1/github/public-repo",
            post(github::inspect_public_repo),
        )
        .route(
            "/api/v1/github/deploy-keys",
            get(github::list_deploy_keys).post(github::create_deploy_key),
        )
        .route(
            "/api/v1/github/deploy-keys/{id}",
            delete(github::delete_deploy_key),
        )
        .route("/api/v1/projects/{id}", get(deploy::get_project))
        .route("/api/v1/services", post(deploy::create_service))
        .route(
            "/api/v1/services/{id}",
            get(deploy::get_service)
                .patch(deploy::update_service)
                .delete(deploy::delete_service),
        )
        .route("/api/v1/services/{id}/deploys", post(deploy::redeploy))
        .route(
            "/api/v1/services/{id}/deploys/{deploy_id}/rollback",
            post(deploy::rollback),
        )
        .route(
            "/api/v1/services/{id}/vars/{key}",
            put(deploy::put_var).delete(deploy::delete_var),
        )
        .route(
            "/api/v1/services/{id}/domains",
            get(domains::get_domains).post(domains::add_domain),
        )
        .route(
            "/api/v1/domains/{name}",
            patch(domains::update_domain).delete(domains::remove_domain),
        )
        .route("/api/v1/domains/{name}/check", post(domains::check_dns))
        .route(
            "/api/v1/domains/{name}/certificate",
            post(domains::issue_cert),
        )
        .route(
            "/api/v1/domains/{name}/primary",
            post(domains::make_primary),
        )
        .route("/api/v1/deploys/{id}/log", get(deploy::deploy_log))
        .route(
            "/api/v1/stream/deploys/{id}",
            get(deploy::stream_deploy_log),
        )
        .route("/api/v1/terminal/users", get(terminal::users))
        .route("/api/v1/stream/terminal", get(terminal::stream))
        .route("/api/v1/session", get(auth::session))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/sessions/revoke-all", post(auth::revoke_all))
        .route("/api/v1/auth/totp/enroll", post(auth::totp_enroll))
        .route("/api/v1/auth/totp/confirm", post(auth::totp_confirm))
        .route("/api/v1/setup/status", get(setup::status))
        .route("/api/v1/setup/claim", post(setup::claim))
        .route("/api/v1/setup/admin", post(setup::admin))
        .route("/api/v1/setup/totp/confirm", post(setup::confirm))
        .route("/api/v1/files", get(files::list))
        .route("/api/v1/files/chmod", post(files::chmod))
        .route("/api/v1/files/mkdir", post(files::mkdir))
        .route("/api/v1/files/delete", post(files::delete))
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ));

    // The editor sends a whole file (up to 5 MB, more once JSON-escaped).
    let editor = Router::new()
        .route("/api/v1/files/content", get(files::read).put(files::write))
        // Two limits: axum's `Json` extractor has its own (2 MiB) and must be raised as well.
        .layer(axum::extract::DefaultBodyLimit::max(EDITOR_BODY_LIMIT))
        .layer(RequestBodyLimitLayer::new(EDITOR_BODY_LIMIT))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(60),
        ));

    // Uploads and downloads are streamed and can take as long as the connection needs.
    let transfers = Router::new()
        .route("/api/v1/files/upload", post(files::upload))
        .route("/api/v1/files/download", get(files::download))
        .layer(RequestBodyLimitLayer::new(max_upload))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            TRANSFER_TIMEOUT,
        ));

    let setup_guard_state = state.auth.clone();
    Router::new()
        .merge(ordinary)
        .merge(editor)
        .merge(transfers)
        .fallback(ui::fallback)
        .method_not_allowed_fallback(error::method_not_allowed)
        .with_state(state)
        // Layers added last run first: request id outermost so its span covers all.
        // Fonts and downloads are already compressed (or must stay byte-exact); the UI's
        // text assets compress well.
        .layer(
            CompressionLayer::new().compress_when(
                DefaultPredicate::new()
                    .and(NotForContentType::new("font/"))
                    .and(NotForContentType::new("application/octet-stream")),
            ),
        )
        .layer(axum::middleware::from_fn_with_state(
            setup_guard_state,
            setup_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            allow_guard_state,
            allow_guard,
        ))
        .layer(axum::middleware::from_fn_with_state(
            limits,
            limits::middleware,
        ))
        .layer(axum::middleware::from_fn(hardening::middleware))
        .layer(axum::middleware::from_fn(request_id::middleware))
}

async fn setup_guard(
    axum::extract::State(auth): axum::extract::State<Auth>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = req.uri().path();
    let allowed_unclaimed = path == "/healthz"
        || path == "/readyz"
        || path.starts_with("/api/v1/setup/")
        || !path.starts_with("/api/");
    if !allowed_unclaimed && auth.setup_required().unwrap_or(false) {
        return HttpError::new(
            opsbox_proto::ErrorCode::Forbidden,
            "Complete first-run setup first.",
        )
        .into_response();
    }
    next.run(req).await
}

/// Refuse requests from addresses outside the allowed list (when there is one). The
/// health endpoints and HMAC-verified GitHub webhook stay reachable.
async fn allow_guard(
    axum::extract::State(allow): axum::extract::State<allowlist::AllowList>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = req.uri().path();
    if path == "/healthz"
        || path == "/readyz"
        || path == "/api/v1/github/webhook"
        || allow.get().is_empty()
    {
        return next.run(req).await;
    }
    let (parts, body) = req.into_parts();
    // Fail closed: with a list set, an address that cannot be read is not allowed.
    let permitted = session::client_ip(&parts)
        .parse::<std::net::IpAddr>()
        .is_ok_and(|ip| allow.permits(ip));
    let req = axum::extract::Request::from_parts(parts, body);
    if permitted {
        next.run(req).await
    } else {
        HttpError::new(
            opsbox_proto::ErrorCode::Forbidden,
            "This address is not allowed to use the panel.",
        )
        .into_response()
    }
}

/// Serve `router(state)` on `listener` until `shutdown` completes.
pub async fn serve(
    listener: TcpListener,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    // Expired sessions and old sign-in failures are dropped once an hour.
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
    let app = router(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await;
    janitor.abort();
    result
}
