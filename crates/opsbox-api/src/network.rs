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
            let moves = match state.priv_client.call(&rid, None, PrivOp::WebMoves).await? {
                PrivReply::WebMoves(rows) => rows,
                _ => return Err(HttpError::internal()),
            };
            let adopted = if let Some(proxy) = &state.proxy {
                proxy.status(&rid).await.ok().map(|s| s.adopted)
            } else {
                None
            };
            Ok(Json(
                json!({"mode":proxy_mode(&state).await,"network":network,"undo":undo,"moves":moves,"adopted":adopted}),
            ))
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
            actor: who.user.actor_label(),
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DiscoverBody {
    target: PortTarget,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MoveBody {
    target: PortTarget,
    #[serde(default)]
    domains: Vec<String>,
    everything_else: bool,
    confirmation: String,
}
pub(crate) async fn discover(
    State(state): State<AppState>,
    Admin(who): Admin,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<DiscoverBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    match state
        .priv_client
        .clone()
        .with_timeout(std::time::Duration::from_secs(120))
        .call(
            &rid,
            Some(&who.user.name),
            PrivOp::WebDiscover {
                target: body.target,
            },
        )
        .await?
    {
        PrivReply::WebPlan(plan) => Ok(Json(json!({"plan":plan}))),
        _ => Err(HttpError::internal()),
    }
}
async fn web_task(
    state: AppState,
    who: opsbox_auth::Principal,
    client: Client,
    rid: String,
    op: PrivOp,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    deny_token(&who)?;
    let worker = state.clone();
    let task = state.tasks.start("web_move", async move {
        let call = worker.priv_client.clone().with_timeout(std::time::Duration::from_secs(600));
        let work = call.call(&rid,Some(&who.user.name),op.clone());
        tokio::pin!(work);
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(400));
        let result = loop {
            tokio::select! {
                result = &mut work => break result,
                _ = tick.tick() => {
                    if let Ok(PrivReply::WebMoves(rows)) = worker.priv_client.call(&rid,None,PrivOp::WebMoves).await {
                        let row = rows.iter().filter(|r|r.active).max_by_key(|r| &r.id).or_else(||rows.last());
                        if let Some(row)=row {worker.tasks.progress("web_move",row.steps.clone());}
                    }
                }
            }
        };
        let receipts = match worker.priv_client.call(&rid, None, PrivOp::WebMoves).await {
            Ok(PrivReply::WebMoves(rows)) => rows, _ => vec![],
        };
        if let Some(receipt)=receipts.iter().filter(|r|r.active).max_by_key(|r|&r.id).or_else(||receipts.last()) {worker.tasks.progress("web_move",receipt.steps.clone());}
        let detail=match &result {Ok(PrivReply::WebMove(receipt))=>json!({"receipt":receipt}), _=>json!({"request":op,"receipts":receipts})};
        worker.store.append_audit(opsbox_store::NewAudit {
            actor:who.user.actor_label(),actor_kind:ActorKind::User,ip:Some(client.ip),action:format!("network.{}",op.name()),target:Some("proxy".into()),
            result:if result.is_ok() {AuditResult::Ok} else {AuditResult::Failed}, detail,
        }).map_err(|_|HttpError::internal())?;
        match result? {
            PrivReply::WebMove(receipt)=>Ok(crate::tasks::Outcome {log:receipt.steps.clone(),result:json!({"receipt":receipt})}),
            PrivReply::Done=>Ok(crate::tasks::Outcome {log:vec!["opsbox now handles TLS for this domain.".into()],result:json!({"ok":true})}),
            _=>Err(HttpError::internal()),
        }
    })?;
    Ok((axum::http::StatusCode::ACCEPTED, Json(json!({"task":task}))))
}
pub(crate) async fn move_web(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<MoveBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    web_task(
        state,
        who,
        client,
        rid,
        PrivOp::WebMove {
            target: body.target,
            domains: body.domains,
            everything_else: body.everything_else,
            confirmation: body.confirmation,
        },
    )
    .await
}
pub(crate) async fn undo_move(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<UndoBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    web_task(
        state,
        who,
        client,
        rid,
        PrivOp::WebUndo {
            id: body.id,
            confirm: body.confirm,
        },
    )
    .await
}
pub(crate) async fn adopted_tls(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    axum::extract::Path(domain): axum::extract::Path<String>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    web_task(state, who, client, rid, PrivOp::WebTls { domain }).await
}

/// Query the helper's current file setting, including changes made without restarting serve.
pub(crate) async fn proxy_mode(state: &AppState) -> opsbox_proto::ProxyMode {
    match state
        .priv_client
        .call("proxy-mode", None, PrivOp::ProxyModeGet)
        .await
    {
        Ok(PrivReply::ProxyMode(mode)) => mode,
        _ => state.proxy_mode,
    }
}

pub(crate) async fn require_builtin(state: &AppState) -> Result<(), HttpError> {
    if proxy_mode(state).await == opsbox_proto::ProxyMode::Builtin {
        Ok(())
    } else {
        Err(HttpError::new(
            ErrorCode::Conflict,
            "opsbox proxy is disabled. Configure routing and certificates in your external proxy, or switch to builtin in Network settings.",
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModeBody {
    mode: opsbox_proto::ProxyMode,
    #[serde(default)]
    confirm: bool,
}
pub(crate) async fn set_mode(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<ModeBody>,
) -> Result<Json<Value>, HttpError> {
    let response = change(
        state.clone(),
        who,
        client,
        rid,
        PrivOp::ProxyModeSet {
            mode: body.mode,
            confirm: body.confirm,
        },
    )
    .await?;
    if body.mode == opsbox_proto::ProxyMode::Builtin {
        for attempt in 0..10 {
            if crate::panel::sync_proxy_routes(&state).await.is_ok() {
                break;
            }
            if attempt < 9 {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            } else {
                return Err(HttpError::new(
                    ErrorCode::Unavailable,
                    "Proxy mode was saved, but routes could not be loaded. Refresh Network settings and check the proxy service logs.",
                ));
            }
        }
    }
    Ok(response)
}
