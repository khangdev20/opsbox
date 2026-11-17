//! The Cleanup endpoints. They never delete anything on the machine running the tests:
//! scanning only reads, and the runs used here fail validation before any scanner starts.

mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_proto::Role;
use serde_json::{Value, json};

#[tokio::test]
async fn only_admins_use_cleanup() {
    let h = Harness::new(true);
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/cleanup/scan").await.status,
        StatusCode::UNAUTHORIZED
    );
    for role in [Role::Viewer, Role::Deployer] {
        let mut b = h.signed_in(&format!("{role:?}"), role);
        assert_eq!(
            b.get("/api/v1/cleanup/scan").await.status,
            StatusCode::FORBIDDEN
        );
        for (m, p, body) in [
            (
                Method::POST,
                "/api/v1/cleanup/run",
                Some(json!({"ids": ["tmp"]})),
            ),
            (
                Method::PUT,
                "/api/v1/cleanup/auto",
                Some(json!({"on": true})),
            ),
        ] {
            let r = b.send(m, p, body, Opts::default()).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{role:?} {p}");
        }
    }
    // Changes need the CSRF header, too.
    let mut a = h.signed_in("boss", Role::Admin);
    let r = a
        .send(
            Method::POST,
            "/api/v1/cleanup/run",
            Some(json!({"ids": ["tmp"]})),
            Opts::bare(),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    h.shutdown().await;
}

#[tokio::test]
async fn a_scan_has_the_shape_the_page_draws() {
    let h = Harness::new(true);
    let mut a = h.signed_in("boss", Role::Admin);
    let r = a.get("/api/v1/cleanup/scan").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    assert!(v["items"].is_array() && v["system"].is_number() && v["auto"].is_boolean());
    assert!(v["scanned"].as_str().is_some_and(|s| s.contains(':')));
    for i in v["items"].as_array().unwrap() {
        for key in ["id", "title", "desc", "bytes", "gb", "on", "warn", "color"] {
            assert!(!i[key].is_null(), "{key} in {i}");
        }
        // Risky things are never selected by default.
        if i["warn"] == true {
            assert_eq!(i["on"], false, "{i}");
        }
    }
    h.shutdown().await;
}

async fn run_task(a: &mut common::Browser, ids: Value) -> Value {
    let r = a
        .send(
            Method::POST,
            "/api/v1/cleanup/run",
            Some(json!({ "ids": ids })),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
    let id = r.json()["task"].as_str().unwrap().to_owned();
    let mut task = Value::Null;
    for _ in 0..100 {
        task = a.get(&format!("/api/v1/tasks/{id}")).await.json();
        if task["status"] != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    task
}

#[tokio::test]
async fn unknown_things_cannot_be_cleaned_and_nothing_runs() {
    let h = Harness::new(true);
    let mut a = h.signed_in("boss", Role::Admin);
    let r = a
        .send(
            Method::POST,
            "/api/v1/cleanup/run",
            Some(json!({"ids": []})),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let task = run_task(&mut a, json!(["tmp", "../../etc"])).await;
    assert_eq!(task["status"], "failed", "{task}");
    assert!(
        task["error"].as_str().unwrap().contains("../../etc"),
        "{task}"
    );
    // A refused request is not audited as a cleanup.
    let audited = h.store.recent_audit(20).unwrap();
    assert!(audited.iter().all(|e| e.action != "cleanup.run"));
    h.shutdown().await;
}
