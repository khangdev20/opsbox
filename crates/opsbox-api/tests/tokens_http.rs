//! Settings, API tokens and what a token may and may not do. Nothing here changes the machine.

mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_proto::Role;
use serde_json::{Value, json};

async fn send(b: &mut common::Browser, m: Method, p: &str, body: Option<Value>) -> common::Reply {
    b.send(m, p, body, Opts::default()).await
}

/// A request with only an `Authorization` header and no cookies, like a script's.
async fn with_token(
    h: &Harness,
    token: &str,
    m: Method,
    p: &str,
    body: Option<Value>,
) -> common::Reply {
    let mut script = h.browser();
    script
        .send(
            m,
            p,
            body,
            Opts {
                csrf: false,
                origin: None,
                extra: vec![("authorization".into(), format!("Bearer {token}"))],
                ..Opts::default()
            },
        )
        .await
}

#[tokio::test]
async fn settings_and_token_management_are_for_admins_signed_in_as_people() {
    let h = Harness::new(true);
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/settings").await.status,
        StatusCode::UNAUTHORIZED
    );
    for role in [Role::Viewer, Role::Deployer] {
        let mut b = h.signed_in(&format!("{role:?}"), role);
        assert_eq!(
            b.get("/api/v1/settings").await.status,
            StatusCode::FORBIDDEN
        );
        let r = send(
            &mut b,
            Method::POST,
            "/api/v1/tokens",
            Some(json!({"name": "abc", "scope": "read"})),
        )
        .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
        assert_eq!(
            send(&mut b, Method::DELETE, "/api/v1/tokens/x", None)
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }
    let mut a = h.signed_in("boss", Role::Admin);
    let v = a.get("/api/v1/settings").await;
    assert_eq!(v.status, StatusCode::OK, "{}", v.text());
    let v = v.json();
    assert!(v["hostname"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(v["tz"].is_string() && v["timezones"].as_array().is_some_and(|z| !z.is_empty()));
    assert!(v["swapMb"].is_number() && v["tokens"] == json!([]));
    h.shutdown().await;
}

#[tokio::test]
async fn a_token_is_created_shown_once_used_and_revoked() {
    let h = Harness::new(true);
    let mut a = h.signed_in("boss", Role::Admin);
    for (body, word) in [
        (json!({"name": "x", "scope": "read"}), "3 to 32"),
        (
            json!({"name": "good-name", "scope": "root"}),
            "read, deploy or full",
        ),
    ] {
        let r = send(&mut a, Method::POST, "/api/v1/tokens", Some(body)).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text());
        assert!(r.text().contains(word), "{word}: {}", r.text());
    }
    let r = send(
        &mut a,
        Method::POST,
        "/api/v1/tokens",
        Some(json!({"name": "uptime-check", "scope": "read"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let created = r.json();
    let secret = created["secret"].as_str().unwrap().to_owned();
    assert!(secret.starts_with("ops_"));
    let id = created["token"]["id"].as_str().unwrap().to_owned();
    // The listing never contains the secret.
    let listing = a.get("/api/v1/settings").await.text();
    assert!(listing.contains("uptime-check") && !listing.contains(&secret));
    assert_eq!(
        send(
            &mut a,
            Method::POST,
            "/api/v1/tokens",
            Some(json!({"name": "UPTIME-CHECK", "scope": "read"}))
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );

    // A read token can read, nothing else.
    assert_eq!(
        with_token(&h, &secret, Method::GET, "/api/v1/attention", None)
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        with_token(&h, &secret, Method::GET, "/api/v1/host", None)
            .await
            .status,
        StatusCode::OK
    );
    let w = with_token(&h, &secret, Method::POST, "/api/v1/system/reboot", None).await;
    assert_eq!(w.status, StatusCode::FORBIDDEN, "{}", w.text());
    assert_eq!(
        with_token(&h, &secret, Method::GET, "/api/v1/settings", None)
            .await
            .status,
        StatusCode::FORBIDDEN,
        "viewer role"
    );
    // Its use shows in the list.
    assert!(a.get("/api/v1/settings").await.json()["tokens"][0]["last"] != "never");

    // Bad and malformed tokens are 401, not 500.
    for bad in ["ops_wrong", "nonsense", "", "ops_"] {
        assert_eq!(
            with_token(&h, bad, Method::GET, "/api/v1/host", None)
                .await
                .status,
            StatusCode::UNAUTHORIZED,
            "{bad:?}"
        );
    }
    // A cookie session wins over a header, and a token is not a cookie.
    let mut spoof = h.browser();
    spoof.jar.insert("opsbox_session".into(), secret.clone());
    assert_eq!(
        spoof.get("/api/v1/host").await.status,
        StatusCode::UNAUTHORIZED
    );

    // Revoking works at once.
    assert_eq!(
        send(
            &mut a,
            Method::DELETE,
            &format!("/api/v1/tokens/{id}"),
            None
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        with_token(&h, &secret, Method::GET, "/api/v1/host", None)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(
            &mut a,
            Method::DELETE,
            &format!("/api/v1/tokens/{id}"),
            None
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    let actions: Vec<_> = h
        .store
        .recent_audit(30)
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    assert!(
        actions.iter().any(|x| x == "token.create") && actions.iter().any(|x| x == "token.revoke")
    );
    h.shutdown().await;
}

#[tokio::test]
async fn even_a_full_access_token_cannot_manage_people_keys_terminals_or_tokens() {
    let h = Harness::new(true);
    let mut a = h.signed_in("boss", Role::Admin);
    let full = send(
        &mut a,
        Method::POST,
        "/api/v1/tokens",
        Some(json!({"name": "everything", "scope": "full"})),
    )
    .await
    .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    let deploy = send(
        &mut a,
        Method::POST,
        "/api/v1/tokens",
        Some(json!({"name": "deployer", "scope": "deploy"})),
    )
    .await
    .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    // What a full token may do: administrator reads.
    for p in [
        "/api/v1/security",
        "/api/v1/access",
        "/api/v1/cleanup/scan",
        "/api/v1/settings",
    ] {
        let r = with_token(&h, &full, Method::GET, p, None).await;
        assert!(
            r.status == StatusCode::OK || r.status == StatusCode::FORBIDDEN,
            "{p}"
        );
    }
    assert_eq!(
        with_token(&h, &full, Method::GET, "/api/v1/security", None)
            .await
            .status,
        StatusCode::OK
    );
    // What no token may do, whatever its scope.
    for (m, p, body) in [
        (
            Method::POST,
            "/api/v1/tokens",
            Some(json!({"name": "another", "scope": "read"})),
        ),
        (Method::DELETE, "/api/v1/tokens/whatever", None),
        (
            Method::POST,
            "/api/v1/users/invites",
            Some(json!({"name": "dana", "role": "viewer"})),
        ),
        (Method::DELETE, "/api/v1/users/boss", None),
        (
            Method::POST,
            "/api/v1/access/keys",
            Some(json!({"user": "x", "key": "ssh-ed25519 AAAA"})),
        ),
        (
            Method::POST,
            "/api/v1/access/keys/remove",
            Some(json!({"id": "x:SHA256:y"})),
        ),
        (
            Method::POST,
            "/api/v1/access/users/x/lock",
            Some(json!({"lock": true})),
        ),
        (
            Method::PUT,
            "/api/v1/security/allow-ips",
            Some(json!({"value": ""})),
        ),
        (Method::GET, "/api/v1/terminal/users", None),
        (Method::POST, "/api/v1/auth/logout", None),
        (Method::POST, "/api/v1/auth/sessions/revoke-all", None),
    ] {
        for t in [&full, &deploy] {
            let r = with_token(&h, t, m.clone(), p, body.clone()).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{m} {p}: {}", r.text());
        }
    }
    // The people-and-keys refusals are explained as such for a token that has the role.
    let r = with_token(
        &h,
        &full,
        Method::POST,
        "/api/v1/users/invites",
        Some(json!({"name": "dana", "role": "viewer"})),
    )
    .await;
    assert!(r.text().contains("tokens cannot do this"), "{}", r.text());
    assert!(h.store.user_by_name("dana").unwrap().is_none());
    // A deployer-scope token cannot reach administrator reads.
    assert_eq!(
        with_token(&h, &deploy, Method::GET, "/api/v1/access", None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    // A deploy scope cannot inherit a deployer account's file access. These use
    // valid paths so a failure cannot be mistaken for path validation.
    let path = h.root.join("apps/token-probe.txt");
    std::fs::write(&path, "unchanged").unwrap();
    for (method, endpoint, body) in [
        (
            Method::GET,
            format!("/api/v1/files?path={}", h.root.join("apps").display()),
            None,
        ),
        (
            Method::PUT,
            "/api/v1/files/content".into(),
            Some(json!({"path": path, "content": "changed"})),
        ),
    ] {
        let r = with_token(&h, &deploy, method, &endpoint, body).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    }
    assert_eq!(std::fs::read_to_string(path).unwrap(), "unchanged");
    h.shutdown().await;
}
