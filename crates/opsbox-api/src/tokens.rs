//! `GET /settings`, `POST /tokens`, `DELETE /tokens/{id}` (docs/BACKEND_SPEC.md 6.4).
//!
//! API tokens let a script (a CI job, an uptime check) call the API with
//! `Authorization: Bearer ops_...`. A token is shown once when it is created and stored only
//! as a hash. Its scope decides what it may do; none can open terminals or manage people,
//! tokens or SSH keys.

use axum::Json;
use axum::extract::{Extension, Path, State};
use opsbox_proto::ErrorCode;
use opsbox_store::{ApiToken, TokenScope};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::session::{Admin, Client, deny_token};
use crate::system::patient;
use crate::{AppState, HttpError, RequestId};

/// How the Settings page words a scope.
fn can_text(scope: TokenScope) -> &'static str {
    match scope {
        TokenScope::Read => "read-only metrics",
        TokenScope::Deploy => "metrics and logs (deploy in Phase 3)",
        TokenScope::Full => "full access (not people, terminals or keys)",
    }
}

fn token_json(t: &ApiToken, now: i64) -> Value {
    json!({
        "id": t.id, "name": t.name, "scope": t.scope.as_str(), "can": can_text(t.scope),
        "last": t.last_used_at.map_or_else(|| "never".to_owned(), |l| ago(now - l)),
    })
}

/// `3 min ago`, `2 h ago`, `5 d ago`.
fn ago(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86400),
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// `GET /settings`: the machine's host name, time zone and swap, and the API tokens.
pub(crate) async fn get_settings(
    State(state): State<AppState>,
    Admin(_): Admin,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let pc = patient(&state, 30);
    let (host, settings, panel) = tokio::join!(
        pc.host_info(&rid, None),
        pc.settings_get(&rid),
        crate::panel::snapshot(&state),
    );
    let host = host?;
    let s = settings?;
    let tokens = state.auth.list_tokens()?;
    let t = now();
    Ok(Json(json!({
        "hostname": host.hostname, "tz": s.timezone,
        // Without the system's list, offer the current zone at least.
        "timezones": if s.timezones.is_empty() { vec![s.timezone.clone()] } else { s.timezones },
        "swapMb": s.swap_mb,
        "tokens": tokens.iter().map(|k| token_json(k, t)).collect::<Vec<_>>(),
        "opsboxUpdate": state.updater.snapshot(),
        "panel": panel,
    })))
}

#[derive(Deserialize)]
pub(crate) struct CreateBody {
    name: String,
    scope: String,
}

/// `POST /tokens`: the answer holds the secret, which is never shown again.
pub(crate) async fn create(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    ApiJson(body): ApiJson<CreateBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let scope = TokenScope::parse(&body.scope)
        .ok_or_else(|| HttpError::new(ErrorCode::Validation, "Choose read, deploy or full."))?;
    let (token, secret) =
        state
            .auth
            .create_token(&who.user, &body.name, scope, Some(&client.ip))?;
    Ok(Json(
        json!({ "token": token_json(&token, now()), "secret": secret }),
    ))
}

/// `DELETE /tokens/{id}`
pub(crate) async fn revoke(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    state.auth.revoke_token(&who.user, &id, Some(&client.ip))?;
    Ok(Json(json!({ "ok": true })))
}
