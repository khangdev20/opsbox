//! Real router tests for limits and timeout errors. No host mutations.
mod common;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use common::{Harness, Opts};
use http_body_util::BodyExt;
use opsbox_proto::Role;
use serde_json::{Value, json};
use tower::ServiceExt;

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
    assert_eq!(r.header("x-content-type-options"), "nosniff");
    assert_eq!(r.header("x-frame-options"), "DENY");
    assert!(
        r.header("content-security-policy")
            .contains("frame-ancestors 'none'")
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
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["error"]["code"], "timeout");
}
