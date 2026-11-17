//! `GET /api/v1/host`.

use axum::Json;
use axum::extract::{Extension, State};
use opsbox_proto::HostView;

use crate::{AppState, HttpError, RequestId};

/// Facts about the machine, read by `priv` (hostname, OS, kernel, uptime, CPUs, RAM).
pub(crate) async fn get_host(
    State(state): State<AppState>,
    Extension(RequestId(request_id)): Extension<RequestId>,
) -> Result<Json<HostView>, HttpError> {
    let info = state.priv_client.host_info(&request_id, None).await?;
    Ok(Json(HostView::from_info(info, state.version)))
}
