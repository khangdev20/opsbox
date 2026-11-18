//! Panel domain, DNS check, certificate task, and raw :8443 listener controls.

mod common;

use std::net::IpAddr;
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_priv::{PrivConfig, PrivServer};
use opsbox_proto::Role;
use opsbox_proxy::{AcmeDirectory, ProxyClient, ProxyConfig, ProxyServer};
use serde_json::{Value, json};
use tokio::sync::oneshot;

async fn send(b: &mut common::Browser, m: Method, p: &str, body: Option<Value>) -> common::Reply {
    b.send(m, p, body, Opts::default()).await
}

#[tokio::test]
async fn panel_endpoints_require_admin_and_validate_domains() {
    let h = Harness::new(true);
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/panel").await.status,
        StatusCode::UNAUTHORIZED
    );

    for role in [Role::Viewer, Role::Deployer] {
        let mut b = h.signed_in(&format!("{role:?}"), role);
        assert_eq!(b.get("/api/v1/panel").await.status, StatusCode::FORBIDDEN);
        assert_eq!(
            send(
                &mut b,
                Method::POST,
                "/api/v1/panel/dns-check",
                Some(json!({"domain": "panel.example.com"})),
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                &mut b,
                Method::POST,
                "/api/v1/panel/certificate",
                Some(json!({"domain": "panel.example.com"})),
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                &mut b,
                Method::POST,
                "/api/v1/panel/local-only",
                Some(json!({"localOnly": true})),
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
    }

    let mut admin = h.signed_in("boss", Role::Admin);
    let st = admin.get("/api/v1/panel").await;
    assert_eq!(st.status, StatusCode::OK);
    assert_eq!(st.json()["certStatus"], "none");
    assert_eq!(st.json()["rawPortOpen"], false);

    for bad in [
        "",
        "localhost",
        "127.0.0.1",
        "*.example.com",
        "http://a.com",
        "a.com:8443",
        "a.com/path",
    ] {
        let r = send(
            &mut admin,
            Method::POST,
            "/api/v1/panel/dns-check",
            Some(json!({"domain": bad})),
        )
        .await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{bad}: {}", r.text());
    }

    // Without a proxy running, certificate issuance fails cleanly with 503.
    let no_proxy = send(
        &mut admin,
        Method::POST,
        "/api/v1/panel/certificate",
        Some(json!({"domain": "panel.example.com"})),
    )
    .await;
    assert_eq!(no_proxy.status, StatusCode::SERVICE_UNAVAILABLE);

    // Closing port 8443 without a live domain certificate requires confirmation.
    let lock_warn = send(
        &mut admin,
        Method::POST,
        "/api/v1/panel/local-only",
        Some(json!({"localOnly": true})),
    )
    .await;
    assert_eq!(lock_warn.status, StatusCode::CONFLICT);
    assert_eq!(lock_warn.error_code(), "confirmation_required");

    h.shutdown().await;
}

#[tokio::test]
async fn readyz_checks_proxy_when_panel_domain_is_active() {
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    std::fs::write(&probe, b"").unwrap();
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata(&probe).unwrap().uid();

    let priv_sock = dir.path().join("priv.sock");
    let mut priv_cfg = PrivConfig::new(&priv_sock, vec![uid]);
    priv_cfg.capacity_profile = Some(common::capacity_profile());
    priv_cfg.capacity_image_size_mb = Some(64);
    priv_cfg.master_key_path = dir.path().join("master.key");
    let priv_srv = PrivServer::bind(priv_cfg).unwrap();
    let (priv_stop, priv_rx) = oneshot::channel::<()>();
    let priv_task = tokio::spawn(priv_srv.run(async {
        let _ = priv_rx.await;
    }));

    let proxy_sock = dir.path().join("proxy.sock");
    let mut proxy_cfg = ProxyConfig::new(&priv_sock);
    proxy_cfg.listen_http = "127.0.0.1:0".parse().unwrap();
    proxy_cfg.listen_https = "127.0.0.1:0".parse().unwrap();
    proxy_cfg.socket_path = proxy_sock.clone();
    proxy_cfg.state_dir = dir.path().join("proxy-state");
    proxy_cfg.acme_directory = AcmeDirectory::Staging;
    let proxy_srv = ProxyServer::bind(proxy_cfg).await.unwrap();
    let (proxy_stop, proxy_rx) = oneshot::channel::<()>();
    let proxy_task = tokio::spawn(async move {
        proxy_srv
            .run(async {
                let _ = proxy_rx.await;
            })
            .await
            .unwrap();
    });

    let store = std::sync::Arc::new(opsbox_store::Store::open_in_memory().unwrap());
    let u = store.create_user("boss", Role::Admin, "unused", 0).unwrap();
    store.set_pending_totp(&u.id, "unused", 0).unwrap();
    store.enable_totp(&u.id, 0, 0).unwrap();

    let state = opsbox_api::AppState::new(
        store.clone(),
        opsbox_priv::PrivClient::new(&priv_sock).with_timeout(Duration::from_secs(5)),
        opsbox_priv::VERSION,
        opsbox_api::SecureCookies::Auto,
        opsbox_auth::AuthConfig::default(),
    )
    .with_proxy(
        ProxyClient::new(&proxy_sock).with_timeout(Duration::from_secs(5)),
        None,
        vec![IpAddr::from([1, 1, 1, 1])],
    );
    let app = opsbox_api::router(state.clone());

    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let resp = app
        .clone()
        .oneshot(
            axum::http::Request::get("/readyz")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["checks"]["proxy"], "ok");

    // When a panel domain is active and proxy goes down, /readyz reports 503.
    store
        .set_setting("opsbox.panel.domain", &json!("panel.example.com"))
        .unwrap();
    let _ = proxy_stop.send(());
    proxy_task.await.unwrap();

    let resp2 = app
        .oneshot(
            axum::http::Request::get("/readyz")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body2: Value =
        serde_json::from_slice(&resp2.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body2["checks"]["proxy"], "failing");

    let _ = priv_stop.send(());
    priv_task.await.unwrap();
}
