//! `/healthz` (is the process alive) and `/readyz` (are its dependencies usable).

use std::time::Duration;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    proxy: Option<&'static str>,
}

fn label(ok: bool) -> &'static str {
    if ok { "ok" } else { "failing" }
}

/// 200 when the database, privileged helper, and (when a panel domain is active)
/// `opsbox-proxy` answer, else 503.
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
    let proxy_required =
        state.proxy.is_some() && crate::panel::active_panel_domain(&state).is_some();
    let proxy_check = async {
        let client = state.proxy.as_ref()?;
        match client
            .clone()
            .with_timeout(Duration::from_secs(2))
            .ping(&request_id)
            .await
        {
            Ok(_) => Some(true),
            Err(e) => {
                if proxy_required {
                    warn!(error = %e, "readiness: reverse proxy check failed");
                }
                Some(false)
            }
        }
    };

    let (db_ok, helper_ok, proxy_opt) = tokio::join!(db, helper, proxy_check);
    let proxy_ok = !proxy_required || proxy_opt == Some(true);
    let proxy_label = match (proxy_opt, proxy_required) {
        (Some(true), _) => Some("ok"),
        (Some(false), true) => Some("failing"),
        _ => None,
    };

    let ready = db_ok && helper_ok && proxy_ok;
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
                proxy: proxy_label,
            },
        }),
    )
}
