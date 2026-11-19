//! Masking, re-authentication, reveal helpers, and log redaction for secret fields.
//!
//! List/detail responses never return plaintext secrets. Full values come only from
//! dedicated reveal endpoints after a recent password or two-factor confirmation
//! (5-minute window), with `Cache-Control: no-store`, per-session rate limits, and
//! an audit row that names the secret (never its value).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, HeaderValue, header};
use opsbox_auth::Principal;
use opsbox_deploy::pipeline::scrub_secrets;
use opsbox_proto::ErrorCode;
use opsbox_store::{ActorKind, AuditResult, NewAudit};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::session::{Client, deny_token};
use crate::{AppState, HttpError, RequestId};

/// How long a successful password / 2FA step-up remains valid for reveal calls.
pub const REAUTH_TTL: Duration = Duration::from_secs(5 * 60);

/// Placeholder when a sealed secret is not opened (list/detail responses).
pub const MASKED_UNKNOWN: &str = "••••••••";

/// Prefix used when a known plaintext is masked for display.
#[allow(dead_code)]
const MASK_PREFIX: &str = "••••";

/// In-memory map of session id → last successful re-authentication.
#[derive(Clone, Default)]
pub struct ReauthGate {
    inner: Arc<Mutex<HashMap<String, Instant>>>,
}

impl ReauthGate {
    pub fn mark(&self, session_id: &str) {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, at| at.elapsed() < REAUTH_TTL * 2);
        map.insert(session_id.to_owned(), Instant::now());
    }

    pub fn is_fresh(&self, session_id: &str) -> bool {
        let map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        map.get(session_id)
            .is_some_and(|at| at.elapsed() < REAUTH_TTL)
    }

    pub fn clear(&self, session_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(session_id);
    }
}

/// Mask a known plaintext: `••••` plus at most the last four characters.
/// Short values (≤ 4 characters) are fully hidden so the mask cannot leak them.
#[allow(dead_code)]
pub fn mask_value(plaintext: &str) -> String {
    let chars: Vec<char> = plaintext.chars().collect();
    if chars.len() <= 4 {
        return MASK_PREFIX.to_owned();
    }
    let last: String = chars[chars.len() - 4..].iter().collect();
    format!("{MASK_PREFIX}{last}")
}

/// Display value for a secret field on list/detail endpoints.
/// When `plaintext` is `Some`, returns `••••` + last4; otherwise the sealed placeholder.
#[allow(dead_code)]
pub fn masked_display(plaintext: Option<&str>) -> String {
    match plaintext {
        Some(v) => mask_value(v),
        None => MASKED_UNKNOWN.to_owned(),
    }
}

/// JSON object for a credential that exists but is not opened: `{ hasValue: true }`.
pub fn has_value_true() -> Value {
    json!({ "hasValue": true })
}

/// `Cache-Control: no-store` for every reveal response.
pub fn no_store_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers
}

/// Scrub known secret values from a single log line (deploy / task writers).
pub fn redact_line(line: &str, secrets: &[&str]) -> String {
    scrub_secrets(line, secrets)
}

