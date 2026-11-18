//! Panel accounts: invitations, profile and removal (Security page, docs/BACKEND_SPEC.md 6.4).
//!
//! An administrator invites someone by name and role and gets a one-time link. The person
//! opens it, chooses a password and sets up two-factor sign-in, the same enrolment the first
//! administrator goes through. Anyone may change their own display name; only an administrator
//! may change someone else's.

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use opsbox_proto::{ErrorCode, Role};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::json::ApiJson;
use crate::session::{Admin, Client, Viewer, check_origin_if_present, session_cookies};
use crate::{AppState, HttpError};

/// An enrolment-only session is short (the same as after a sign-in without 2FA).
const ENROLL_MAX_AGE: i64 = 15 * 60;

#[derive(Deserialize)]
pub(crate) struct InviteBody {
    name: String,
    role: String,
    #[serde(default)]
    display_name: Option<String>,
}

/// `POST /users/invites`: admins only.
pub(crate) async fn invite(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    ApiJson(body): ApiJson<InviteBody>,
) -> Result<Json<serde_json::Value>, HttpError> {
    crate::session::deny_token(&who)?;
    let role = Role::parse(&body.role).ok_or_else(|| {
        HttpError::new(ErrorCode::Validation, "Choose admin, deployer or viewer.")
    })?;
    let (token, expires) = state.auth.invite_user(
        &who.user,
        &body.name,
        role,
        body.display_name.as_deref(),
        Some(&client.ip),
    )?;
    Ok(Json(json!({
        "token": token, "expires": expires,
        "name": body.name.trim().to_lowercase(), "role": role,
        "displayName": body.display_name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    })))
}

/// Public invite peek: omit `displayName` when unset (optional field).
#[derive(Serialize)]
pub(crate) struct InviteInfoBody {
    name: String,
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    role: Role,
}

/// `GET /invites/{token}`: public. Who the invitation is for, without using it up.
pub(crate) async fn info(
    State(state): State<AppState>,
    client: Client,
    Path(token): Path<String>,
) -> Result<Json<InviteInfoBody>, HttpError> {
    let i = state.auth.invite_info(&token, &client.ip)?;
    Ok(Json(InviteInfoBody {
        name: i.name,
        display_name: i.display_name,
        role: i.role,
    }))
}

#[derive(Deserialize)]
pub(crate) struct ClaimBody {
    token: String,
    password: String,
    #[serde(default)]
    display_name: Option<String>,
}

/// `POST /invites/claim`: public. Choose a password; the answer is an enrolment session.
pub(crate) async fn claim(
    State(state): State<AppState>,
    client: Client,
    ApiJson(body): ApiJson<ClaimBody>,
) -> Result<Response, HttpError> {
    check_origin_if_present(&client.headers)?;
    let issued = state
        .auth
        .claim_invite(
            &body.token,
            &body.password,
            body.display_name.as_deref(),
            &client.ip,
            client.user_agent.as_deref(),
        )
        .await?;
    let cookies = session_cookies(client.https, &issued.token, ENROLL_MAX_AGE);
    let out = Json(json!({
        "user": issued.user.name,
        "displayName": issued.user.display_name,
        "role": issued.user.role,
        "enrollRequired": true,
    }));
    Ok((cookies, out).into_response())
}

#[derive(Deserialize)]
pub(crate) struct UpdateBody {
    #[serde(default)]
    display_name: Option<String>,
}

/// `PATCH /users/{name}`: change the display name. Yourself, or an admin for anyone.
pub(crate) async fn update(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    client: Client,
    Path(name): Path<String>,
    ApiJson(body): ApiJson<UpdateBody>,
) -> Result<Json<serde_json::Value>, HttpError> {
    crate::session::deny_token(&who)?;
    let target = name.trim().to_lowercase();
    let self_update = who.user.name.eq_ignore_ascii_case(&target);
    if !self_update && who.user.role != Role::Admin {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "Only an owner can change someone else's display name.",
        ));
    }
    let updated = state.auth.set_display_name(
        &who.user,
        &target,
        body.display_name.as_deref(),
        Some(&client.ip),
    )?;
    Ok(Json(json!({
        "name": updated.name,
        "displayName": updated.display_name,
        "role": updated.role,
    })))
}

/// `DELETE /users/{name}`: admins only.
pub(crate) async fn remove(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, HttpError> {
    crate::session::deny_token(&who)?;
    state.auth.remove_user(&who.user, &name, Some(&client.ip))?;
    Ok(Json(json!({ "ok": true })))
}
