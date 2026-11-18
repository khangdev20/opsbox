//! Who is calling: cookies, role checks, CSRF and Origin checks.
//!
//! A handler declares what it needs by taking one of the extractors [`Viewer`],
//! [`Deployer`] or [`Admin`] as a parameter. The extractor authenticates the
//! session, refuses roles that are too weak and, for anything that changes state,
//! verifies the CSRF token and the Origin header. A handler that takes none of them
//! is public; there is no way to forget the check on a handler that takes one.

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::header::{AUTHORIZATION, COOKIE, HOST, ORIGIN, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Method};
use axum::response::{AppendHeaders, IntoResponseParts};
use opsbox_auth::tokens::{self, csrf_token};
use opsbox_auth::{Principal, SessionKindView};
use opsbox_proto::{ErrorCode, Role};

use crate::{AppState, HttpError};

pub const SESSION_COOKIE: &str = "opsbox_session";
pub const CSRF_COOKIE: &str = "opsbox_csrf";
pub const SETUP_COOKIE: &str = "opsbox_setup";
pub const CSRF_HEADER: &str = "x-opsbox-csrf";

/// When to mark cookies `Secure` (browsers then send them over HTTPS only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecureCookies {
    /// When the request came through an HTTPS proxy (`X-Forwarded-Proto: https`).
    Auto,
    Always,
    Never,
}

pub fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(k, v)| (k == name).then_some(v))
}

fn is_https(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

/// Cookie attributes shared by all of ours.
fn attrs(
    secure: SecureCookies,
    headers: &HeaderMap,
    http_only: bool,
    max_age: i64,
    path: &str,
) -> String {
    let mut s = format!("; Path={path}; SameSite=Strict; Max-Age={max_age}");
    if http_only {
        s.push_str("; HttpOnly");
    }
    if secure == SecureCookies::Always || (secure == SecureCookies::Auto && is_https(headers)) {
        s.push_str("; Secure");
    }
    s
}

fn header(value: String) -> HeaderValue {
    // Tokens are base64url or hex and attribute text is ASCII.
    HeaderValue::from_str(&value).expect("cookie is plain ASCII")
}

/// `Set-Cookie` headers that start a session: the secret cookie (not readable by
/// scripts) and the CSRF cookie (read by the page and sent back in a header).
pub fn session_cookies(
    secure: SecureCookies,
    req_headers: &HeaderMap,
    token: &str,
    max_age: i64,
) -> impl IntoResponseParts + use<> {
    AppendHeaders([
        (
            SET_COOKIE,
            header(format!(
                "{SESSION_COOKIE}={token}{}",
                attrs(secure, req_headers, true, max_age, "/")
            )),
        ),
        (
            SET_COOKIE,
            header(format!(
                "{CSRF_COOKIE}={}{}",
                csrf_token(token),
                attrs(secure, req_headers, false, max_age, "/")
            )),
        ),
    ])
}

pub fn clear_session_cookies(
    secure: SecureCookies,
    req_headers: &HeaderMap,
) -> impl IntoResponseParts + use<> {
    AppendHeaders([
        (
            SET_COOKIE,
            header(format!(
                "{SESSION_COOKIE}={}",
                attrs(secure, req_headers, true, 0, "/")
            )),
        ),
        (
            SET_COOKIE,
            header(format!(
                "{CSRF_COOKIE}={}",
                attrs(secure, req_headers, false, 0, "/")
            )),
        ),
    ])
}

/// The first-run setup cookie only goes to the setup endpoints.
pub fn setup_cookie(
    secure: SecureCookies,
    req_headers: &HeaderMap,
    token: &str,
    max_age: i64,
) -> impl IntoResponseParts + use<> {
    AppendHeaders([(
        SET_COOKIE,
        header(format!(
            "{SETUP_COOKIE}={token}{}",
            attrs(secure, req_headers, true, max_age, "/api/v1/setup")
        )),
    )])
}

/// The caller's address. Behind a local proxy the address comes from
/// `X-Forwarded-For` (its rightmost entry, the one the proxy added); otherwise it is
/// the TCP peer. Unknown in tests that call the router directly.
pub fn client_ip(parts: &Parts) -> String {
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0.ip());
    if peer.is_some_and(|ip| ip.is_loopback())
        && let Some(fwd) = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
    {
        return fwd.to_string();
    }
    peer.map_or_else(|| "unknown".to_owned(), |ip| ip.to_string())
}

