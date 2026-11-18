//! Security checks, the allowed-address list, invitations and account removal.
//! Nothing here changes the machine: the fixes used are refused before anything runs.

mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_proto::Role;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

async fn send(b: &mut common::Browser, m: Method, p: &str, body: Option<Value>) -> common::Reply {
    b.send(m, p, body, Opts::default()).await
}

#[tokio::test]
async fn the_security_page_data_has_checks_users_and_an_audit_trail() {
    let h = Harness::new(true);
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/security").await.status,
        StatusCode::UNAUTHORIZED
    );

    let mut admin = h.signed_in("boss", Role::Admin);
    let r = admin.get("/api/v1/security").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    let checks = v["checks"].as_array().unwrap();
    assert!(checks.iter().any(|c| c["id"] == "2fa") && checks.iter().any(|c| c["id"] == "lockout"));
    assert_eq!(v["total"], checks.len());
    assert_eq!(
        v["passing"],
        checks.iter().filter(|c| c["ok"] == true).count()
    );
    for c in checks {
        assert!(
            c["t"].as_str().is_some_and(|t| !t.is_empty()) && c["ok"].is_boolean(),
            "{c}"
        );
    }
    let me = v["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["name"] == "boss")
        .unwrap();
    assert_eq!(
        (me["name"].as_str(), me["you"].as_bool()),
        (Some("boss"), Some(true))
    );
    assert!(v["allowIps"].is_string());
    // `signed_in` skipped 2FA, so the check says who still has to set it up.
    let tfa = checks.iter().find(|c| c["id"] == "2fa").unwrap();
    assert_eq!(tfa["ok"], false);
    assert!(tfa["t"].as_str().unwrap().contains("boss"));

    // The audit trail is for administrators only; everyone signed in may see the checks.
    let mut viewer = h.signed_in("watcher", Role::Viewer);
    let rv = viewer.get("/api/v1/security").await;
    assert_eq!(rv.status, StatusCode::OK);
    assert_eq!(rv.json()["audit"], json!([]));
    h.shutdown().await;
}

#[tokio::test]
async fn only_admins_change_security_settings_and_unknown_fixes_are_refused() {
    let h = Harness::new(true);
    for role in [Role::Viewer, Role::Deployer] {
        let mut b = h.signed_in(&format!("{role:?}"), role);
        for (m, p, body) in [
            (
                Method::PUT,
                "/api/v1/security/allow-ips",
                Some(json!({"value": "1.2.3.4"})),
            ),
            (Method::POST, "/api/v1/security/checks/secrets/fix", None),
            (
                Method::POST,
                "/api/v1/users/invites",
                Some(json!({"name": "dana", "role": "viewer"})),
            ),
            (Method::DELETE, "/api/v1/users/someone", None),
        ] {
            let r = send(&mut b, m, p, body).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{role:?} {p}");
        }
    }
    let mut a = h.signed_in("boss", Role::Admin);
    for id in ["firewall", "lockout", "..%2Fetc", "masterkey"] {
        let r = send(
            &mut a,
            Method::POST,
            &format!("/api/v1/security/checks/{id}/fix"),
            None,
        )
        .await;
        assert!(
            r.status == StatusCode::BAD_REQUEST || r.status == StatusCode::NOT_FOUND,
            "{id}: {} {}",
            r.status,
            r.text()
        );
    }
    assert!(
        h.store
            .recent_audit(20)
            .unwrap()
            .iter()
            .all(|e| e.action != "security.fix")
    );
    h.shutdown().await;
}