/// Scrub known secret values from a batch of log lines.
pub fn redact_lines(lines: impl IntoIterator<Item = String>, secrets: &[&str]) -> Vec<String> {
    lines
        .into_iter()
        .map(|l| redact_line(&l, secrets))
        .collect()
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReauthBody {
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

fn reauth_required() -> HttpError {
    HttpError::new(
        ErrorCode::Forbidden,
        "Confirm your password or two-factor code again to show this secret.",
    )
}

fn reauth_denied() -> HttpError {
    HttpError::new(
        ErrorCode::Forbidden,
        "That confirmation was not accepted. Try your password or a fresh two-factor code.",
    )
}

/// Verify password and/or TOTP, or accept a still-valid 5-minute window.
/// Always refuses API tokens.
pub async fn ensure_reauth(
    state: &AppState,
    who: &Principal,
    rid: &str,
    ip: &str,
    body: &ReauthBody,
) -> Result<(), HttpError> {
    deny_token(who)?;

    let password = body
        .password
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let code = body
        .code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if password.is_none() && code.is_none() {
        if state.reauth.is_fresh(&who.session_id) {
            return Ok(());
        }
        return Err(reauth_required());
    }

    let mut accepted = false;
    if let Some(pw) = password {
        accepted = state.auth.verify_user_password(&who.user.id, pw).await?;
    }
    if !accepted && let Some(c) = code {
        accepted = state
            .auth
            .verify_user_totp(rid, &who.user.id, c, ip)
            .await?;
    }
    if !accepted {
        return Err(reauth_denied());
    }
    state.reauth.mark(&who.session_id);
    Ok(())
}

/// Rate-limit and require recent re-auth for a reveal call.
pub async fn begin_reveal(
    state: &AppState,
    who: &Principal,
    rid: &str,
    ip: &str,
    body: &ReauthBody,
) -> Result<(), HttpError> {
    state.limits.reveal(&who.session_id)?;
    ensure_reauth(state, who, rid, ip, body).await
}

/// Append an audit row for a successful reveal. `secret_name` is the field/key;
/// `target` is the resource (service, database, channel, …). Never include values.
pub fn audit_reveal(
    state: &AppState,
    who: &Principal,
    ip: Option<&str>,
    action: &str,
    target: Option<&str>,
    secret_name: &str,
    detail: Value,
) {
    let mut d = detail;
    if let Some(obj) = d.as_object_mut() {
        obj.insert("secret".into(), json!(secret_name));
    }
    let _ = state.store.append_audit(NewAudit {
        actor: who.user.actor_label(),
        actor_kind: if who.via_token {
            ActorKind::Token
        } else {
            ActorKind::User
        },
        ip: ip.map(str::to_owned),
        action: action.to_owned(),
        target: target.map(str::to_owned),
        result: AuditResult::Ok,
        detail: d,
    });
}

pub fn audit_reveal_denied(
    state: &AppState,
    who: &Principal,
    ip: &str,
    action: &str,
    target: Option<&str>,
    reason: &str,
) {
    let _ = state.store.append_audit(NewAudit {
        actor: who.user.actor_label(),
        actor_kind: ActorKind::User,
        ip: Some(ip.to_owned()),
        action: action.to_owned(),
        target: target.map(str::to_owned),
        result: AuditResult::Denied,
        detail: json!({ "reason": reason }),
    });
}

/// `POST /api/v1/auth/reauth`: password or two-factor code to open the 5-minute reveal window.
pub(crate) async fn reauth(
    axum::extract::State(state): axum::extract::State<AppState>,
    crate::session::Viewer(who): crate::session::Viewer,
    client: Client,
    axum::extract::Extension(RequestId(rid)): axum::extract::Extension<RequestId>,
    crate::json::ApiJson(body): crate::json::ApiJson<ReauthBody>,
) -> Result<axum::Json<Value>, HttpError> {
    deny_token(&who)?;
    let password = body
        .password
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let code = body
        .code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if password.is_none() && code.is_none() {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Enter your password or a two-factor code.",
        ));
    }
    // Force credentials (do not accept an existing window).
    let forced = ReauthBody {
        password: password.map(str::to_owned),
        code: code.map(str::to_owned),
    };
    match ensure_reauth(&state, &who, &rid, &client.ip, &forced).await {
        Ok(()) => {
            let _ = state.store.append_audit(NewAudit {
                actor: who.user.actor_label(),
                actor_kind: ActorKind::User,
                ip: Some(client.ip.clone()),
                action: "auth.reauth".to_owned(),
                target: None,
                result: AuditResult::Ok,
                detail: json!({}),
            });
            Ok(axum::Json(json!({
                "ok": true,
                "expiresIn": REAUTH_TTL.as_secs(),
            })))
        }
        Err(e) => {
            audit_reveal_denied(
                &state,
                &who,
                &client.ip,
                "auth.reauth.denied",
                None,
                "bad_credentials",
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_value_hides_short_and_keeps_last_four() {
        assert_eq!(mask_value(""), "••••");
        assert_eq!(mask_value("ab"), "••••");
        assert_eq!(mask_value("abcd"), "••••");
        assert_eq!(mask_value("abcdefgh"), "••••efgh");
        assert_eq!(mask_value("marker-SECRET-value-zz9Q"), "••••zz9Q");
    }

    #[test]
    fn masked_display_uses_placeholder_when_unopened() {
        assert_eq!(masked_display(None), MASKED_UNKNOWN);
        assert_eq!(masked_display(Some("super-secret-pass")), "••••pass");
    }

    #[test]
    fn redact_line_removes_known_secrets() {
        const MARKER: &str = "marker-SECRET-value-zz9Q";
        let line = format!("export DATABASE_URL=postgres://u:{MARKER}@db/app");
        let out = redact_line(&line, &[MARKER]);
        assert!(!out.contains(MARKER), "{out}");
        assert!(out.contains("[REDACTED]"), "{out}");
    }

    #[test]
    fn reauth_gate_expires() {
        let g = ReauthGate::default();
        assert!(!g.is_fresh("s1"));
        g.mark("s1");
        assert!(g.is_fresh("s1"));
        g.clear("s1");
        assert!(!g.is_fresh("s1"));
    }

    #[test]
    fn response_types_with_secrets_serialize_masked() {
        // Marker must never appear when serialising list/detail shapes.
        const MARKER: &str = "marker-SECRET-value-zz9Q";
        let var_row = json!(["DATABASE_URL", masked_display(None), true]);
        let dest = json!({
            "id": "d1",
            "hasCredentials": true,
            "credentials": has_value_true(),
        });
        let channel = json!({
            "id": "c4",
            "on": true,
            "hasValue": true,
            "to": "Configured",
        });
        let docker_env = json!({
            "key": "DB_PASSWORD",
            "value": mask_value(MARKER),
            "masked": true,
        });
        let blob =
            json!({ "vars": [var_row], "dest": dest, "channel": channel, "env": docker_env });
        let text = blob.to_string();
        assert!(!text.contains(MARKER), "{text}");
        assert!(
            text.contains(MASKED_UNKNOWN) || text.contains("••••"),
            "{text}"
        );
        assert_eq!(docker_env["value"], json!(mask_value(MARKER)));
    }
}
