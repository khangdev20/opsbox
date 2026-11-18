//! `GET /api/v1/system/uninstall/plan` and `POST /api/v1/system/uninstall`.

use axum::Json;
use axum::extract::{Extension, Query, State};
use opsbox_store::{ActorKind, AuditResult};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::panel::active_panel_domain;
use crate::session::{Admin, Client, deny_token};
use crate::system::invalid;
use crate::{AppState, HttpError, RequestId};

#[derive(Debug, Deserialize)]
pub(crate) struct PlanQuery {
    #[serde(default)]
    pub purge: bool,
}

fn collect_domains(state: &AppState) -> Vec<String> {
    let mut domains = Vec::new();
    if let Some(p) = active_panel_domain(state) {
        domains.push(p);
    }
    if let Ok(routes) = state.store.list_active_service_routes() {
        for r in routes {
            if !domains.iter().any(|d| d == &r.domain) {
                domains.push(r.domain);
            }
        }
    }
    domains.sort();
    domains.dedup();
    domains
}

fn plan_json(plan: &opsbox_proto::UninstallPlan) -> Value {
    serde_json::to_value(plan).unwrap_or_else(|_| json!({}))
}

/// `GET /api/v1/system/uninstall/plan`
pub(crate) async fn plan(
    State(state): State<AppState>,
    Admin(who): Admin,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(q): Query<PlanQuery>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let domains = collect_domains(&state);
    let plan = state
        .priv_client
        .uninstall_plan(&rid, q.purge, domains)
        .await?;
    Ok(Json(plan_json(&plan)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UninstallBody {
    password: String,
    /// Must be the exact word `uninstall`.
    confirm: String,
    #[serde(default)]
    purge: bool,
}

/// `POST /api/v1/system/uninstall`: re-auth, typed confirm, schedule detached uninstall.
pub(crate) async fn exec(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<UninstallBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let audit = |result: AuditResult, detail: Value| {
        state.auth.audit(
            &who.user.name,
            ActorKind::User,
            Some(&client.ip),
            "system.uninstall",
            None,
            result,
            detail,
        );
    };
    let ok = state
        .auth
        .verify_user_password(&who.user.id, &body.password)
        .await
        .map_err(|_| HttpError::internal())?;
    if !ok {
        audit(AuditResult::Denied, json!({ "reason": "wrong_password" }));
        return Err(HttpError::new(
            opsbox_proto::ErrorCode::Forbidden,
            "Incorrect account password.",
        ));
    }
    if body.confirm.trim() != "uninstall" {
        return Err(invalid("confirm", "Type uninstall to confirm."));
    }

    let domains = collect_domains(&state);
    let plan = state
        .priv_client
        .uninstall_plan(&rid, body.purge, domains.clone())
        .await?;

    if let Err(e) = state
        .priv_client
        .uninstall(&rid, &who.user.name, body.purge)
        .await
    {
        audit(
            AuditResult::Failed,
            json!({ "purge": body.purge, "domains": domains }),
        );
        return Err(HttpError::from(e));
    }

    audit(
        AuditResult::Ok,
        json!({
            "purge": body.purge,
            "domains": domains,
            "containers": plan.containers,
            "adopted": plan.adopted.as_ref().map(|a| &a.holder),
        }),
    );

    Ok(Json(json!({
        "ok": true,
        "message": "opsbox is being removed. This page will stop responding in a moment.",
        "plan": plan_json(&plan),
    })))
}
