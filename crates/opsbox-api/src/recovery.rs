//! Recovery / watchdog status endpoint (`GET /api/v1/recovery`) for the panel.
//!
//! Reads the watchdog's published status via `opsbox-priv` (`PrivOp::WatchdogStatus`)
//! and fetches recent `opsbox-watchdog.service` journal lines without performing
//! privileged actions directly in `serve`.

use axum::Json;
use axum::extract::{Extension, State};
use serde_json::{Value, json};

use crate::session::Admin;
use crate::system::patient;
use crate::{AppState, HttpError, RequestId};

/// `GET /api/v1/recovery` (Administrator only).
pub(crate) async fn get(
    State(state): State<AppState>,
    Admin(_): Admin,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let pc = patient(&state, 15);
    let (status_res, journal_res) = tokio::join!(
        state.priv_client.watchdog_status(&rid),
        pc.journal(&rid, Some("opsbox-watchdog.service"), 6, None, 25, None),
    );
    let st = status_res?;

    let mut log_lines: Vec<String> = match journal_res {
        Ok(entries) => entries
            .into_iter()
            .rev()
            .map(|e| format!("{} {}", e.proc, e.msg))
            .collect(),
        Err(_) => Vec::new(),
    };
    if log_lines.is_empty() {
        log_lines.clone_from(&st.recent_events);
    }

    let checks: Vec<[&str; 4]> = st
        .checks
        .iter()
        .map(|c| {
            [
                c.name.as_str(),
                c.status.as_str(),
                c.state.as_str(),
                c.note.as_str(),
            ]
        })
        .collect();

    Ok(Json(json!({
        "running": st.running,
        "failed": st.failed,
        "degraded": st.degraded,
        "updatedAt": st.updated_at,
        "currentVersion": st.current_version,
        "previousVersion": st.previous_version,
        "canRollback": st.can_rollback,
        "updateMessage": st.update_message,
        "safeModeListen": st.safe_mode_listen,
        "sshOpenUntil": st.ssh_open_until,
        "checks": checks,
        "events": st.recent_events,
        "log": log_lines,
    })))
}
