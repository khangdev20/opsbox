//! `GET /access`, `POST /access/keys`, `POST /access/keys/remove`,
//! `POST /access/users/{name}/lock` (docs/BACKEND_SPEC.md 6.4).
//!
//! Who can log in to the server over SSH: Linux accounts, their `authorized_keys`, and the
//! SSH daemon's settings. Panel accounts (people who sign in to opsbox) are separate and live
//! on the Security page.

use axum::Json;
use axum::extract::{Extension, Path, State};
use opsbox_proto::{AccessUser, ErrorCode};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::session::{Admin, Client};
use crate::system::{audit, patient};
use crate::{AppState, HttpError, RequestId};

fn user_json(u: &AccessUser) -> Value {
    json!({
        "name": u.name, "groups": u.groups.join(", "), "sudo": u.sudo, "last": u.last_login,
        "locked": u.locked,
        "note": if u.uid == 0 { "direct login" } else if u.sudo { "can use sudo" } else { "" },
    })
}

/// A key's id is `<user>:<fingerprint>`: user names cannot contain a colon.
fn key_id(user: &str, fp: &str) -> String {
    format!("{user}:{fp}")
}

/// `GET /access`
pub(crate) async fn get(
    State(state): State<AppState>,
    Admin(_): Admin,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let pc = patient(&state, 30);
    let a = pc.access_list(&rid).await?;
    // Emergency SSH is a firewall entry; if the firewall cannot be read the page still works.
    let ssh_open = pc
        .firewall_list(&rid)
        .await
        .ok()
        .and_then(|f| f.ssh_open_until);
    Ok(Json(json!({
        "keys": a.keys.iter().map(|k| json!({
            "id": key_id(&k.user, &k.fingerprint), "user": k.user,
            "name": if k.name.is_empty() { "unnamed key" } else { &k.name },
            "type": k.kind, "fp": k.fingerprint,
            "note": k.options.as_deref().map(|o| format!("restricted: {o}")),
            "stale": if k.weak { Some("weak key type") } else { None },
            "added": null, "used": null,
        })).collect::<Vec<_>>(),
        "users": a.users.iter().map(user_json).collect::<Vec<_>>(),
        // Keys opsbox uses to reach other servers arrive with the desktop app (Phase 6).
        "out": [],
        "sshOpen": ssh_open.is_some(),
        "ssh": { "installed": a.ssh.installed, "active": a.ssh.active, "port": a.ssh.port,
                 "passwordLogin": a.ssh.password_login, "rootLogin": a.ssh.root_login },
    })))
}

#[derive(Deserialize)]
pub(crate) struct AddKey {
    user: String,
    key: String,
}

/// `POST /access/keys`
pub(crate) async fn add_key(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<AddKey>,
) -> Result<Json<Value>, HttpError> {
    crate::session::deny_token(&who)?;
    let fp = patient(&state, 30)
        .ssh_key_add(&rid, &who.user.name, &body.user, &body.key)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "access.key.add",
        Some(&body.user),
        json!({ "fingerprint": fp }),
    );
    Ok(Json(json!({ "fingerprint": fp })))
}

#[derive(Deserialize)]
pub(crate) struct RemoveKey {
    id: String,
}

/// `POST /access/keys/remove`
pub(crate) async fn remove_key(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<RemoveKey>,
) -> Result<Json<Value>, HttpError> {
    crate::session::deny_token(&who)?;
    let Some((user, fp)) = body.id.split_once(':') else {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "That is not a key id.",
        ));
    };
    patient(&state, 30)
        .ssh_key_remove(&rid, &who.user.name, user, fp)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "access.key.remove",
        Some(user),
        json!({ "fingerprint": fp }),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct LockBody {
    lock: bool,
}

/// `POST /access/users/{name}/lock`
pub(crate) async fn lock_user(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(name): Path<String>,
    ApiJson(body): ApiJson<LockBody>,
) -> Result<Json<Value>, HttpError> {
    crate::session::deny_token(&who)?;
    patient(&state, 30)
        .user_lock(&rid, &who.user.name, &name, body.lock)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        if body.lock {
            "access.user.lock"
        } else {
            "access.user.unlock"
        },
        Some(&name),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}
