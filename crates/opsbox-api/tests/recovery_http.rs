//! HTTP tests for `/api/v1/recovery` role enforcement and payload shape.

mod common;

use axum::http::StatusCode;
use common::Harness;
use opsbox_proto::Role;

#[tokio::test]
async fn recovery_requires_admin_and_returns_watchdog_snapshot() {
    let h = Harness::new(true);

    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/recovery").await.status,
        StatusCode::UNAUTHORIZED
    );

    for role in [Role::Viewer, Role::Deployer] {
        let mut b = h.signed_in(&format!("{role:?}"), role);
        assert_eq!(
            b.get("/api/v1/recovery").await.status,
            StatusCode::FORBIDDEN
        );
    }

    let mut admin = h.signed_in("boss", Role::Admin);
    let r = admin.get("/api/v1/recovery").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    assert!(v["running"].is_boolean());
    assert!(v["failed"].is_boolean());
    assert!(v["canRollback"].is_boolean());
    assert!(v["checks"].is_array());
    assert!(v["log"].is_array());

    h.shutdown().await;
}
