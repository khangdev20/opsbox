//! Security headers on every response: API, UI, errors, 404s, and conditional HSTS.
//! No host mutations.
mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use common::{Harness, Opts};
use http_body_util::BodyExt;
use opsbox_api::transport::PROXY_SECRET_HEADER;
use opsbox_proto::Role;
use serde_json::{Value, json};
use tower::ServiceExt;

const PROXY_SECRET: &str = "test-proxy-secret-for-hardening-hdrs!";

const REQUIRED: &[(&str, &str)] = &[
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
    ("referrer-policy", "same-origin"),
    (
        "permissions-policy",
        "camera=(), microphone=(), geolocation=()",
    ),
    ("cross-origin-opener-policy", "same-origin"),
];

fn assert_required(r: &common::Reply, path: &str) {
    for &(name, want) in REQUIRED {
        assert_eq!(
            r.header(name),
            want,
            "{path}: {name} want {want:?}, got {:?}",
            r.header(name)
        );
    }
    let csp = r.header("content-security-policy");
    assert!(
        csp.contains("frame-ancestors 'none'")
            && csp.contains("default-src 'self'")
            && csp.contains("object-src 'none'"),
        "{path}: CSP incomplete: {csp}"
    );
    assert!(
        r.header("server").is_empty(),
        "{path}: Server header must be absent"
    );
    if r.status.is_client_error() || r.status.is_server_error() {
        let body = r.text();
        assert!(
            !body.contains("stack backtrace")
                && !body.contains("panicked at")
                && !body.contains("/Users/")
                && !body.contains("/home/"),
            "{path}: error body must not leak internals: {body}"
        );
    }
}

/// Representative paths across the router (health, API, UI, tools prefix, 404s).
const ROUTE_TABLE: &[(&str, Method)] = &[
    ("/healthz", Method::GET),
    ("/readyz", Method::GET),
    ("/api/v1/session", Method::GET),
    ("/api/v1/attention", Method::GET),
    ("/api/v1/no-such-route-16r", Method::GET),
    ("/", Method::GET),
    ("/index.html", Method::GET),
    ("/js/api.js", Method::GET),
    ("/definitely-missing-16r", Method::GET),
    ("/tools/pgweb/", Method::GET),
];

#[tokio::test]
async fn every_route_returns_the_required_security_headers() {
    let h = Harness::new(false);
    let mut b = h.signed_in("hdr-reader", Role::Viewer);
    for &(path, ref method) in ROUTE_TABLE {
        let r = b.send(method.clone(), path, None, Opts::bare()).await;
        assert_required(&r, path);
        if path.starts_with("/api/") {
            assert_eq!(
                r.header("cache-control"),
                "no-store",
                "{path} must be no-store"
            );
        }
        if path == "/" || path.ends_with(".html") {
            assert_eq!(
                r.header("cache-control"),
                "no-store",
                "{path} HTML must be no-store"
            );
        }
        if path.starts_with("/tools/") {
            assert!(
                r.header("content-security-policy").contains("unsafe-eval"),
                "{path}: tools CSP should allow eval for vendor UIs"
            );
        } else {
            assert!(
                !r.header("content-security-policy").contains("unsafe-eval"),
                "{path}: panel CSP must not allow eval"
            );
        }
        assert!(
            r.header("strict-transport-security").is_empty(),
            "{path}: loopback HTTP must not send HSTS"
        );
    }
}

