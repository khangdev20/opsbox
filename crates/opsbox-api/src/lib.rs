//! The HTTP server: router, request ids, the error format, health endpoints and the
//! embedded web UI.
//!
//! Phase 0 has no authentication, so the server is meant to listen on loopback only
//! and exposes nothing but non-secret host facts.

mod error;
mod health;
mod host;
mod request_id;
mod ui;

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use opsbox_priv::PrivClient;
use opsbox_store::Store;
use tokio::net::TcpListener;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

pub use error::HttpError;
pub use request_id::RequestId;

/// Request bodies above this are rejected (docs/BACKEND_SPEC.md section 6.1). The
/// streamed upload endpoint will get its own, larger limit in a later phase.
const BODY_LIMIT: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Shared by every handler. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub priv_client: PrivClient,
    pub version: &'static str,
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
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}
