//! Authentication over HTTP: setup, sign-in, cookies, CSRF, Origin, roles, lockout.

mod common;

use axum::Router;
use axum::http::{Method, StatusCode};
use axum::routing::get;
use common::{Browser, HOST, Harness, ORIGIN, Opts, PASSWORD};
use opsbox_api::session::{Admin, Deployer, Viewer};
use opsbox_proto::Role;
use serde_json::json;

use std::time::{SystemTime, UNIX_EPOCH};

fn decode_base32(s: &str) -> Vec<u8> {
    let (mut buf, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("not base32"),
        };
        buf = (buf << 5) | u32::from(v);
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
            buf &= (1 << bits) - 1;
        }
    }
    out
}

fn code_for(secret: &str, ahead: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    opsbox_priv::code_at(&decode_base32(secret), now / 30 + ahead)
}

/// Run the whole first-run flow through the HTTP endpoints; returns the browser
/// (signed in as admin) and the TOTP secret.
async fn setup(h: &Harness) -> (Browser, String) {
    let token = h.state.auth.issue_setup_token().unwrap();
    let mut b = h.browser();
    assert_eq!(
        b.post("/api/v1/setup/claim", json!({ "token": token }))
            .await
            .status,
        StatusCode::OK
    );
    let r = b
        .post(
            "/api/v1/setup/admin",
            json!({ "username": "admin", "password": PASSWORD }),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let secret = r.json()["secret"].as_str().unwrap().to_owned();
    let r = b
        .post(
            "/api/v1/setup/totp/confirm",
            json!({ "code": code_for(&secret, 0) }),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    (b, secret)
}

// ------------------------------------------------------------------ public vs protected

#[tokio::test]
async fn protected_endpoints_refuse_anonymous_callers_in_the_spec_format() {
    let h = Harness::new(true);
    let mut b = h.browser();
    for path in ["/api/v1/host"] {
        let r = b.get(path).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{path}");
        assert_eq!(r.error_code(), "unauthorized");
        assert_eq!(r.json()["error"]["message"], "Sign in to continue.");
    }
    // A made-up or malformed cookie is no better.
    b.jar.insert("opsbox_session".into(), "forged".into());
    assert_eq!(b.get("/api/v1/host").await.status, StatusCode::UNAUTHORIZED);
    // Health endpoints stay public.
    assert_eq!(b.get("/healthz").await.status, StatusCode::OK);
    h.shutdown().await;
}

#[tokio::test]
async fn api_responses_are_never_cached() {
    let h = Harness::new(true);
    let mut b = h.browser();
    for path in [
        "/api/v1/session",
        "/api/v1/host",
        "/healthz",
        "/api/v1/nope",
    ] {
        assert_eq!(
            b.get(path).await.header("cache-control"),
            "no-store",
            "{path}"
        );
    }
}

#[tokio::test]
async fn session_endpoint_tells_the_page_what_to_show() {
    let h = Harness::new(true);
    let mut b = h.browser();
    let v = b.get("/api/v1/session").await.json();
    assert_eq!(
        v,
        json!({"authenticated": false, "enrollRequired": false, "setupRequired": true})
    );

    let (mut admin, _) = setup(&h).await;
    let v = admin.get("/api/v1/session").await.json();
    assert_eq!(
        (
            v["authenticated"].clone(),
            v["user"].clone(),
            v["role"].clone(),
            v["setupRequired"].clone()
        ),
        (json!(true), json!("admin"), json!("admin"), json!(false))
    );
    let v = b.get("/api/v1/session").await.json();
    assert_eq!(v["authenticated"], false);
    assert_eq!(
        v["setupRequired"], false,
        "anonymous visitors learn that setup is over"
    );
}

// ------------------------------------------------------------------ first-run setup

#[tokio::test]
async fn setup_over_http_ends_with_a_signed_in_admin_and_closed_endpoints() {
    let h = Harness::new(true);
    let token = h.state.auth.issue_setup_token().unwrap();
    let mut b = h.browser();

    // Before claiming, the later steps have nothing to work with.
    let r = b
        .post(
            "/api/v1/setup/admin",
            json!({"username": "admin", "password": PASSWORD}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json()["error"]["field"], "token");

    let r = b
        .post("/api/v1/setup/claim", json!({"token": "AAAA-AAAA-AAAA"}))
        .await;
    assert_eq!(
        (r.status, r.json()["error"]["field"].clone()),
        (StatusCode::BAD_REQUEST, json!("token"))
    );

    let r = b.post("/api/v1/setup/claim", json!({"token": token})).await;
    assert_eq!(r.status, StatusCode::OK);
    let setup_cookie = b.set_cookies.last().unwrap().clone();
    assert!(setup_cookie.starts_with("opsbox_setup="), "{setup_cookie}");
    assert!(
        setup_cookie.contains("HttpOnly") && setup_cookie.contains("SameSite=Strict"),
        "{setup_cookie}"
    );
    assert!(
        setup_cookie.contains("Path=/api/v1/setup"),
        "the setup cookie goes nowhere else: {setup_cookie}"
    );

    let r = b
        .post(
            "/api/v1/setup/admin",
            json!({"username": "admin", "password": "short"}),
        )
        .await;
    assert_eq!(
        (r.status, r.json()["error"]["field"].clone()),
        (StatusCode::BAD_REQUEST, json!("password"))
    );

    let r = b
        .post(
            "/api/v1/setup/admin",
            json!({"username": "Admin", "password": PASSWORD}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    let uri = v["otpauthUri"].as_str().unwrap();
    assert!(
        uri.starts_with("otpauth://totp/opsbox%20") && uri.contains(":admin?secret="),
        "{uri}"
    );
    assert_eq!(v["account"], "admin");
    let secret = v["secret"].as_str().unwrap().to_owned();

    let r = b
        .post("/api/v1/setup/totp/confirm", json!({"code": "000000"}))
        .await;
    assert_eq!(
        (r.status, r.json()["error"]["field"].clone()),
        (StatusCode::BAD_REQUEST, json!("code"))
    );

    let r = b
        .post(
            "/api/v1/setup/totp/confirm",
            json!({"code": code_for(&secret, 0)}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["role"], "admin");
    assert!(b.jar.contains_key("opsbox_session") && b.jar.contains_key("opsbox_csrf"));
    assert_eq!(
        b.get("/api/v1/host").await.status,
        StatusCode::OK,
        "setup signs the admin in"
    );

    // From now on the setup endpoints do not exist.
    for (path, body) in [
        ("/api/v1/setup/claim", json!({"token": token})),
        (
            "/api/v1/setup/admin",
            json!({"username": "evil", "password": PASSWORD}),
        ),
        ("/api/v1/setup/totp/confirm", json!({"code": "123456"})),
    ] {
        let r = b.post(path, body.clone()).await;
        assert_eq!(
            (r.status, r.error_code()),
            (StatusCode::NOT_FOUND, "not_found".to_owned()),
            "{path}"
        );
        let r = h.browser().post(path, body).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{path} for a stranger");
    }
    assert_eq!(h.store.list_users().unwrap().len(), 1);
    h.shutdown().await;
}

// ------------------------------------------------------------------ cookies

#[tokio::test]
async fn session_cookies_have_the_required_attributes() {
    let h = Harness::new(true);
    let (b, _) = setup(&h).await;
    let session = b
        .set_cookies
        .iter()
        .rev()
        .find(|c| c.starts_with("opsbox_session="))
        .unwrap();
    let csrf = b
        .set_cookies
        .iter()
        .rev()
        .find(|c| c.starts_with("opsbox_csrf="))
        .unwrap();
    for c in [session, csrf] {
        assert!(
            c.contains("SameSite=Strict") && c.contains("Path=/") && c.contains("Max-Age=604800"),
            "{c}"
        );
        assert!(
            !c.contains("Secure"),
            "plain HTTP on loopback must not set Secure: {c}"
        );
    }
    assert!(
        session.contains("HttpOnly"),
        "scripts must not read the session: {session}"
    );
    assert!(
        !csrf.contains("HttpOnly"),
        "the page must read the CSRF token: {csrf}"
    );
    // The CSRF value is derived from the session, not the session itself.
    assert_ne!(b.jar["opsbox_csrf"], b.jar["opsbox_session"]);
    h.shutdown().await;
}

#[tokio::test]
async fn cookies_are_marked_secure_behind_an_https_proxy() {
    let h = Harness::new(true);
    let (mut b, secret) = setup(&h).await;
    b.send(Method::POST, "/api/v1/auth/logout", None, Opts::default())
        .await;
    let opts = Opts {
        extra: vec![("x-forwarded-proto".into(), "https".into())],
        ..Opts::bare()
    };
    let r = b
        .send(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({"username": "admin", "password": PASSWORD, "code": code_for(&secret, 1)})),
            opts,
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let all: Vec<_> = b.set_cookies.iter().rev().take(2).collect();
    assert!(all.iter().all(|c| c.contains("Secure")), "{all:?}");
    h.shutdown().await;
}

// ------------------------------------------------------------------ CSRF and Origin

#[tokio::test]
async fn changing_requests_need_the_csrf_token_and_a_matching_origin() {
    let h = Harness::new(true);
    let mut b = h.signed_in("someone", Role::Admin);
    let path = "/api/v1/auth/sessions/revoke-all";

    let attempts: Vec<(&str, Opts)> = vec![
        ("no CSRF header, no Origin", Opts::bare()),
        (
            "Origin but no CSRF header",
            Opts {
                csrf: false,
                ..Opts::default()
            },
        ),
        (
            "CSRF header but no Origin",
            Opts {
                origin: None,
                ..Opts::default()
            },
        ),
        (
            "foreign Origin",
            Opts {
                origin: Some("http://evil.example".into()),
                ..Opts::default()
            },
        ),
        (
            "Origin: null",
            Opts {
                origin: Some("null".into()),
                ..Opts::default()
            },
        ),
        (
            "Origin of a lookalike host",
            Opts {
                origin: Some(format!("{ORIGIN}.evil.example")),
                ..Opts::default()
            },
        ),
        (
            "wrong CSRF value",
            Opts {
                extra: vec![("x-opsbox-csrf".into(), "0".repeat(64))],
                csrf: false,
                ..Opts::default()
            },
        ),
    ];
    for (what, opts) in attempts {
        let r = b.send(Method::POST, path, None, opts).await;
        assert_eq!(
            (r.status, r.error_code()),
            (StatusCode::FORBIDDEN, "forbidden".to_owned()),
            "{what}"
        );
    }
    // None of those refusals did anything: the session still works...
    assert_eq!(b.get("/api/v1/host").await.status, StatusCode::OK);
    // ...and the real request goes through.
    let r = b.send(Method::POST, path, None, Opts::default()).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    h.shutdown().await;
}

#[tokio::test]
async fn a_csrf_token_from_another_session_does_not_work() {
    let h = Harness::new(true);
    let mut a = h.signed_in("alice", Role::Admin);
    let b = h.signed_in("bobby", Role::Admin);
    a.jar
        .insert("opsbox_csrf".into(), b.jar["opsbox_csrf"].clone());
    let r = a
        .send(Method::POST, "/api/v1/auth/logout", None, Opts::default())
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn reads_do_not_need_csrf() {
    let h = Harness::new(true);
    let mut b = h.signed_in("viewer1", Role::Viewer);
    assert_eq!(
        b.send(Method::GET, "/api/v1/host", None, Opts::bare())
            .await
            .status,
        StatusCode::OK
    );
    h.shutdown().await;
}

#[tokio::test]
async fn sign_in_from_another_site_is_refused_before_anything_is_checked() {
    let h = Harness::new(true);
    let (_, secret) = setup(&h).await;
    let mut b = h.browser();
    let evil = Opts {
        origin: Some("http://evil.example".into()),
        csrf: false,
        ..Opts::default()
    };
    let body = json!({"username": "admin", "password": PASSWORD, "code": code_for(&secret, 1)});
    let r = b
        .send(
            Method::POST,
            "/api/v1/auth/login",
            Some(body.clone()),
            evil.clone(),
        )
        .await;
    assert_eq!(
        (r.status, r.error_code()),
        (StatusCode::FORBIDDEN, "forbidden".to_owned())
    );
    assert!(b.jar.is_empty());
    // A cross-site attempt must not count against the account either.
    assert_eq!(h.store.failures_for_user("admin", 0).unwrap().0, 0);
    // Same for the setup endpoints.
    let r = b
        .send(
            Method::POST,
            "/api/v1/setup/claim",
            Some(json!({"token": "AAAA-AAAA-AAAA"})),
            evil,
        )
        .await;
    assert_eq!(
        r.status,
        StatusCode::NOT_FOUND,
        "setup is closed, so it is simply not there"
    );
    h.shutdown().await;
}

// ------------------------------------------------------------------ sign in and out

#[tokio::test]
async fn sign_in_logout_and_sign_out_everywhere() {
    let h = Harness::new(true);
    let (mut first, secret) = setup(&h).await;

    // Signing in from a second browser.
    let mut second = h.browser();
    let r = second
        .send(
            Method::POST,
            "/api/v1/auth/login",
            Some(
                json!({"username": " Admin ", "password": PASSWORD, "code": code_for(&secret, 1)}),
            ),
            Opts::bare(),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    assert_eq!(
        (
            v["user"].clone(),
            v["role"].clone(),
            v["enrollRequired"].clone()
        ),
        (json!("admin"), json!("admin"), json!(false))
    );
    let token = second.jar["opsbox_session"].clone();
    assert!(
        !r.text().contains(&token),
        "the session token is only ever in the cookie"
    );
    assert_eq!(second.get("/api/v1/host").await.status, StatusCode::OK);

    // Logout ends just this session and clears the cookies.
    let r = second.post("/api/v1/auth/logout", json!(null)).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(second.jar.is_empty(), "{:?}", second.jar);
    assert_eq!(
        first.get("/api/v1/host").await.status,
        StatusCode::OK,
        "other sessions stay"
    );
    second.jar.insert("opsbox_session".into(), token);
    assert_eq!(
        second.get("/api/v1/host").await.status,
        StatusCode::UNAUTHORIZED,
        "the old token is dead"
    );

    // Sign out everywhere kills the other session too.
    let r = first
        .post("/api/v1/auth/sessions/revoke-all", json!(null))
        .await;
    assert_eq!(
        (r.status, r.json()["sessions"].clone()),
        (StatusCode::OK, json!(1))
    );
    assert_eq!(
        first.get("/api/v1/host").await.status,
        StatusCode::UNAUTHORIZED
    );
    h.shutdown().await;
}

#[tokio::test]
async fn wrong_credentials_all_look_the_same_and_repeated_failures_lock_out() {
    let h = Harness::new(true);
    let (_, secret) = setup(&h).await;
    let mut b = h.browser();
    let mut messages = Vec::new();
    for (u, p, c) in [
        ("admin", "wrong password here", code_for(&secret, 1)),
        ("admin", PASSWORD, "000000".to_owned()),
        ("ghost", PASSWORD, code_for(&secret, 1)),
    ] {
        let r = b
            .send(
                Method::POST,
                "/api/v1/auth/login",
                Some(json!({"username": u, "password": p, "code": c})),
                Opts::bare(),
            )
            .await;
        assert_eq!(
            (r.status, r.error_code()),
            (StatusCode::UNAUTHORIZED, "unauthorized".to_owned()),
            "{u}"
        );
        messages.push(r.json()["error"]["message"].as_str().unwrap().to_owned());
    }
    assert!(messages.windows(2).all(|w| w[0] == w[1]), "{messages:?}");

    for _ in 0..2 {
        b.send(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({"username": "admin", "password": "wrong password here", "code": "000000"})),
            Opts::bare(),
        )
        .await;
    }
    // Five failures so far: the correct credentials are now refused, with a wait time.
    let r = b
        .send(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({"username": "admin", "password": PASSWORD, "code": code_for(&secret, 1)})),
            Opts::bare(),
        )
        .await;
    assert_eq!(
        (r.status, r.error_code()),
        (StatusCode::TOO_MANY_REQUESTS, "rate_limited".to_owned())
    );
    let wait: u64 = r.header("retry-after").parse().expect("Retry-After header");
    assert!((1..=30).contains(&wait), "{wait}");
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Try again in")
    );
    h.shutdown().await;
}

#[tokio::test]
async fn bad_request_bodies_get_validation_errors_not_plain_text() {
    let h = Harness::new(true);
    let mut b = h.browser();
    for body in [
        None,
        Some(json!({"username": 5})),
        Some(json!({"password": "x"})),
        Some(json!([])),
    ] {
        let r = b
            .send(Method::POST, "/api/v1/auth/login", body, Opts::bare())
            .await;
        assert_eq!(
            (r.status, r.error_code()),
            (StatusCode::BAD_REQUEST, "validation".to_owned())
        );
    }
    let r = b
        .send(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({"username": "", "password": ""})),
            Opts::bare(),
        )
        .await;
    assert_eq!(r.json()["error"]["field"], "username");
}

// ------------------------------------------------------------------ 2FA recovery

#[tokio::test]
async fn after_reset_2fa_the_password_alone_opens_only_the_enrolment_endpoints() {
    let h = Harness::new(true);
    let (_, secret) = setup(&h).await;
    h.state.auth.reset_2fa("admin", "cli:root").unwrap();

    let mut b = h.browser();
    let r = b
        .send(
            Method::POST,
            "/api/v1/auth/login",
            Some(json!({"username": "admin", "password": PASSWORD})),
            Opts::bare(),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(r.json()["enrollRequired"], true);
    assert!(
        b.set_cookies.last().unwrap().contains("Max-Age=900")
            || b.set_cookies[b.set_cookies.len() - 2].contains("Max-Age=900")
    );

    let s = b.get("/api/v1/session").await.json();
    assert_eq!(
        (s["authenticated"].clone(), s["enrollRequired"].clone()),
        (json!(false), json!(true))
    );
    // Nothing else works yet.
    let r = b.get("/api/v1/host").await;
    assert_eq!(
        (r.status, r.error_code()),
        (StatusCode::FORBIDDEN, "forbidden".to_owned())
    );
    assert!(
        r.json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("two-factor")
    );
    assert_eq!(
        b.post("/api/v1/auth/logout", json!(null)).await.status,
        StatusCode::FORBIDDEN
    );

    let r = b.post("/api/v1/auth/totp/enroll", json!(null)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let new_secret = r.json()["secret"].as_str().unwrap().to_owned();
    assert_ne!(new_secret, secret);
    let r = b
        .post("/api/v1/auth/totp/confirm", json!({"code": "000000"}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = b
        .post(
            "/api/v1/auth/totp/confirm",
            json!({"code": code_for(&new_secret, 0)}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(
        b.get("/api/v1/host").await.status,
        StatusCode::OK,
        "now a full session"
    );
    // A full session cannot restart enrolment.
    assert_eq!(
        b.post("/api/v1/auth/totp/enroll", json!(null)).await.status,
        StatusCode::FORBIDDEN
    );
    h.shutdown().await;
}

// ------------------------------------------------------------------ roles

fn role_router(h: &Harness) -> Router {
    Router::new()
        .route("/viewer", get(|Viewer(_): Viewer| async { "ok" }))
        .route("/deployer", get(|Deployer(_): Deployer| async { "ok" }))
        .route("/admin", get(|Admin(_): Admin| async { "ok" }))
        .with_state(h.state.clone())
}

#[tokio::test]
async fn roles_are_enforced_by_the_extractors() {
    let h = Harness::new(true);
    let app = role_router(&h);
    let mut results = Vec::new();
    for (role, name) in [
        (Role::Viewer, "vv"),
        (Role::Deployer, "dd"),
        (Role::Admin, "aa"),
    ] {
        let mut b = h.signed_in(&format!("{name}user"), role);
        b.app = app.clone();
        let mut row = Vec::new();
        for path in ["/viewer", "/deployer", "/admin"] {
            row.push(b.get(path).await.status.as_u16());
        }
        results.push((role, row));
    }
    assert_eq!(results[0].1, [200, 403, 403], "viewer");
    assert_eq!(results[1].1, [200, 200, 403], "deployer");
    assert_eq!(results[2].1, [200, 200, 200], "admin");
    // Anonymous callers get 401, not 403.
    let mut anon = h.browser();
    anon.app = app;
    assert_eq!(anon.get("/admin").await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_role_message_is_a_sentence_for_people() {
    let h = Harness::new(true);
    let mut b = h.signed_in("vv", Role::Viewer);
    b.app = role_router(&h);
    let r = b.get("/admin").await;
    assert_eq!(
        r.json()["error"]["message"],
        "Your account is not allowed to do this."
    );
}

#[test]
fn host_header_constant_matches_origin_constant() {
    assert_eq!(ORIGIN, format!("http://{HOST}"));
}
