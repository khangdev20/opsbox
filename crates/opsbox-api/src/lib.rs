//! The HTTP server: router, request ids, the error format, health endpoints and the
//! embedded web UI.
//!
//! Phase 0 has no authentication, so the server is meant to listen on loopback only
//! and exposes nothing but non-secret host facts.

mod auth;
mod cleanup;
mod error;
mod files;
mod health;
mod host;
mod json;
mod metrics;
mod request_id;
pub mod session;
mod setup;
mod system;
mod tasks;
mod terminal;
mod ui;

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use opsbox_auth::{Auth, AuthConfig};
use opsbox_metrics::Metrics;
use opsbox_priv::PrivClient;
use opsbox_store::Store;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

pub use error::HttpError;
pub use files::FilesSettings;
pub use request_id::RequestId;
pub use session::SecureCookies;
pub use terminal::{DEFAULT_IDLE as DEFAULT_TERMINAL_IDLE, TerminalSettings};

/// Request bodies above this are rejected (docs/BACKEND_SPEC.md section 6.1). The
/// streamed upload endpoint will get its own, larger limit in a later phase.
const BODY_LIMIT: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// The editor sends whole files: 5 MB of text can be more than 10 MB once JSON-escaped.
const EDITOR_BODY_LIMIT: usize = 12 * 1024 * 1024;
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(2 * 3600);

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
    pub secure_cookies: SecureCookies,
    pub version: &'static str,
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
        Self {
            store,
            priv_client,
            auth,
            metrics,
            tasks: tasks::Tasks::default(),
            files: FilesSettings::default(),
            terminal: TerminalSettings::default(),
            secure_cookies,
            version,
        }
    }
}

/// Check that the embedded web UI is complete. Used by `opsbox selftest`; returns the
/// number of embedded files.
pub fn check_embedded_ui() -> Result<usize, String> {
    ui::check_embedded()
}

impl AppState {
    /// Use these file-transfer settings instead of the defaults.
    #[must_use]
    pub fn with_files(mut self, files: FilesSettings) -> Self {
        self.files = files;
        self
    }

    /// Close a terminal nobody has typed in for this long.
    #[must_use]
    pub fn with_terminal_idle(mut self, idle: Duration) -> Self {
        self.terminal = TerminalSettings::new(idle);
        self
    }
}

/// Build the full application: API routes, the embedded UI as the fallback, and the
/// middleware stack.
pub fn router(state: AppState) -> Router {
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
        .route("/api/v1/cleanup/scan", get(cleanup::scan))
        .route("/api/v1/cleanup/run", post(cleanup::run))
        .route("/api/v1/cleanup/auto", put(cleanup::set_auto))
        .route("/api/v1/terminal/users", get(terminal::users))
        .route("/api/v1/stream/terminal", get(terminal::stream))
        .route("/api/v1/session", get(auth::session))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/sessions/revoke-all", post(auth::revoke_all))
        .route("/api/v1/auth/totp/enroll", post(auth::totp_enroll))
        .route("/api/v1/auth/totp/confirm", post(auth::totp_confirm))
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
        .layer(axum::middleware::from_fn(request_id::middleware))
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
