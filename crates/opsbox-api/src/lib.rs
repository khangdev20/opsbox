//! The HTTP server: router, request ids, the error format, health endpoints and the
//! embedded web UI.
//!
//! Phase 0 has no authentication, so the server is meant to listen on loopback only
//! and exposes nothing but non-secret host facts.

mod auth;
mod error;
mod health;
mod host;
mod json;
mod request_id;
pub mod session;
mod setup;
mod ui;

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::{get, post};
use opsbox_auth::{Auth, AuthConfig};
use opsbox_priv::PrivClient;
use opsbox_store::Store;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

pub use error::HttpError;
pub use request_id::RequestId;
pub use session::SecureCookies;

/// Request bodies above this are rejected (docs/BACKEND_SPEC.md section 6.1). The
/// streamed upload endpoint will get its own, larger limit in a later phase.
const BODY_LIMIT: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared by every handler. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub priv_client: PrivClient,
    pub auth: Auth,
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
        Self {
            store,
            priv_client,
            auth,
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

/// Build the full application: API routes, the embedded UI as the fallback, and the
/// middleware stack.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/api/v1/host", get(host::get_host))
        .route("/api/v1/session", get(auth::session))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/sessions/revoke-all", post(auth::revoke_all))
        .route("/api/v1/auth/totp/enroll", post(auth::totp_enroll))
        .route("/api/v1/auth/totp/confirm", post(auth::totp_confirm))
        .route("/api/v1/setup/claim", post(setup::claim))
        .route("/api/v1/setup/admin", post(setup::admin))
        .route("/api/v1/setup/totp/confirm", post(setup::confirm))
        .fallback(ui::fallback)
        .method_not_allowed_fallback(error::method_not_allowed)
        .with_state(state)
        // Layers added last run first: request id outermost so its span covers all.
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        // Fonts are already compressed; the UI's text assets compress well.
        .layer(
            CompressionLayer::new()
                .compress_when(DefaultPredicate::new().and(NotForContentType::new("font/"))),
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
