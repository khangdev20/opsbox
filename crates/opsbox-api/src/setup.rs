//! First-run setup (docs/BACKEND_SPEC.md 6.3). These endpoints answer `404` as soon
//! as an admin has finished two-factor enrolment, as if they had never existed.

use axum::Json;
use axum::extract::{Extension, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::auth::host_label;
use crate::json::ApiJson;
use crate::session::{
    Client, SETUP_COOKIE, check_origin_if_present, cookie_value, session_cookies, setup_cookie,
};
use crate::{AppState, HttpError, RequestId};

const SETUP_MAX_AGE: i64 = 30 * 60;

/// Once an admin is enrolled these endpoints behave as if they did not exist. Checked
/// before anything else (cookie, Origin, body) so a stranger learns nothing more.
fn ensure_open(state: &AppState) -> Result<(), HttpError> {
    if state.auth.setup_required()? {
        Ok(())
    } else {
        Err(HttpError::not_found())
    }
}

#[derive(Deserialize)]
pub(crate) struct ClaimBody {
    token: String,
}

/// `GET /api/v1/setup/status`: tells the UI whether first-run setup is still open.
pub(crate) async fn status(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, HttpError> {
    ensure_open(&state)?;
    Ok(Json(json!({ "setupRequired": true })))
}

/// `POST /api/v1/setup/claim`: trade the console token for a short setup session.
pub(crate) async fn claim(
    State(state): State<AppState>,
    client: Client,
    ApiJson(body): ApiJson<ClaimBody>,
) -> Result<Response, HttpError> {
    ensure_open(&state)?;
    check_origin_if_present(&client.headers)?;
    let session = state.auth.claim_setup(&body.token, &client.ip)?;
    Ok((
        setup_cookie(
            state.secure_cookies,
            &client.headers,
            &session,
            SETUP_MAX_AGE,
        ),
        Json(json!({ "ok": true })),
    )
        .into_response())
}

fn setup_session(client: &Client) -> Result<&str, HttpError> {
    cookie_value(&client.headers, SETUP_COOKIE).ok_or_else(|| {
        HttpError::from(opsbox_auth::AuthError::Invalid {
            field: "token",
            message: "Claim the server with the setup token first.".into(),
        })
    })
}

#[derive(Deserialize)]
pub(crate) struct AdminBody {
    username: String,
    password: String,
}

/// `POST /api/v1/setup/admin`: create the admin account; returns what the authenticator needs.
pub(crate) async fn admin(
    State(state): State<AppState>,
    client: Client,
    Extension(RequestId(request_id)): Extension<RequestId>,
    ApiJson(body): ApiJson<AdminBody>,
) -> Result<Json<serde_json::Value>, HttpError> {
    ensure_open(&state)?;
    check_origin_if_present(&client.headers)?;
    let session = setup_session(&client)?.to_owned();
    let host = host_label(&state, &request_id).await;
    let e = state
        .auth
        .create_admin(
            &request_id,
            &session,
            &body.username,
            &body.password,
            &host,
            &client.ip,
        )
        .await?;
    Ok(Json(
        json!({ "secret": e.secret_base32, "otpauthUri": e.otpauth_uri, "account": body.username.trim().to_lowercase() }),
    ))
}

#[derive(Deserialize)]
pub(crate) struct ConfirmBody {
    code: String,
}

/// `POST /api/v1/setup/totp/confirm`: the first code finishes setup and signs the admin in.
pub(crate) async fn confirm(
    State(state): State<AppState>,
    client: Client,
    Extension(RequestId(request_id)): Extension<RequestId>,
    ApiJson(body): ApiJson<ConfirmBody>,
) -> Result<Response, HttpError> {
    ensure_open(&state)?;
    check_origin_if_present(&client.headers)?;
    let session = setup_session(&client)?.to_owned();
    let issued = state
        .auth
        .finish_setup(
            &request_id,
            &session,
            &body.code,
            &client.ip,
            client.user_agent.as_deref(),
        )
        .await?;
    // Sign in, and retire the setup cookie.
    let cookies = (
        session_cookies(
            state.secure_cookies,
            &client.headers,
            &issued.token,
            7 * 24 * 3600,
        ),
        setup_cookie(state.secure_cookies, &client.headers, "", 0),
    );
    Ok((
        cookies,
        Json(json!({ "user": issued.user.name, "role": issued.user.role })),
    )
        .into_response())
}