pub fn user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// A browser sends `Origin` on every cross-site request. It must name this server.
pub fn origin_matches_host(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(ORIGIN).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let authority = origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"));
    let host = headers.get(HOST).and_then(|v| v.to_str().ok());
    matches!((authority, host), (Some(a), Some(h)) if a.eq_ignore_ascii_case(h))
}

fn forbidden(message: &str) -> HttpError {
    HttpError::new(ErrorCode::Forbidden, message)
}

fn unauthorized() -> HttpError {
    HttpError::new(ErrorCode::Unauthorized, "Sign in to continue.")
}

fn is_safe(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Persist acceptance before dispatching an authenticated mutation. If the audit
/// store is unavailable, no system command is started. Handlers add the detailed
/// outcome under their domain-specific action names afterwards.
async fn audit_acceptance(
    parts: &Parts,
    state: &AppState,
    who: &Principal,
) -> Result<(), HttpError> {
    if is_safe(&parts.method) {
        return Ok(());
    }
    let store = state.store.clone();
    let entry = opsbox_store::NewAudit {
        actor: who.user.name.clone(),
        actor_kind: if who.via_token {
            opsbox_store::ActorKind::Token
        } else {
            opsbox_store::ActorKind::User
        },
        ip: Some(client_ip(parts)),
        action: "request.accepted".into(),
        target: Some(parts.uri.path().chars().take(512).collect()),
        result: opsbox_store::AuditResult::Ok,
        detail: serde_json::json!({ "method": parts.method.as_str(), "phase": "accepted" }),
    };
    if !tokio::task::spawn_blocking(move || store.append_audit(entry))
        .await
        .is_ok_and(|r| r.is_ok())
    {
        tracing::error!("refused a mutation because its acceptance could not be audited");
        return Err(HttpError::new(
            ErrorCode::Unavailable,
            "The audit log could not be written. No change was started. Check disk space and database permissions.",
        ));
    }
    Ok(())
}

/// Pre-login requests (sign in, setup) have no session to protect, but a page on
/// another site must not be able to submit them from a visitor's browser.
pub fn check_origin_if_present(headers: &HeaderMap) -> Result<(), HttpError> {
    if headers.contains_key(ORIGIN) && !origin_matches_host(headers) {
        return Err(forbidden(
            "This request came from another site and was refused.",
        ));
    }
    Ok(())
}

/// The token in an `Authorization: Bearer <token>` header.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

/// Refuse a caller that used an API token. Managing people, tokens, SSH keys and terminals
/// needs a signed-in account.
pub fn deny_token(who: &Principal) -> Result<(), HttpError> {
    if who.via_token {
        return Err(forbidden(
            "API tokens cannot do this. Sign in with an account instead.",
        ));
    }
    Ok(())
}

/// Cancel streams within five seconds of revocation, expiry or an allow-list change.
/// No plaintext credential is kept by the upgraded connection.
pub(crate) async fn stream_ended(state: AppState, who: Principal, ip: String) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        if !state.allow.get().is_empty()
            && !ip.parse::<IpAddr>().is_ok_and(|a| state.allow.permits(a))
        {
            return;
        }
        let auth = state.auth.clone();
        let principal = who.clone();
        if !tokio::task::spawn_blocking(move || auth.stream_active(&principal))
            .await
            .is_ok_and(|r| r.unwrap_or(false))
        {
            return;
        }
    }
}

pub(crate) fn stream_slot(
    state: &AppState,
) -> Result<tokio::sync::OwnedSemaphorePermit, HttpError> {
    state.stream_slots.clone().try_acquire_owned().map_err(|_| {
        HttpError::new(
            ErrorCode::RateLimited,
            "Too many streams are open. Close a tab and try again.",
        )
    })
}

pub(crate) fn bounded_socket(
    ws: axum::extract::ws::WebSocketUpgrade,
) -> axum::extract::ws::WebSocketUpgrade {
    ws.max_message_size(64 * 1024)
        .max_frame_size(64 * 1024)
        .write_buffer_size(16 * 1024)
        .max_write_buffer_size(256 * 1024)
}