#[tokio::test]
async fn the_allowed_list_is_validated_saved_and_survives_a_restart() {
    let h = Harness::new(true);
    let mut a = h.signed_in("boss", Role::Admin);
    for bad in ["not-an-ip", "1.2.3.4/40", "10.0.0.1, ,x"] {
        let r = send(
            &mut a,
            Method::PUT,
            "/api/v1/security/allow-ips",
            Some(json!({"value": bad})),
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}");
        assert!(r.text().contains("not a valid"), "{}", r.text());
    }
    let r = send(
        &mut a,
        Method::PUT,
        "/api/v1/security/allow-ips",
        Some(json!({"value": " 203.0.113.0/24 , 198.51.100.7"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    assert_eq!(
        r.json()["allowIps"],
        "203.0.113.0/24, 198.51.100.7",
        "{}",
        r.text()
    );
    // (These in-process requests have no peer address, so with a list set the server refuses
    // them, which is the cautious behaviour; the state is read directly here.)
    assert_eq!(h.state.allow_list_text(), "203.0.113.0/24, 198.51.100.7");
    assert_eq!(
        a.get("/api/v1/security").await.status,
        StatusCode::FORBIDDEN
    );
    assert!(
        h.store
            .recent_audit(5)
            .unwrap()
            .iter()
            .any(|e| e.action == "security.allow_ips")
    );
    // A new process reads the saved list from the database.
    let again = opsbox_api::AppState::new(
        h.store.clone(),
        h.state.priv_client.clone(),
        "t",
        opsbox_api::SecureCookies::Auto,
        opsbox_auth::AuthConfig::default(),
    );
    assert_eq!(again.allow_list_text(), "203.0.113.0/24, 198.51.100.7");
    h.state.allow.set(Vec::new());
    assert_eq!(a.get("/api/v1/security").await.json()["allowIps"], "");
    h.shutdown().await;
}

/// One HTTP/1.1 request over a real connection, so the server sees a real peer address.
async fn raw(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, String)],
    body: Option<&Value>,
) -> (u16, String) {
    let body = body.map(ToString::to_string).unwrap_or_default();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (
        status,
        text.split("\r\n\r\n").nth(1).unwrap_or("").to_owned(),
    )
}

#[tokio::test]
async fn a_list_that_excludes_you_needs_confirmation_and_then_blocks_other_addresses() {
    let h = Harness::new(true);
    let b = h.signed_in("boss", Role::Admin);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // The local proxy (and only a peer that shows its secret) may report the caller's address.
    const PROXY_SECRET: &str = "test-proxy-secret-test-proxy-secret";
    tokio::spawn(opsbox_api::serve(
        listener,
        h.state.clone().with_proxy_secret(PROXY_SECRET),
        std::future::pending(),
    ));
    let cookie = b
        .jar
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");
    let auth = |xff: &str| -> Vec<(&'static str, String)> {
        vec![
            ("Cookie", cookie.clone()),
            ("X-Opsbox-CSRF", b.jar["opsbox_csrf"].clone()),
            ("Origin", format!("http://{addr}")),
            ("X-Forwarded-For", xff.to_owned()),
            ("X-Opsbox-Proxy-Secret", PROXY_SECRET.to_owned()),
        ]
    };
    // Through a local proxy, the person's address is the one the proxy reports.
    let put = |xff: &str, confirm: bool| {
        let headers = auth(xff);
        async move {
            raw(
                addr,
                "PUT",
                "/api/v1/security/allow-ips",
                &headers,
                Some(&json!({"value": "198.51.100.0/24", "confirm": confirm})),
            )
            .await
        }
    };
    let (status, body) = put("203.0.113.9", false).await;
    assert_eq!(status, 409, "{body}");
    assert!(
        body.contains("confirmation_required") && body.contains("203.0.113.9"),
        "{body}"
    );
    // Confirming applies it; this address is now outside the list.
    assert_eq!(put("203.0.113.9", true).await.0, 200);
    assert_eq!(
        raw(addr, "GET", "/api/v1/security", &auth("203.0.113.9"), None)
            .await
            .0,
        403
    );
    // An address on the list, a tunnel (no forwarded address, so loopback) and the health check still work.
    assert_eq!(
        raw(
            addr,
            "GET",
            "/api/v1/security",
            &auth("198.51.100.77"),
            None
        )
        .await
        .0,
        200
    );
    let mut tunnel = auth("");
    tunnel.truncate(3);
    assert_eq!(
        raw(addr, "GET", "/api/v1/security", &tunnel, None).await.0,
        200
    );
    // A local client that is not the proxy cannot name another address: it is the tunnel
    // (loopback), not 203.0.113.9, which the list would refuse.
    let mut spoof = auth("203.0.113.9");
    spoof.pop();
    let (status, body) = raw(addr, "GET", "/api/v1/security", &spoof, None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("allowIps"), "{body}");
    assert_eq!(
        raw(addr, "GET", "/healthz", &auth("203.0.113.9"), None)
            .await
            .0,
        200
    );
    // And from inside the list, the person can clear it again.
    let (status, _) = raw(
        addr,
        "PUT",
        "/api/v1/security/allow-ips",
        &auth("198.51.100.77"),
        Some(&json!({"value": ""})),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        raw(addr, "GET", "/api/v1/security", &auth("203.0.113.9"), None)
            .await
            .0,
        200
    );
    h.shutdown().await;
}

#[tokio::test]
async fn an_invitation_over_http_from_link_to_enrolment() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("boss", Role::Admin);
    for (body, word) in [
        (
            json!({"name": "dana", "role": "root"}),
            "admin, deployer or viewer",
        ),
        (json!({"name": "d", "role": "viewer"}), "3 to 32"),
        (json!({"name": "boss", "role": "viewer"}), "already taken"),
    ] {
        let r = send(
            &mut admin,
            Method::POST,
            "/api/v1/users/invites",
            Some(body),
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text());
        assert!(
            r.text().to_lowercase().contains(word),
            "{word}: {}",
            r.text()
        );
    }
    let r = send(
        &mut admin,
        Method::POST,
        "/api/v1/users/invites",
        Some(json!({"name": " Dana ", "role": "deployer"})),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let inv = r.json();
    let token = inv["token"].as_str().unwrap().to_owned();
    assert_eq!(
        (inv["name"].as_str(), inv["role"].as_str()),
        (Some("dana"), Some("deployer"))
    );
    // The token itself is not kept anywhere in the database.
    let all = serde_json::to_string(
        &h.store
            .list_users()
            .unwrap()
            .iter()
            .map(|u| u.password_hash.clone())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert!(!all.contains(&token));

    // The invited person, not signed in, opens the link.
    let mut guest = h.browser();
    let info = guest.get(&format!("/api/v1/invites/{token}")).await;
    assert_eq!(info.json(), json!({"name": "dana", "role": "deployer"}));
    assert_eq!(
        guest.get("/api/v1/invites/not-a-real-token").await.status,
        StatusCode::BAD_REQUEST
    );
    // A page on another site cannot claim it.
    let foreign = guest
        .send(
            Method::POST,
            "/api/v1/invites/claim",
            Some(json!({"token": token, "password": "a long enough passphrase"})),
            Opts {
                origin: Some("http://evil.example".into()),
                ..Opts::bare()
            },
        )
        .await;
    assert_eq!(foreign.status, StatusCode::FORBIDDEN);
    let weak = guest
        .send(
            Method::POST,
            "/api/v1/invites/claim",
            Some(json!({"token": token, "password": "short"})),
            Opts::bare(),
        )
        .await;
    assert_eq!(weak.status, StatusCode::BAD_REQUEST);
    let ok = guest
        .send(
            Method::POST,
            "/api/v1/invites/claim",
            Some(json!({"token": token, "password": "a long enough passphrase"})),
            Opts::bare(),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    assert_eq!(ok.json()["enrollRequired"], true);
    let cookie = guest
        .set_cookies
        .iter()
        .find(|c| c.starts_with("opsbox_session="))
        .unwrap();
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Strict")
            && cookie.contains("Max-Age=900"),
        "{cookie}"
    );
    // The new session can only finish two-factor setup.
    let s = guest.get("/api/v1/session").await.json();
    assert_eq!(
        (s["authenticated"].clone(), s["enrollRequired"].clone()),
        (json!(false), json!(true))
    );
    assert_eq!(
        guest.get("/api/v1/security").await.status,
        StatusCode::FORBIDDEN
    );
    // The link is used up.
    let again = h
        .browser()
        .send(
            Method::POST,
            "/api/v1/invites/claim",
            Some(json!({"token": token, "password": "another long passphrase"})),
            Opts::bare(),
        )
        .await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    // The list shows dana with her role.
    let users = admin.get("/api/v1/security").await.json()["users"].clone();
    let dana = users
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["name"] == "dana")
        .unwrap();
    assert_eq!(
        (dana["roleKey"].as_str(), dana["pending"].as_bool()),
        (Some("deployer"), Some(false))
    );
    let actions: Vec<_> = h
        .store
        .recent_audit(20)
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    for a in ["user.invite", "user.invite.claim"] {
        assert!(actions.iter().any(|x| x == a), "{a} in {actions:?}");
    }
    h.shutdown().await;
}

#[tokio::test]
async fn removing_an_account_end_its_sessions_and_has_safeguards() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("boss", Role::Admin);
    let mut other = h.signed_in("temp", Role::Viewer);
    assert_eq!(other.get("/api/v1/security").await.status, StatusCode::OK);
    let me = send(&mut admin, Method::DELETE, "/api/v1/users/boss", None).await;
    assert_eq!(me.status, StatusCode::BAD_REQUEST);
    assert!(me.text().contains("your own account"));
    let gone = send(&mut admin, Method::DELETE, "/api/v1/users/temp", None).await;
    assert_eq!(gone.status, StatusCode::OK, "{}", gone.text());
    // Their session stops working at once.
    assert_eq!(
        other.get("/api/v1/security").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&mut admin, Method::DELETE, "/api/v1/users/temp", None)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert!(
        h.store
            .recent_audit(10)
            .unwrap()
            .iter()
            .any(|e| e.action == "user.remove")
    );
    h.shutdown().await;
}

#[tokio::test]
async fn display_name_updates_session_and_audit_actor() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("boss", Role::Admin);
    let bad = send(
        &mut admin,
        Method::PATCH,
        "/api/v1/users/boss",
        Some(json!({"display_name": "x".repeat(65)})),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text());
    assert!(bad.text().to_lowercase().contains("64"));

    let ok = send(
        &mut admin,
        Method::PATCH,
        "/api/v1/users/boss",
        Some(json!({"display_name": "  Boss Person  "})),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());
    assert_eq!(ok.json()["displayName"], json!("Boss Person"));

    let session = admin.get("/api/v1/session").await.json();
    assert_eq!(session["user"], json!("boss"));
    assert_eq!(session["displayName"], json!("Boss Person"));
    // `signed_in` bypasses TOTP enrolment, so the session reports the real off state.
    assert_eq!(session["totp"]["enabled"], json!(false));
    assert_eq!(session["totp"]["method"], json!(null));

    let actors: Vec<_> = h
        .store
        .recent_audit(20)
        .unwrap()
        .into_iter()
        .filter(|e| e.action == "user.display_name")
        .map(|e| e.actor)
        .collect();
    assert!(
        actors.iter().any(|a| a == "Boss Person (boss)" || a == "boss"),
        "expected display-name audit with boss as actor, got {actors:?}"
    );
    // After the name is set, a later mutation shows "Display (username)".
    let again = send(
        &mut admin,
        Method::PATCH,
        "/api/v1/users/boss",
        Some(json!({"display_name": "Boss Person"})),
    )
    .await;
    assert_eq!(again.status, StatusCode::OK, "{}", again.text());
    let labeled: Vec<_> = h
        .store
        .recent_audit(5)
        .unwrap()
        .into_iter()
        .filter(|e| e.action == "user.display_name")
        .map(|e| e.actor)
        .collect();
    assert!(
        labeled.iter().any(|a| a == "Boss Person (boss)"),
        "{labeled:?}"
    );

    let mut viewer = h.signed_in("cara", Role::Viewer);
    let self_ok = send(
        &mut viewer,
        Method::PATCH,
        "/api/v1/users/cara",
        Some(json!({"display_name": "Cara Q"})),
    )
    .await;
    assert_eq!(self_ok.status, StatusCode::OK, "{}", self_ok.text());
    let denied = send(
        &mut viewer,
        Method::PATCH,
        "/api/v1/users/boss",
        Some(json!({"display_name": "Nope"})),
    )
    .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.text());
    h.shutdown().await;
}

