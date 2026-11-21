mod common;
use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_alerts::{Engine, Observation};
use opsbox_proto::{Role, Secret};
use serde_json::json;
use std::collections::BTreeSet;

#[tokio::test]
async fn secrets_are_sealed_redacted_context_bound_and_configuration_survives_reloads() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("alert-admin", Role::Admin);
    let secret = "a-long-HMAC-secret-never-returned";
    let reply = admin
        .send(
            Method::PUT,
            "/api/v1/alerts/channels/c4",
            Some(json!({"kind":"webhook","url":"https://example.com/alerts","secret":secret})),
            Opts::default(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let stored = h.store.get_setting("alerts.engine.v1").unwrap().unwrap();
    assert!(!stored.to_string().contains(secret));
    assert!(!stored.to_string().contains("example.com"));
    let e: Engine = serde_json::from_value(stored).unwrap();
    let sealed = &e.channels["c4"].sealed;
    let opened = h
        .state
        .priv_client
        .secret_open("test", None, "alerts:channel:c4", sealed)
        .await
        .unwrap();
    assert!(opened.expose().contains(secret));
    assert!(
        h.state
            .priv_client
            .secret_open("test", None, "alerts:channel:c3", sealed)
            .await
            .is_err()
    );
    assert!(
        h.state
            .priv_client
            .secret_seal("test", None, "totp:admin", Secret::new("bad"))
            .await
            .is_err()
    );
    let reply = admin.get("/api/v1/alerts").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(!reply.text().contains(secret));
    assert!(!reply.text().contains(sealed));
    assert_eq!(reply.json()["rules"].as_array().unwrap().len(), 13);
    assert_eq!(reply.json()["channels"][3]["on"], true);
    assert_eq!(reply.json()["channels"][3]["verified"], false);
    let no_csrf = Opts {
        csrf: false,
        ..Opts::default()
    };
    assert_eq!(
        admin
            .send(
                Method::PUT,
                "/api/v1/alerts/rules/disk",
                Some(json!({"on":false})),
                no_csrf
            )
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        admin
            .send(
                Method::PUT,
                "/api/v1/alerts/rules/disk",
                Some(json!({"on":false})),
                Opts::default()
            )
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        admin.get("/api/v1/alerts").await.json()["rules"][0]["on"],
        false
    );
    assert_eq!(
        admin
            .send(
                Method::DELETE,
                "/api/v1/alerts/channels/c4",
                None,
                Opts::default()
            )
            .await
            .status,
        StatusCode::OK
    );
    assert!(
        h.store.get_setting("alerts.engine.v1").unwrap().unwrap()["channels"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    h.shutdown().await;
}

#[tokio::test]
async fn discord_url_is_validated_sealed_and_redacted() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("discord-admin", Role::Admin);
    for url in [
        "http://discord.com/api/webhooks/123/private-token",
        "https://127.0.0.1/api/webhooks/123/private-token",
        "https://discord.com.evil.test/api/webhooks/123/private-token",
        "https://discord.com/api/webhooks/123/private-token/extra",
    ] {
        let reply = admin
            .send(
                Method::PUT,
                "/api/v1/alerts/channels/c5",
                Some(json!({"kind":"discord","url":url})),
                Opts::default(),
            )
            .await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{url}: {}",
            reply.text()
        );
    }
    let url = "https://discord.com/api/webhooks/123/private-token";
    let reply = admin
        .send(
            Method::PUT,
            "/api/v1/alerts/channels/c5",
            Some(json!({"kind":"discord","url":url})),
            Opts::default(),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
    let stored = h.store.get_setting("alerts.engine.v1").unwrap().unwrap();
    assert!(!stored.to_string().contains("private-token"));
    let listed = admin.get("/api/v1/alerts").await;
    assert!(!listed.text().contains("private-token"));
    assert_eq!(listed.json()["channels"][4]["to"], "…/oken");
    let reveal = admin
        .post("/api/v1/alerts/channels/c5/reveal", json!({}))
        .await;
    assert_eq!(reveal.status, StatusCode::BAD_REQUEST);
    h.shutdown().await;
}

#[tokio::test]
async fn roles_validation_missing_channel_and_priv_outage_fail_safely() {
    let h = Harness::new(false);
    let mut anonymous = h.browser();
    assert_eq!(
        anonymous.get("/api/v1/alerts").await.status,
        StatusCode::UNAUTHORIZED
    );
    for (name, role) in [("viewer", Role::Viewer), ("deployer", Role::Deployer)] {
        let mut b = h.signed_in(name, role);
        assert_eq!(b.get("/api/v1/alerts").await.status, StatusCode::FORBIDDEN);
        assert_eq!(
            b.post("/api/v1/alerts/test", json!({})).await.status,
            StatusCode::FORBIDDEN
        );
    }
    let mut b = h.signed_in("admin", Role::Admin);
    assert_eq!(
        b.post("/api/v1/alerts/test", json!({})).await.status,
        StatusCode::BAD_REQUEST
    );
    for config in [
        json!({"kind":"webhook","url":"http://example.com","secret":"1234567890123456"}),
        json!({"kind":"webhook","url":"https://example.com","secret":"short"}),
        json!({"kind":"telegram","token":"token","chat":"3"}),
    ] {
        let reply = b
            .send(
                Method::PUT,
                "/api/v1/alerts/channels/c4",
                Some(config),
                Opts::default(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
    }
    assert_eq!(
        b.send(
            Method::PUT,
            "/api/v1/alerts/quiet",
            Some(json!({"from":"24:00","to":"07:00"})),
            Opts::default()
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        b.send(
            Method::PUT,
            "/api/v1/alerts/channels/c4",
            Some(json!({"kind":"webhook","url":"https://example.com","secret":"1234567890123456"})),
            Opts::default()
        )
        .await
        .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(h.store.get_setting("alerts.engine.v1").unwrap().is_none());
    h.shutdown().await;
}

#[tokio::test]
async fn silence_is_per_target_persistent_and_attention_links_match() {
    let h = Harness::new(true);
    let mut b = h.signed_in("admin", Role::Admin);
    let mut e = Engine::default();
    let known = BTreeSet::from(["disk".into()]);
    let observation =
        || Observation::new("disk", "/", "Disk / is filling".into(), "#/cleanup", false);
    e.evaluate(vec![observation()], &known, 43200);
    e.evaluate(vec![observation()], &known, 43260);
    h.store
        .set_setting("alerts.engine.v1", &serde_json::to_value(e).unwrap())
        .unwrap();
    let att = b.get("/api/v1/attention").await.json();
    let item = att["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["alertId"] == "disk:/")
        .unwrap();
    assert_eq!(item["id"], "disk:/");
    assert_eq!(item["link"][1], "#/alerts?id=disk%3A%2F");
    assert_eq!(
        b.post(
            "/api/v1/alerts/silence",
            json!({"id":"disk:/","seconds":3600})
        )
        .await
        .status,
        StatusCode::OK
    );
    let response = b.get("/api/v1/alerts").await.json();
    assert!(response["incidents"][0]["silenceUntil"].as_i64().unwrap() > 43260);
    assert_eq!(
        b.post(
            "/api/v1/alerts/silence",
            json!({"id":"disk:/","seconds":-1})
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        b.post(
            "/api/v1/alerts/silence",
            json!({"id":"absent","seconds":3600})
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        b.send(
            Method::PUT,
            "/api/v1/alerts/quiet",
            Some(json!({"from":"00:00","to":"00:00"})),
            Opts::default()
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(b.get("/api/v1/alerts").await.json()["quiet"]["tz"], "UTC");
    h.shutdown().await;
}

#[tokio::test]
async fn all_channel_credentials_are_sealed_and_failed_test_stays_unverified() {
    let h = Harness::new(true);
    let mut b = h.signed_in("channel-admin", Role::Admin);
    let configurations = [
        (
            "c1",
            json!({"kind":"email","host":"smtp.example.com","port":587,"username":"opsbox","password":"smtp-password-private","from":"a@example.com","to":"b@example.com"}),
        ),
        (
            "c2",
            json!({"kind":"slack","url":"https://hooks.slack.com/services/PRIVATE/PRIVATE/slack-private-token"}),
        ),
        (
            "c3",
            json!({"kind":"telegram","token":"12345:telegram-private-token","chat":"123456"}),
        ),
        (
            "c4",
            json!({"kind":"webhook","url":"https://127.0.0.1:9/notify","secret":"webhook-private-signing-secret"}),
        ),
    ];
    for (id, body) in configurations {
        let reply = b
            .send(
                Method::PUT,
                &format!("/api/v1/alerts/channels/{id}"),
                Some(body.clone()),
                Opts::default(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        let stored = h.store.get_setting("alerts.engine.v1").unwrap().unwrap();
        assert!(!stored.to_string().contains("private"));
        assert!(!b.get("/api/v1/alerts").await.text().contains("private"));
    }
    let reply = b.post("/api/v1/alerts/test", json!({"channel":"c4"})).await;
    assert_eq!(
        reply.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        reply.text()
    );
    let channels = b.get("/api/v1/alerts").await.json();
    assert_eq!(channels["channels"][3]["verified"], false);
    assert!(channels["channels"][3]["error"].is_string());
    let audit = h.store.recent_audit(100).unwrap();
    assert!(audit.iter().all(|a| !a.detail.contains("private")));
    let token = b
        .post(
            "/api/v1/tokens",
            json!({"name":"alerts-test","scope":"full"}),
        )
        .await
        .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut script = h.browser();
    let reply = script
        .send(
            Method::POST,
            "/api/v1/alerts/test",
            Some(json!({"channel":"c4"})),
            Opts {
                csrf: false,
                origin: None,
                extra: vec![("authorization".into(), format!("Bearer {token}"))],
                ..Opts::default()
            },
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.text());
    h.shutdown().await;
}
