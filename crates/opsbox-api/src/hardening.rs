//! Default security headers on every response from `serve`.
//!
//! One middleware owns Cache-Control (API + HTML), CSP (panel vs `/tools/…`),
//! HSTS (HTTPS on a real domain only), and the other browser-hardening headers.
//! Early limit errors (413/408) are rewritten into the API error shape here too.

use std::net::IpAddr;

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use opsbox_proto::ErrorCode;

use crate::transport::Transport;
use crate::HttpError;

/// Panel UI and API: scripts from this origin only; inline styles remain (lit/html).
pub(crate) const PANEL_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'";

/// pgweb / Adminer under `/tools/…` only: their UIs need inline script and eval.
pub(crate) const TOOLS_CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'";

const HSTS: &str = "max-age=31536000";
const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=()";

pub(crate) async fn middleware(req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let https = req
        .extensions()
        .get::<Transport>()
        .is_some_and(|t| t.https);
    let api = path.starts_with("/api/");
    let tools = path == "/tools" || path.starts_with("/tools/");

    let mut response = next.run(req).await;
    if api {
        let code = match response.status() {
            StatusCode::PAYLOAD_TOO_LARGE => Some((
                ErrorCode::TooLarge,
                "That request is larger than this endpoint accepts.",
            )),
            StatusCode::REQUEST_TIMEOUT => Some((
                ErrorCode::Timeout,
                "This request took too long. Check its result before trying again.",
            )),
            _ => None,
        };
        if let Some((code, message)) = code {
            response = HttpError::new(code, message).into_response();
        }
    }

    apply_headers(response.headers_mut(), Apply {
        path: &path,
        host: &host,
        https,
        api,
        tools,
    });
    response
}

struct Apply<'a> {
    path: &'a str,
    host: &'a str,
    https: bool,
    api: bool,
    tools: bool,
}

fn apply_headers(h: &mut axum::http::HeaderMap, a: Apply<'_>) {
    // Never advertise a framework version.
    h.remove(header::SERVER);

    if a.api || wants_html_no_store(a.path, h) {
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("no-store"),
        );
    }

    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    h.insert(
        "permissions-policy",
        HeaderValue::from_static(PERMISSIONS_POLICY),
    );
    h.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(if a.tools { TOOLS_CSP } else { PANEL_CSP }),
    );

    if should_send_hsts(a.https, a.host) {
        h.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(HSTS),
        );
    } else {
        h.remove(header::STRICT_TRANSPORT_SECURITY);
    }
}

/// HTML documents must not be stored by shared caches (SPA index, `.html`, 304s).
fn wants_html_no_store(path: &str, headers: &axum::http::HeaderMap) -> bool {
    if let Some(ct) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        && ct
            .split(';')
            .next()
            .is_some_and(|t| t.trim().eq_ignore_ascii_case("text/html"))
    {
        return true;
    }
    path == "/"
        || path == "/kit"
        || path.ends_with('/')
        || path.ends_with(".html")
}

/// HSTS only over HTTPS on a real DNS name. Self-signed IP mode and loopback HTTP
/// must not pin the browser: a later certificate change would lock people out.
pub(crate) fn should_send_hsts(https: bool, host_header: &str) -> bool {
    https && host_is_real_domain(host_header)
}

/// `Host` value without the port, if any (`panel.example.com:443`, `[::1]:8443`).
fn host_without_port(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => host,
    }
}

fn host_is_real_domain(host_header: &str) -> bool {
    let host = host_without_port(host_header.trim());
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return false;
    }
    if host.parse::<IpAddr>().is_ok() {
        return false;
    }
    // Need at least one dot so single-label names (and bare hostnames) stay out.
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty()
                && l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn hsts_only_on_https_with_a_real_domain() {
        assert!(should_send_hsts(true, "panel.example.com"));
        assert!(should_send_hsts(true, "panel.example.com:443"));
        assert!(!should_send_hsts(false, "panel.example.com"));
        assert!(!should_send_hsts(true, "203.0.113.7"));
        assert!(!should_send_hsts(true, "203.0.113.7:8443"));
        assert!(!should_send_hsts(true, "127.0.0.1:8443"));
        assert!(!should_send_hsts(true, "localhost"));
        assert!(!should_send_hsts(true, "[::1]:8443"));
        assert!(!should_send_hsts(true, ""));
    }

    #[test]
    fn apply_sets_panel_headers_and_api_no_store() {
        let mut h = HeaderMap::new();
        apply_headers(
            &mut h,
            Apply {
                path: "/api/v1/session",
                host: "127.0.0.1:8443",
                https: false,
                api: true,
                tools: false,
            },
        );
        assert_eq!(h.get("cache-control").unwrap(), "no-store");
        assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(h.get("x-frame-options").unwrap(), "DENY");
        assert_eq!(h.get("referrer-policy").unwrap(), "same-origin");
        assert_eq!(
            h.get("permissions-policy").unwrap(),
            "camera=(), microphone=(), geolocation=()"
        );
        assert_eq!(
            h.get("cross-origin-opener-policy").unwrap(),
            "same-origin"
        );
        assert_eq!(h.get("content-security-policy").unwrap(), PANEL_CSP);
        assert!(h.get("strict-transport-security").is_none());
        assert!(h.get(header::SERVER).is_none());
    }

    #[test]
    fn tools_prefix_gets_the_looser_csp_only() {
        let mut h = HeaderMap::new();
        apply_headers(
            &mut h,
            Apply {
                path: "/tools/pgweb/",
                host: "panel.example.com",
                https: true,
                api: false,
                tools: true,
            },
        );
        assert_eq!(h.get("content-security-policy").unwrap(), TOOLS_CSP);
        assert_eq!(
            h.get("strict-transport-security").unwrap(),
            "max-age=31536000"
        );
    }

    #[test]
    fn html_gets_no_store_even_without_content_type() {
        let mut h = HeaderMap::new();
        apply_headers(
            &mut h,
            Apply {
                path: "/",
                host: "127.0.0.1:8443",
                https: false,
                api: false,
                tools: false,
            },
        );
        assert_eq!(h.get("cache-control").unwrap(), "no-store");
    }
}