#[tokio::test]
async fn audit_list_returns_ip_filters_and_pages_and_export_is_admin_only() {
    use opsbox_store::{ActorKind, AuditResult, NewAudit};
    use serde_json::json;

    let h = Harness::new(true);
    for i in 0..5 {
        h.store
            .append_audit(NewAudit {
                actor: if i < 3 { "boss" } else { "other" }.into(),
                actor_kind: ActorKind::User,
                ip: Some(if i == 0 {
                    "127.0.0.1".into()
                } else if i == 1 {
                    "panel".into()
                } else if i == 2 {
                    "=1+1".into()
                } else {
                    format!("203.0.113.{i}")
                }),
                action: format!("auth.probe{i}"),
                target: None,
                result: if i == 2 {
                    AuditResult::Denied
                } else {
                    AuditResult::Ok
                },
                detail: json!({ "n": i }),
            })
            .unwrap();
    }

    let mut viewer = h.signed_in("watcher", Role::Viewer);
    assert_eq!(
        viewer.get("/api/v1/audit").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        viewer.get("/api/v1/audit/export.csv").await.status,
        StatusCode::FORBIDDEN
    );

    let mut admin = h.signed_in("boss", Role::Admin);
    let list = admin.get("/api/v1/audit?actor=boss&limit=2").await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    let body = list.json();
    let entries = body["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e["user"] == "boss"));
    assert!(entries[0]["ip"].is_string() || entries[0]["ip"].is_null());
    assert!(body["nextCursor"].as_i64().is_some());

    let page2 = admin
        .get(&format!(
            "/api/v1/audit?actor=boss&limit=10&cursor={}",
            body["nextCursor"]
        ))
        .await;
    assert_eq!(page2.status, StatusCode::OK, "{}", page2.text());
    assert_eq!(page2.json()["entries"].as_array().unwrap().len(), 1);

    let denied = admin.get("/api/v1/audit?result=denied").await;
    assert_eq!(denied.status, StatusCode::OK);
    assert_eq!(denied.json()["entries"].as_array().unwrap().len(), 1);

    let prefix = admin.get("/api/v1/audit?action=auth.probe").await;
    assert_eq!(prefix.status, StatusCode::OK);
    assert!(prefix.json()["entries"].as_array().unwrap().len() >= 5);

    // Seed enough rows that a naive collect-all would be obvious; export must still succeed.
    for i in 0..120 {
        h.store
            .append_audit(NewAudit {
                actor: "bulk".into(),
                actor_kind: ActorKind::User,
                ip: Some("198.51.100.9".into()),
                action: format!("bulk.{i}"),
                target: None,
                result: AuditResult::Ok,
                detail: json!({}),
            })
            .unwrap();
    }
    let csv = admin.get("/api/v1/audit/export.csv?actor=bulk").await;
    assert_eq!(csv.status, StatusCode::OK, "{}", csv.text());
    assert!(csv.header("content-type").starts_with("text/csv"));
    assert_eq!(csv.header("cache-control"), "no-store");
    assert!(
        csv.header("content-disposition")
            .contains("attachment; filename=\"opsbox-audit-"),
        "{}",
        csv.header("content-disposition")
    );
    let text = csv.text();
    assert!(text.starts_with(
        "seq,ts,actor,actor_kind,ip,action,target,result,detail,hash\n"
    ));
    assert!(text.contains("# chain: ok"), "{text}");
    assert!(text.matches("\n").count() >= 120);
    // Formula-looking detail from earlier rows must be neutralised when exported unfiltered.
    let all = admin.get("/api/v1/audit/export.csv?action=auth.probe").await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text());
    let all_text = all.text();
    assert!(
        all_text.contains("'=1+1") || all_text.contains("\"'=1+1"),
        "formula cells must be neutralised: {all_text}"
    );

    let sec = admin.get("/api/v1/security").await;
    assert_eq!(sec.status, StatusCode::OK);
    let sec_json = sec.json();
    let preview = sec_json["audit"].as_array().unwrap();
    assert!(preview.iter().any(|e| e.get("ip").is_some()), "{preview:?}");

    h.shutdown().await;
}
