//! `/healthz` (is the process alive) and `/readyz` (are its dependencies usable).

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use serde::Serialize;
use tracing::warn;

use crate::{AppState, RequestId};

#[derive(Serialize)]
pub(crate) struct Health {
    status: &'static str,
    version: &'static str,
}

/// Cheap and dependency-free, so a watchdog can poll it often.
pub(crate) async fn healthz(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        status: "ok",
        version: state.version,
    })
}

#[derive(Serialize)]
pub(crate) struct Readiness {
    status: &'static str,
    checks: Checks,
}

#[derive(Serialize)]
struct Checks {
    database: &'static str,
    privileged_helper: &'static str,
}

fn label(ok: bool) -> &'static str {
    if ok { "ok" } else { "failing" }
}

/// 200 when the database and the privileged helper both answer, else 503.
/// (The proxy joins the checks in Phase 4.)
pub(crate) async fn readyz(
    State(state): State<AppState>,
    Extension(RequestId(request_id)): Extension<RequestId>,
) -> (StatusCode, Json<Readiness>) {
    let store = state.store.clone();
    let db = async move {
        match tokio::task::spawn_blocking(move || store.ping()).await {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                warn!(error = %e, "readiness: database check failed");
                false
            }
            Err(e) => {
                warn!(error = %e, "readiness: database check crashed");
                false
            }
        }
    };
    let helper = async {
        match state.priv_client.ping(&request_id).await {
            Ok(_) => true,
            Err(e) => {
                warn!(error = %e, "readiness: privileged helper check failed");
                false
            }
        }
    };
    let (db_ok, helper_ok) = tokio::join!(db, helper);

    let ready = db_ok && helper_ok;
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(Readiness {
            status: if ready { "ready" } else { "not_ready" },
            checks: Checks {
                database: label(db_ok),
                privileged_helper: label(helper_ok),
            },
        }),
    )
}