/// Authenticate the caller with `role` at least, applying CSRF rules to unsafe methods.
async fn authorize(
    parts: &mut Parts,
    state: &AppState,
    role: Role,
    allow_enrol: bool,
) -> Result<Principal, HttpError> {
    // A script may use an API token instead of a session. Nothing about a token is ambient
    // (a browser never adds it by itself), so CSRF and Origin checks do not apply to it.
    if cookie_value(&parts.headers, SESSION_COOKIE).is_none()
        && let Some(secret) = bearer(&parts.headers)
    {
        let auth = state.auth.clone();
        let secret = secret.to_owned();
        let who = tokio::task::spawn_blocking(move || auth.authenticate_token(&secret))
            .await
            .map_err(|_| HttpError::internal())??
            .ok_or_else(unauthorized)?;
        state.limits.session(&who.session_id)?;
        if !who.user.role.allows(role) {
            return Err(forbidden("This token is not allowed to do this."));
        }
        // Scopes are capabilities, not just role ranks. In particular a deployment
        // token must never inherit a deployer's write access to application files.
        let path = parts.uri.path();
        let metrics = matches!(
            path,
            "/api/v1/host" | "/api/v1/metrics" | "/api/v1/attention" | "/api/v1/stream/metrics"
        );
        let allowed = match who.user.role {
            Role::Viewer => is_safe(&parts.method) && metrics,
            Role::Deployer => {
                let deploy_mutation = parts.method == Method::POST
                    && path.starts_with("/api/v1/services/")
                    && (path.ends_with("/deploys") || path.ends_with("/rollback"));
                let deploy_read = is_safe(&parts.method)
                    && (path.starts_with("/api/v1/projects/")
                        || (path.starts_with("/api/v1/services/") && !path.contains("/vars"))
                        || path.starts_with("/api/v1/deploys/")
                        || path.starts_with("/api/v1/stream/deploys/")
                        || path.starts_with("/api/v1/tasks/"));
                if who.scoped_service.is_some() {
                    deploy_mutation || deploy_read
                } else {
                    deploy_mutation
                        || deploy_read
                        || (is_safe(&parts.method)
                            && (metrics
                                || matches!(
                                    path,
                                    "/api/v1/system/journal"
                                        | "/api/v1/stream/journal"
                                        | "/api/v1/github"
                                        | "/api/v1/github/repos"
                                        | "/api/v1/templates"
                                )
                                || path.starts_with("/api/v1/docker/")
                                || path.starts_with("/api/v1/stream/containers/")))
                }
            }
            Role::Admin => true,
        };
        if !allowed {
            return Err(forbidden("This endpoint is outside the token's scope."));
        }
        if allow_enrol {
            return Err(forbidden("This needs a signed-in account, not a token."));
        }
        audit_acceptance(parts, state, &who).await?;
        return Ok(who);
    }
    let token = cookie_value(&parts.headers, SESSION_COOKIE)
        .ok_or_else(unauthorized)?
        .to_owned();
    let who = {
        let auth = state.auth.clone();
        let t = token.clone();
        tokio::task::spawn_blocking(move || auth.authenticate(&t))
            .await
            .map_err(|_| HttpError::internal())??
    }
    .ok_or_else(unauthorized)?;

    state.limits.session(&who.session_id)?;

    if who.kind == SessionKindView::Enroll && !allow_enrol {
        return Err(forbidden(
            "Finish setting up two-factor authentication first.",
        ));
    }
    if !who.user.role.allows(role) {
        return Err(forbidden("Your account is not allowed to do this."));
    }
    if !is_safe(&parts.method) {
        let sent = parts
            .headers
            .get(CSRF_HEADER)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !tokens::ct_eq(sent, &csrf_token(&token)) || !origin_matches_host(&parts.headers) {
            return Err(forbidden(
                "The request could not be verified. Reload the page and try again.",
            ));
        }
    }
    audit_acceptance(parts, state, &who).await?;
    Ok(who)
}

