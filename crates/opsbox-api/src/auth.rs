//! Sign-in, sessions and two-factor enrolment endpoints (docs/BACKEND_SPEC.md 6.2).

use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use opsbox_auth::{LoginOutcome, LoginRequest, SessionKindView};
use serde::Deserialize;
use serde_json::json;

use crate::json::ApiJson;
use crate::session::{
    Client, Enrolling, SESSION_COOKIE, Viewer, check_origin_if_present, clear_session_cookies,
    cookie_value, session_cookies,
};
use crate::{AppState, HttpError, RequestId};

/// Cookie lifetime for a normal session (the seven-day absolute limit).
const SESSION_MAX_AGE: i64 = 7 * 24 * 3600;
/// An enrolment-only session is short.
const ENROLL_MAX_AGE: i64 = 15 * 60;

#[derive(Deserialize)]
pub(crate) struct LoginBody {
    username: String,
    password: String,
    #[serde(default)]
    code: String,
}

/// `GET /api/v1/session`: public. Tells the page what to show first.
pub(crate) async fn session(
    State(state): State<AppState>,
    client: Client,
) -> Result<Json<serde_json::Value>, HttpError> {
    let setup_required = state.auth.setup_required()?;
    let who = match cookie_value(&client.headers, SESSION_COOKIE) {
        Some(token) => state.auth.authenticate(token)?,
        None => None,
    };
    Ok(Json(match who {
        Some(p) => json!({
            "authenticated": p.kind == SessionKindView::Web,
            "enrollRequired": p.kind == SessionKindView::Enroll,
            "user": p.user.name,
            "role": p.user.role,
            "setupRequired": setup_required,
        }),
        None => {
            json!({ "authenticated": false, "enrollRequired": false, "setupRequired": setup_required })
        }
    }))
}

/// `POST /api/v1/auth/login`
pub(crate) async fn login(
    State(state): State<AppState>,
    client: Client,
    Extension(RequestId(request_id)): Extension<RequestId>,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Response, HttpError> {
    check_origin_if_present(&client.headers)?;
    let outcome = state
        .auth
        .login(
            &request_id,
            LoginRequest {
                username: body.username,
                password: body.password,
                code: body.code,
                ip: Some(client.ip.clone()),
                user_agent: client.user_agent.clone(),
            },
        )
        .await?;
    let (issued, enroll, max_age) = match outcome {
        LoginOutcome::Session(i) => (i, false, SESSION_MAX_AGE),
        LoginOutcome::EnrollRequired(i) => (i, true, ENROLL_MAX_AGE),
    };
    let cookies = session_cookies(
        state.secure_cookies,
        &client.headers,
        &issued.token,
        max_age,
    );
    let body = Json(
        json!({ "user": issued.user.name, "role": issued.user.role, "enrollRequired": enroll }),
    );
    Ok((cookies, body).into_response())
}

/// `POST /api/v1/auth/logout`
pub(crate) async fn logout(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    client: Client,
) -> Result<Response, HttpError> {
    state.auth.logout(&who, Some(&client.ip))?;
    Ok((
        clear_session_cookies(state.secure_cookies, &client.headers),
        Json(json!({ "ok": true })),
    )
        .into_response())
}

/// `POST /api/v1/auth/sessions/revoke-all`: sign out on every device, this one included.
pub(crate) async fn revoke_all(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    client: Client,
) -> Result<Response, HttpError> {
    let n = state.auth.revoke_all(&who.user, Some(&client.ip))?;
    Ok((
        clear_session_cookies(state.secure_cookies, &client.headers),
        Json(json!({ "sessions": n })),
    )
        .into_response())
}

/// The machine name for the authenticator app's label; never worth failing a request.
pub(crate) async fn host_label(state: &AppState, request_id: &str) -> String {
    state
        .priv_client
        .host_info(request_id, None)
        .await
        .map_or_else(|_| "server".to_owned(), |h| h.hostname)
}

/// `POST /api/v1/auth/totp/enroll`: start two-factor enrolment (enrolment sessions only).
pub(crate) async fn totp_enroll(
    State(state): State<AppState>,
    Enrolling(who): Enrolling,
    Extension(RequestId(request_id)): Extension<RequestId>,
) -> Result<Json<serde_json::Value>, HttpError> {
    let host = host_label(&state, &request_id).await;
    let e = state.auth.begin_totp(&request_id, &who.user, &host).await?;
    Ok(Json(
        json!({ "secret": e.secret_base32, "otpauthUri": e.otpauth_uri, "account": who.user.name }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct CodeBody {
    code: String,
}

/// `POST /api/v1/auth/totp/confirm`: first code from the app; the session becomes a full one.
pub(crate) async fn totp_confirm(
    State(state): State<AppState>,
    Enrolling(who): Enrolling,
    client: Client,
    Extension(RequestId(request_id)): Extension<RequestId>,
    ApiJson(body): ApiJson<CodeBody>,
) -> Result<Response, HttpError> {
    state
        .auth
        .confirm_totp(&request_id, &who.user, &body.code, &client.ip)
        .await?;
    let issued =
        state
            .auth
            .upgrade_enroll_session(&who, Some(&client.ip), client.user_agent.as_deref())?;
    let cookies = session_cookies(
        state.secure_cookies,
        &client.headers,
        &issued.token,
        SESSION_MAX_AGE,
    );
    Ok((
        cookies,
        Json(json!({ "user": issued.user.name, "role": issued.user.role })),
    )
        .into_response())
}