#[tokio::test]
async fn not_found_and_method_errors_still_carry_security_headers() {
    let h = Harness::new(false);
    let mut b = h.browser();
    let missing = b
        .send(
            Method::GET,
            "/api/v1/does-not-exist-16r",
            None,
            Opts::bare(),
        )
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_required(&missing, "api 404");
    assert_eq!(missing.header("cache-control"), "no-store");

    let ui_missing = b
        .send(Method::GET, "/no-page-here-16r", None, Opts::bare())
        .await;
    assert_eq!(ui_missing.status, StatusCode::NOT_FOUND);
    assert_required(&ui_missing, "ui 404");

    let wrong = b.send(Method::DELETE, "/healthz", None, Opts::bare()).await;
    assert_eq!(wrong.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_required(&wrong, "405");
}

#[tokio::test]
async fn hsts_present_only_on_https_with_a_real_domain() {
    // Trusted proxy + HTTPS + DNS Host → HSTS.
    let h = Harness::new_with(false, |s| s.with_proxy_secret(PROXY_SECRET));
    let mut b = h.browser();
    let mut opts = Opts::bare();
    opts.host = "panel.example.com".into();
    opts.peer = Some("127.0.0.1:9".parse().unwrap());
    opts.extra = vec![
        (PROXY_SECRET_HEADER.into(), PROXY_SECRET.into()),
        ("x-forwarded-proto".into(), "https".into()),
        ("x-forwarded-for".into(), "203.0.113.50".into()),
    ];
    let r = b.send(Method::GET, "/api/v1/session", None, opts).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        r.header("strict-transport-security"),
        "max-age=31536000",
        "panel domain over HTTPS must send HSTS"
    );
    assert_required(&r, "hsts domain");
    assert_eq!(r.header("cache-control"), "no-store");

    // Self-signed IP mode (TLS listener, Host is an IP) → no HSTS.
    let h_ip = Harness::new_with(false, |s| s.with_tls(true));
    let mut b_ip = h_ip.browser();
    let mut opts_ip = Opts::bare();
    opts_ip.host = "203.0.113.7:8443".into();
    let r_ip = b_ip
        .send(Method::GET, "/api/v1/session", None, opts_ip)
        .await;
    assert!(
        r_ip.header("strict-transport-security").is_empty(),
        "self-signed IP must not send HSTS: got {:?}",
        r_ip.header("strict-transport-security")
    );

    // Loopback HTTP → no HSTS (covered above; pin explicitly).
    let h_local = Harness::new(false);
    let r_local = h_local.browser().get("/api/v1/session").await;
    assert!(r_local.header("strict-transport-security").is_empty());
}

#[tokio::test]
async fn auth_is_throttled_before_body_parsing_and_health_remains_available() {
    let h = Harness::new(false);
    let mut b = h.browser();
    for _ in 0..20 {
        assert_eq!(
            b.post("/api/v1/auth/login", json!({})).await.status,
            StatusCode::BAD_REQUEST
        );
    }
    let r = b.post("/api/v1/setup/claim", json!({})).await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.error_code(), "rate_limited");
    assert!(r.header("retry-after").parse::<u64>().unwrap() >= 1);
    assert!(!r.header("x-request-id").is_empty());
    assert_eq!(r.header("cache-control"), "no-store");
    assert_required(&r, "rate limited");
    assert_eq!(b.get("/healthz").await.status, StatusCode::OK);
    assert_eq!(
        b.get("/api/v1/session").await.status,
        StatusCode::OK,
        "auth has a separate, stricter bucket"
    );
}

#[tokio::test]
async fn ordinary_ip_budget_cannot_be_bypassed_by_rotating_cookies() {
    let h = Harness::new(false);
    let mut b = h.browser();
    for i in 0..120 {
        b.jar
            .insert("opsbox_session".into(), format!("invented-{i}"));
        assert_eq!(
            b.get("/api/v1/attention").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let r = b.get("/api/v1/attention").await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(r.error_code(), "rate_limited");
}

#[tokio::test]
async fn session_budget_is_shared_across_addresses() {
    let h = Harness::new(false);
    let b = h.signed_in("reader", Role::Viewer);
    let cookie = format!("opsbox_session={}", b.jar["opsbox_session"]);
    let mut last = StatusCode::OK;
    for i in 0..121 {
        let req = Request::get("/api/v1/attention")
            .header("cookie", &cookie)
            .extension(ConnectInfo(
                format!("192.0.2.{}:1234", i + 1)
                    .parse::<std::net::SocketAddr>()
                    .unwrap(),
            ))
            .body(Body::empty())
            .unwrap();
        last = h.app.clone().oneshot(req).await.unwrap().status();
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        h.signed_in("other-reader", Role::Viewer)
            .get("/api/v1/attention")
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn oversized_requests_get_json_errors_and_security_headers() {
    let h = Harness::new(false);
    let mut b = h.browser();
    let r = b
        .send(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({"username":"x".repeat(1024*1024)})),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.error_code(), "too_large");
    assert_eq!(r.header("cache-control"), "no-store");
    assert_required(&r, "413");
    assert!(
        r.header("content-security-policy")
            .contains("default-src 'self'")
    );
}

#[tokio::test(start_paused = true)]
async fn a_stalled_body_times_out_in_the_api_format() {
    let h = Harness::new(false);
    let req = Request::post("/api/v1/auth/login")
        .header("content-type", "application/json")
        .body(Body::from_stream(futures_util::stream::pending::<
            Result<axum::body::Bytes, std::io::Error>,
        >()))
        .unwrap();
    let response = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    let headers = response.headers().clone();
    assert_eq!(
        headers
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["error"]["code"], "timeout");
}
