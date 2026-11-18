//! Data tools (pgweb, Adminer) behind opsbox sign-in (docs/BACKEND_SPEC.md 7.6).
//!
//! The tool containers listen on `127.0.0.1` only. `/tools/<service>/...` is the one door to
//! them: it needs a signed-in panel session (and ownership of the tool), never forwards the
//! panel's own cookies, and keeps the tool's cookies scoped to its path.

use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::header::{
    AUTHORIZATION, CONNECTION, CONTENT_LENGTH, COOKIE, HOST, LOCATION, ORIGIN, REFERER, SET_COOKIE,
    TRANSFER_ENCODING, UPGRADE,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use http_body_util::Limited;
use hyper::Request as HyperRequest;
use hyper_util::rt::TokioIo;
use opsbox_proto::ErrorCode;
use opsbox_store::ServiceKind;
use serde_json::json;
use tokio::net::TcpStream;

use crate::deploy::{audit_user, service_writable};
use crate::session::{Client, ToolGateway};
use crate::{AppState, HttpError};

/// Templates that may be reached through the gateway.
const DATA_TOOLS: &[&str] = &["adminer", "pgweb"];
const MAX_BODY: usize = 8 << 20;
const HEADER_TIMEOUT: Duration = Duration::from_secs(120);

const HOP_BY_HOP: &[&str] = &[
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "x-opsbox-csrf",
];

fn is_hop(name: &HeaderName) -> bool {
    name == CONNECTION
        || name == TRANSFER_ENCODING
        || name == UPGRADE
        || name == HOST
        || HOP_BY_HOP.contains(&name.as_str())
}

/// Cookies for the tool: everything except the panel's own.
fn tool_cookies(value: &str) -> Option<String> {
    let kept: Vec<&str> = value
        .split(';')
        .map(str::trim)
        .filter(|c| {
            !c.is_empty() && !c.starts_with("opsbox_session=") && !c.starts_with("opsbox_csrf=")
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

/// Keep a tool's cookie on `prefix` only, as `SameSite=Strict`.
fn scope_set_cookie(value: &str, prefix: &str) -> String {
    let mut parts: Vec<String> = value
        .split(';')
        .map(|p| p.trim().to_owned())
        .filter(|p| {
            let l = p.to_ascii_lowercase();
            !l.starts_with("path=") && !l.starts_with("domain=") && !l.starts_with("samesite=")
        })
        .collect();
    parts.push(format!("Path={prefix}/"));
    parts.push("SameSite=Strict".to_owned());
    parts.join("; ")
}

pub(crate) async fn root(Path(service): Path<String>) -> Redirect {
    Redirect::permanent(&format!("/tools/{service}/"))
}

/// `ANY /tools/{service}/`: the tool's front page.
pub(crate) async fn index(
    state: State<AppState>,
    who: ToolGateway,
    client: Client,
    Path(service): Path<String>,
    req: Request,
) -> Result<Response, HttpError> {
    gateway(state, who, client, Path((service, String::new())), req).await
}

/// `ANY /tools/{service}/{*rest}`: forward to the tool's loopback port.
pub(crate) async fn gateway(
    State(state): State<AppState>,
    ToolGateway(who): ToolGateway,
    client: Client,
    Path((service, rest)): Path<(String, String)>,
    req: Request,
) -> Result<Response, HttpError> {
    let svc = state
        .store
        .get_service(service.trim())
        .map_err(|_| HttpError::internal())?
        .filter(|s| {
            s.kind == ServiceKind::Tool
                && s.template_id
                    .as_deref()
                    .is_some_and(|t| DATA_TOOLS.contains(&t))
        })
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Data tool not found."))?;
    if !service_writable(&who, &svc) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "You do not have access to this tool.",
        ));
    }
    let port = svc
        .active_host_port
        .filter(|_| svc.active_container.is_some() && !svc.internal_only && !svc.public_port)
        .ok_or_else(|| {
            HttpError::new(
                ErrorCode::Conflict,
                "This tool is not running. Deploy it from Apps first.",
            )
        })?;

    let (parts, body) = req.into_parts();
    if parts.headers.contains_key(UPGRADE) {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "This tool connection type is not supported.",
        ));
    }
    if rest.is_empty() && parts.method == axum::http::Method::GET {
        audit_user(
            &state,
            &who.user.name,
            false,
            Some(&client.ip),
            "database.tool.open",
            Some(&svc.name),
            json!({ "template": svc.template_id }),
        );
    }

    let path_and_query = match parts.uri.query() {
        Some(q) => format!("/{rest}?{q}"),
        None => format!("/{rest}"),
    };
    let mut up = HyperRequest::builder()
        .method(parts.method.clone())
        .uri(path_and_query)
        .version(hyper::Version::HTTP_11);
    {
        let h = up.headers_mut().ok_or_else(HttpError::internal)?;
        for (name, value) in &parts.headers {
            if is_hop(name) || name == AUTHORIZATION || name == ORIGIN || name == REFERER {
                continue;
            }
            if name == COOKIE {
                if let Some(c) = value.to_str().ok().and_then(tool_cookies)
                    && let Ok(v) = HeaderValue::from_str(&c)
                {
                    h.append(COOKIE, v);
                }
                continue;
            }
            h.append(name.clone(), value.clone());
        }
        h.insert(
            HOST,
            HeaderValue::from_str(&format!("127.0.0.1:{port}"))
                .map_err(|_| HttpError::internal())?,
        );
    }
    let up = up
        .body(Limited::new(body, MAX_BODY))
        .map_err(|_| HttpError::internal())?;

    let unavailable = || {
        HttpError::new(
            ErrorCode::Conflict,
            "The tool did not answer. Check that it is running.",
        )
    };
    let stream = tokio::time::timeout(
        Duration::from_secs(5),
        TcpStream::connect(("127.0.0.1", port)),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .map_err(|_| unavailable())?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let resp = tokio::time::timeout(HEADER_TIMEOUT, sender.send_request(up))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;

    let prefix = format!("/tools/{}", svc.name);
    let (rparts, rbody) = resp.into_parts();
    let mut out = Response::new(Body::new(rbody));
    *out.status_mut() = rparts.status;
    let oh: &mut HeaderMap = out.headers_mut();
    for (name, value) in &rparts.headers {
        if is_hop(name) || name == CONTENT_LENGTH && rparts.status == StatusCode::NO_CONTENT {
            continue;
        }
        if name == SET_COOKIE {
            if let Ok(v) = value
                .to_str()
                .map(|v| scope_set_cookie(v, &prefix))
                .map_err(|_| ())
                .and_then(|v| HeaderValue::from_str(&v).map_err(|_| ()))
            {
                oh.append(SET_COOKIE, v);
            }
            continue;
        }
        if name == LOCATION
            && let Ok(l) = value.to_str()
            && l.starts_with('/')
            && !l.starts_with("//")
            && let Ok(v) = HeaderValue::from_str(&format!("{prefix}{l}"))
        {
            oh.insert(LOCATION, v);
            continue;
        }
        oh.append(name.clone(), value.clone());
    }
    Ok(out.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_cookies_never_reach_the_tool() {
        assert_eq!(
            tool_cookies("opsbox_session=abc; opsbox_csrf=def; adminer_sid=1"),
            Some("adminer_sid=1".to_owned())
        );
        assert_eq!(tool_cookies("opsbox_session=abc"), None);
    }

    #[test]
    fn tool_cookies_are_scoped_to_their_path() {
        let v = scope_set_cookie(
            "sid=1; Path=/; Domain=x.test; HttpOnly; SameSite=Lax",
            "/tools/adm",
        );
        assert!(v.contains("Path=/tools/adm/") && v.contains("SameSite=Strict"));
        assert!(!v.contains("Path=/;") && !v.contains("Domain") && !v.contains("Lax"));
    }
}
