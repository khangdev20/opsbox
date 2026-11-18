//! Administrator-only, interactive port-conflict resolution through typed privileged ops.
use crate::{
    AppState, HttpError, RequestId,
    json::ApiJson,
    session::{Admin, Client, deny_token},
};
use axum::{
    Json,
    extract::{Extension, State},
};
use opsbox_proto::{ErrorCode, PortTarget, PrivOp, PrivReply};
use opsbox_store::{ActorKind, AuditResult};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) async fn get_network(
    State(state): State<AppState>,
    Admin(_): Admin,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    match state
        .priv_client
        .call(&rid, None, PrivOp::ProxyNetwork)
        .await?
    {
        PrivReply::ProxyNetwork { network, undo } => {
            Ok(Json(json!({"network":network,"undo":undo})))
        }
        _ => Err(HttpError::internal()),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TakeoverBody {
    port: u16,
    target: PortTarget,
    #[serde(default)]
    confirm: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UndoBody {
    id: String,
    #[serde(default)]
    confirm: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeepBody {
    #[serde(default)]
    confirm: bool,
}
async fn change(
    state: AppState,
    who: opsbox_auth::Principal,
    client: Client,
    rid: String,
    op: PrivOp,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let action = op.name();
    let result = state
        .priv_client
        .clone()
        .with_timeout(std::time::Duration::from_secs(90))
        .call(&rid, Some(&who.user.name), op.clone())
        .await;
    let detail = match &result {
        Ok(PrivReply::PortTakeover(receipt)) => json!({"request":op, "receipt":receipt}),
        _ => json!({"request":op}),
    };
    state
        .store
        .append_audit(opsbox_store::NewAudit {
            actor: who.user.name,
            actor_kind: ActorKind::User,
            ip: Some(client.ip),
            action: format!("network.{action}"),
            target: Some("proxy".into()),
            result: if result.is_ok() {
                AuditResult::Ok
            } else {
                AuditResult::Failed
            },
            detail,
        })
        .map_err(|_| HttpError::internal())?;
    match result? {
        PrivReply::PortTakeover(receipt) => Ok(Json(json!({"ok":true,"receipt":receipt}))),
        PrivReply::Done => Ok(Json(json!({"ok":true}))),
        _ => Err(HttpError::new(
            ErrorCode::Internal,
            "Unexpected network response.",
        )),
    }
}
pub(crate) async fn takeover(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<TakeoverBody>,
) -> Result<Json<Value>, HttpError> {
    change(
        state,
        who,
        client,
        rid,
        PrivOp::ProxyTakeover {
            port: body.port,
            target: body.target,
            confirm: body.confirm,
        },
    )
    .await
}
pub(crate) async fn undo(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<UndoBody>,
) -> Result<Json<Value>, HttpError> {
    change(
        state,
        who,
        client,
        rid,
        PrivOp::ProxyUndo {
            id: body.id,
            confirm: body.confirm,
        },
    )
    .await
}
pub(crate) async fn keep(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<KeepBody>,
) -> Result<Json<Value>, HttpError> {
    change(
        state,
        who,
        client,
        rid,
        PrivOp::ProxyKeepPorts {
            confirm: body.confirm,
        },
    )
    .await
}
