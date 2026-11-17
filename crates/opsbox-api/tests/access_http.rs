//! The Access endpoints. Nothing here changes the machine: every request used either only
//! reads or is refused before anything is written.

mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_proto::Role;
use serde_json::{Value, json};

const ED: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILccwtaElMJNNkowDtiI0r07uTtMUL77z/R1AI9LiDmm khang@laptop";

async fn send(b: &mut common::Browser, m: Method, p: &str, body: Option<Value>) -> common::Reply {
    b.send(m, p, body, Opts::default()).await
}

#[tokio::test]
async fn only_admins_see_or_change_access() {
    let h = Harness::new(true);
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/access").await.status,
        StatusCode::UNAUTHORIZED
    );
    for role in [Role::Viewer, Role::Deployer] {
        let mut b = h.signed_in(&format!("{role:?}"), role);
        assert_eq!(
            b.get("/api/v1/access").await.status,
            StatusCode::FORBIDDEN,
            "{role:?}"
        );
        for (m, p, body) in [
            (
                Method::POST,
                "/api/v1/access/keys",
                json!({"user": "x", "key": ED}),
            ),
            (
                Method::POST,
                "/api/v1/access/keys/remove",
                json!({"id": "x:SHA256:y"}),
            ),
            (
                Method::POST,
                "/api/v1/access/users/x/lock",
                json!({"lock": true}),
            ),
        ] {
            let r = send(&mut b, m, p, Some(body)).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{role:?} {p}");
        }
    }
    h.shutdown().await;
}

#[tokio::test]
async fn the_listing_has_the_shape_the_page_draws() {
    let h = Harness::new(true);
    let mut a = h.signed_in("boss", Role::Admin);
    let r = a.get("/api/v1/access").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    assert!(v["users"].is_array() && v["keys"].is_array() && v["out"] == json!([]));
    assert!(v["sshOpen"].is_boolean() && v["ssh"]["port"].is_number());
    for u in v["users"].as_array().unwrap() {
        for k in ["name", "groups", "sudo", "last", "locked", "note"] {
            assert!(!u[k].is_null(), "{k} in {u}");
        }
    }
    for k in v["keys"].as_array().unwrap() {
        assert!(k["id"].as_str().unwrap().contains(":SHA256:"), "{k}");
    }
    h.shutdown().await;
}

#[tokio::test]
async fn bad_keys_and_choices_are_explained_and_change_nothing() {
    let h = Harness::new(true);
    let mut a = h.signed_in("boss", Role::Admin);
    for (key, word) in [
        ("-----BEGIN OPENSSH PRIVATE KEY-----\nxxx", "private key"),
        ("hello world", "not a public key"),
        ("ssh-rsa AAAA nope", "not a public key"),
    ] {
        let r = send(
            &mut a,
            Method::POST,
            "/api/v1/access/keys",
            Some(json!({"user": "whoever", "key": key})),
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{key}: {}", r.text());
        assert_eq!(r.error_code(), "validation");
        assert!(
            r.text().to_lowercase().contains(word),
            "{key}: {}",
            r.text()
        );
    }
    // A valid key for a user who does not exist.
    let r = send(
        &mut a,
        Method::POST,
        "/api/v1/access/keys",
        Some(json!({"user": "no-such-person", "key": ED})),
    )
    .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND, "{}", r.text());
    // A hostile user name never gets near a command.
    let r = send(
        &mut a,
        Method::POST,
        "/api/v1/access/keys",
        Some(json!({"user": "a;b", "key": ED})),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = send(
        &mut a,
        Method::POST,
        "/api/v1/access/keys/remove",
        Some(json!({"id": "no-colon-here"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    // Root is never locked from here.
    let r = send(
        &mut a,
        Method::POST,
        "/api/v1/access/users/root/lock",
        Some(json!({"lock": true})),
    )
    .await;
    if r.status != StatusCode::NOT_FOUND {
        // (Where root has no login shell it is not listed at all.)
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
        assert!(r.text().contains("root"), "{}", r.text());
    }
    let audited = h.store.recent_audit(50).unwrap();
    assert!(
        audited.iter().all(|e| !e.action.starts_with("access.")),
        "nothing was changed"
    );
    h.shutdown().await;
}
