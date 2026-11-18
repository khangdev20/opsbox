//! Prompt 16o: masked secret fields, reveal re-auth / audit / rate-limit.
mod common;

use axum::http::{Method, StatusCode, header};
use common::{Harness, Opts, PASSWORD};
use opsbox_auth::password::{HashParams, Hasher};
use opsbox_proto::{Role, Secret};
use opsbox_store::{NewService, ServiceKind, SourceKind};
use serde_json::json;

const MARKER: &str = "marker-SECRET-value-zz9Q";

fn set_password(h: &Harness, name: &str) {
    let user = h.store.user_by_name(name).unwrap().unwrap();
    let hash = Hasher::new(HashParams {
        memory_kib: 8,
        iterations: 1,
        parallelism: 1,
    })
    .hash(PASSWORD)
    .unwrap();
    h.store.set_password_hash(&user.id, &hash, 0).unwrap();
}

#[tokio::test]
async fn list_responses_mask_secrets_and_reveal_needs_reauth_audit_and_rate_limit() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("secrets-admin", Role::Admin);
    set_password(&h, "secrets-admin");
    let admin_user = h.store.user_by_name("secrets-admin").unwrap().unwrap();

    let now = 1_700_000_000_i64;
    let (proj, env) = h.store.ensure_default_project(now).unwrap();
    let svc = h
        .store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: "web",
            kind: ServiceKind::App,
            owner_id: Some(&admin_user.id),
            source_kind: SourceKind::GitUrl,
            repo: "https://example.com/app.git",
            branch: "main",
            build_method: "dockerfile",
            dockerfile_path: None,
            start_cmd: None,
            port: 3000,
            health_path: "/",
            domain: None,
            image_ref: None,
            deploy_key_id: None,
            auto_deploy: true,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 0,
            graph_y: 0,
            note: None,
            now,
        })
        .unwrap();
    let sealed = h
        .state
        .priv_client
        .secret_seal(
            "test",
            Some("secrets-admin"),
            &format!("deploy:var:{}:DATABASE_URL", svc.id),
            Secret::new(format!("postgres://u:{MARKER}@db/app")),
        )
        .await
        .unwrap();
    h.store
        .upsert_service_var(&svc.id, "DATABASE_URL", &sealed, true, now)
        .unwrap();

    // List/detail must not contain the marker.
    let list = admin.get("/api/v1/projects/default").await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text());
    assert!(
        !list.text().contains(MARKER),
        "marker leaked in project list: {}",
        list.text()
    );
    let svc_json = list.json()["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "web" || s["id"] == svc.id)
        .cloned()
        .expect("web service");
    let vars = svc_json["vars"].as_array().unwrap();
    let db_url = vars
        .iter()
        .find(|v| v[0] == "DATABASE_URL")
        .expect("DATABASE_URL row");
    assert_eq!(db_url[1], "••••••••");
    assert_eq!(db_url[2], true);

    // Reveal without re-auth is refused (service id in the URL is the name).
    let path = "/api/v1/services/web/vars/DATABASE_URL/reveal";
    let denied = admin
        .send(Method::POST, path, Some(json!({})), Opts::default())
        .await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.text());
    assert!(!denied.text().contains(MARKER));

    // Wrong password is refused.
    let bad = admin
        .send(
            Method::POST,
            "/api/v1/auth/reauth",
            Some(json!({ "password": "not-the-password" })),
            Opts::default(),
        )
        .await;
    assert_eq!(bad.status, StatusCode::FORBIDDEN, "{}", bad.text());

    // Correct password opens the window; reveal returns the value with no-store.
    let ok = admin
        .send(
            Method::POST,
            "/api/v1/auth/reauth",
            Some(json!({ "password": PASSWORD })),
            Opts::default(),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text());

    let shown = admin
        .send(Method::POST, path, Some(json!({})), Opts::default())
        .await;
    assert_eq!(shown.status, StatusCode::OK, "{}", shown.text());
    assert_eq!(shown.header(header::CACHE_CONTROL.as_str()), "no-store");
    assert!(
        shown.json()["value"].as_str().unwrap().contains(MARKER),
        "{}",
        shown.text()
    );

    let audit = h.store.recent_audit(50).unwrap();
    let reveal = audit
        .iter()
        .find(|e| e.action == "service.var.reveal")
        .expect("reveal audited");
    assert_eq!(reveal.target.as_deref(), Some("web"));
    let detail: serde_json::Value = serde_json::from_str(&reveal.detail).unwrap();
    assert_eq!(detail["secret"], json!("DATABASE_URL"));
    assert!(!reveal.detail.contains(MARKER));

    // Exhaust the reveal rate limit (burst 10; one already used).
    for i in 0..20 {
        let r = admin
            .send(Method::POST, path, Some(json!({})), Opts::default())
            .await;
        if r.status == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(r.error_code(), "rate_limited");
            assert!(i >= 8, "rate limit should kick in after the burst, got at {i}");
            h.shutdown().await;
            return;
        }
        assert_eq!(r.status, StatusCode::OK, "iter {i}: {}", r.text());
    }
    panic!("reveal rate limit never triggered");
}