macro_rules! role_extractor {
    ($(#[$doc:meta])* $name:ident, $role:expr) => {
        $(#[$doc])*
        pub struct $name(pub Principal);

        impl FromRequestParts<AppState> for $name {
            type Rejection = HttpError;
            async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, HttpError> {
                authorize(parts, state, $role, false).await.map($name)
            }
        }
    };
}

role_extractor!(
    /// Any signed-in account (read-only access is enough).
    Viewer,
    Role::Viewer
);
role_extractor!(
    /// Deployers and admins.
    Deployer,
    Role::Deployer
);
role_extractor!(
    /// Admins only.
    Admin,
    Role::Admin
);

/// A signed-in browser session (never a token) for the data-tool gateway (`/tools/...`). The
/// proxied tools make their own requests and cannot send the panel's CSRF header, so unsafe
/// methods are accepted only when the browser's `Origin` names this server (the session cookie
/// is also `SameSite=Strict`).
pub struct ToolGateway(pub Principal);

impl FromRequestParts<AppState> for ToolGateway {
    type Rejection = HttpError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, HttpError> {
        let token = cookie_value(&parts.headers, SESSION_COOKIE)
            .ok_or_else(unauthorized)?
            .to_owned();
        let who = {
            let auth = state.auth.clone();
            tokio::task::spawn_blocking(move || auth.authenticate(&token))
                .await
                .map_err(|_| HttpError::internal())??
        }
        .ok_or_else(unauthorized)?;
        state.limits.session(&who.session_id)?;
        if who.kind != SessionKindView::Web || !who.user.role.allows(Role::Deployer) {
            return Err(forbidden("Your account is not allowed to do this."));
        }
        if !is_safe(&parts.method) && !origin_matches_host(&parts.headers) {
            return Err(forbidden(
                "The request could not be verified. Reload the page and try again.",
            ));
        }
        audit_acceptance(parts, state, &who).await?;
        Ok(Self(who))
    }
}

/// A session that may only finish two-factor enrolment.
pub struct Enrolling(pub Principal);

impl FromRequestParts<AppState> for Enrolling {
    type Rejection = HttpError;
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, HttpError> {
        let who = authorize(parts, state, Role::Viewer, true).await?;
        if who.kind != SessionKindView::Enroll {
            return Err(forbidden("Two-factor authentication is already set up."));
        }
        Ok(Self(who))
    }
}

/// The caller's address and user agent, for audit entries and sessions.
pub struct Client {
    pub ip: String,
    pub user_agent: Option<String>,
    pub headers: HeaderMap,
}

impl<S: Send + Sync> FromRequestParts<S> for Client {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self {
            ip: client_ip(parts),
            user_agent: user_agent(&parts.headers),
            headers: parts.headers.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn cookies_are_found_by_exact_name() {
        let h = headers(&[
            ("cookie", "a=1; opsbox_session=tok; opsbox_session_x=no"),
            ("cookie", "b=2"),
        ]);
        assert_eq!(cookie_value(&h, "opsbox_session"), Some("tok"));
        assert_eq!(cookie_value(&h, "b"), Some("2"));
        assert_eq!(cookie_value(&h, "missing"), None);
    }

    #[test]
    fn origin_must_name_this_host() {
        let ok = headers(&[
            ("origin", "http://127.0.0.1:8443"),
            ("host", "127.0.0.1:8443"),
        ]);
        assert!(origin_matches_host(&ok));
        assert!(origin_matches_host(&headers(&[
            ("origin", "https://Panel.Example.com"),
            ("host", "panel.example.com")
        ])));
        for (o, h) in [
            ("http://evil.example", "127.0.0.1:8443"),
            ("http://127.0.0.1:8443.evil.example", "127.0.0.1:8443"),
            ("null", "127.0.0.1:8443"),
            ("ftp://127.0.0.1:8443", "127.0.0.1:8443"),
        ] {
            assert!(
                !origin_matches_host(&headers(&[("origin", o), ("host", h)])),
                "{o} vs {h}"
            );
        }
        assert!(
            !origin_matches_host(&headers(&[("host", "127.0.0.1:8443")])),
            "missing Origin"
        );
    }

    #[test]
    fn pre_login_origin_check_allows_absent_but_not_foreign() {
        assert!(check_origin_if_present(&headers(&[("host", "x")])).is_ok());
        assert!(
            check_origin_if_present(&headers(&[("origin", "http://x"), ("host", "x")])).is_ok()
        );
        assert!(
            check_origin_if_present(&headers(&[("origin", "http://evil"), ("host", "x")])).is_err()
        );
    }

    #[test]
    fn secure_attribute_follows_the_policy() {
        let plain = HeaderMap::new();
        let via_https = headers(&[("x-forwarded-proto", "https")]);
        assert!(!attrs(SecureCookies::Auto, &plain, true, 10, "/").contains("Secure"));
        assert!(attrs(SecureCookies::Auto, &via_https, true, 10, "/").contains("Secure"));
        assert!(attrs(SecureCookies::Always, &plain, true, 10, "/").contains("Secure"));
        assert!(!attrs(SecureCookies::Never, &via_https, true, 10, "/").contains("Secure"));
        let a = attrs(SecureCookies::Never, &plain, true, 10, "/");
        assert!(
            a.contains("HttpOnly") && a.contains("SameSite=Strict") && a.contains("Max-Age=10")
        );
        assert!(!attrs(SecureCookies::Never, &plain, false, 10, "/").contains("HttpOnly"));
    }
}
