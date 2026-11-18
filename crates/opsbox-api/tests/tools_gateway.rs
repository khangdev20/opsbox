//! The data-tool gateway: reachable only with a signed-in session, never forwards the panel's
//! cookies, scopes the tool's own cookies, and checks `Origin` on writes.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use opsbox_api::{AppState, SecureCookies, router};
use opsbox_auth::AuthConfig;
use opsbox_auth::password::HashParams;
use opsbox_priv::PrivClient;
use opsbox_proto::Role;
use opsbox_store::{NewService, ServiceKind, SessionKind, SourceKind, Store};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceExt;

const HOST: &str = "127.0.0.1:8443";
const ORIGIN: &str = "http://127.0.0.1:8443";

async fn upstream(seen: Arc<Mutex<Vec<String>>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap_or(0);
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf[..n]).into_owned());
                let body = "hi";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nSet-Cookie: adminer_sid=1; Path=/; Domain=evil.test\r\nLocation: /next\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    port
}

async fn call(
    app: &Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> (StatusCode, axum::http::HeaderMap) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, HOST);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    if let Some(o) = origin {
        b = b.header(header::ORIGIN, o);
    }
    let resp = app
        .clone()
        .oneshot(b.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let st = resp.status();
    let h = resp.headers().clone();
    let _ = resp.into_body().collect().await;
    (st, h)
}

#[tokio::test]
async fn gateway_needs_a_session_and_isolates_cookies() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = upstream(seen.clone()).await;

    let store = Arc::new(Store::open_in_memory().unwrap());
    let admin = store
        .create_user("alice", Role::Admin, "unused", 0)
        .unwrap();
    store.set_pending_totp(&admin.id, "unused", 0).unwrap();
    store.enable_totp(&admin.id, 0, 0).unwrap();
    let viewer = store.create_user("bob", Role::Viewer, "unused", 0).unwrap();
    store.set_pending_totp(&viewer.id, "unused", 0).unwrap();
    store.enable_totp(&viewer.id, 0, 0).unwrap();
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let (tok_a, tok_v) = ("sess-alice-tools-0001", "sess-bob-tools-0001");
    for (u, t) in [(&admin, tok_a), (&viewer, tok_v)] {
        store
            .create_session(
                &u.id,
                &opsbox_auth::tokens::hash_token(t),
                SessionKind::Web,
                None,
                None,
                now,
            )
            .unwrap();
    }
    let cookie = |t: &str| {
        format!(
            "opsbox_session={t}; opsbox_csrf={}; adminer_sid=1",
            opsbox_auth::tokens::csrf_token(t)
        )
    };

    let (proj, env) = store.resolve_project("default", now).unwrap().unwrap();
    let mk = |name: &str, kind: ServiceKind, template: &str| {
        store
            .create_service_with_exposure(
                NewService {
                    project_id: &proj.id,
                    env_id: &env.id,
                    name,
                    kind,
                    owner_id: None,
                    source_kind: SourceKind::DockerImage,
                    repo: "adminer:4.17.1",
                    branch: "4.17.1",
                    build_method: "Adminer",
                    dockerfile_path: None,
                    start_cmd: None,
                    port: 8080,
                    health_path: "tcp",
                    domain: None,
                    image_ref: Some("adminer:4.17.1"),
                    deploy_key_id: None,
                    auto_deploy: false,
                    memory_limit_mb: None,
                    cpu_limit_millis: None,
                    graph_x: 0,
                    graph_y: 0,
                    note: None,
                    now,
                },
                None,
                Some(template),
                false,
                false,
            )
            .unwrap()
    };
    let tool = mk("adm", ServiceKind::Tool, "adminer");
    store
        .complete_deploy_switch(&tool.id, "dep1", "adminer:4.17.1", "cid1", port, None, now)
        .unwrap();
    let other = mk("web", ServiceKind::Tool, "kuma");
    store
        .complete_deploy_switch(&other.id, "dep2", "x:1", "cid2", port, None, now)
        .unwrap();

    let state = AppState::new(
        store.clone(),
        PrivClient::new("/nonexistent/priv.sock").with_timeout(Duration::from_secs(1)),
        "0.1.0-test",
        SecureCookies::Auto,
        AuthConfig {
            hash_params: HashParams {
                memory_kib: 1024,
                iterations: 1,
                parallelism: 1,
            },
            ..Default::default()
        },
    );
    let app = router(state);
    let admin_cookie = cookie(tok_a);

    // Not signed in.
    let (st, _) = call(&app, Method::GET, "/tools/adm/", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    assert!(seen.lock().unwrap().is_empty());

    // Viewers cannot use data tools; unknown and non-data tools are not found.
    let (st, _) = call(&app, Method::GET, "/tools/adm/", Some(&cookie(tok_v)), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = call(&app, Method::GET, "/tools/web/", Some(&admin_cookie), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        &app,
        Method::GET,
        "/tools/nope/x",
        Some(&admin_cookie),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(seen.lock().unwrap().is_empty());

    // An admin reaches the tool; the panel's cookies stay behind and the tool's are scoped.
    let (st, h) = call(
        &app,
        Method::GET,
        "/tools/adm/?server=pg-main",
        Some(&admin_cookie),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    {
        let seen = seen.lock().unwrap();
        let req = seen.last().unwrap().to_ascii_lowercase();
        assert!(req.starts_with("get /?server=pg-main http/1.1"), "{req}");
        assert!(req.contains("cookie: adminer_sid=1\r\n"), "{req}");
        assert!(
            !req.contains("opsbox_session") && !req.contains("opsbox_csrf"),
            "{req}"
        );
        assert!(req.contains(&format!("host: 127.0.0.1:{port}")), "{req}");
    }
    let set_cookie = h.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(
        set_cookie.contains("Path=/tools/adm/") && set_cookie.contains("SameSite=Strict"),
        "{set_cookie}"
    );
    assert!(!set_cookie.contains("evil.test"));
    assert_eq!(h.get(header::LOCATION).unwrap(), "/tools/adm/next");
    assert_eq!(h.get("x-frame-options").unwrap(), "DENY");

    // Writes need a matching Origin (the tool cannot send the panel's CSRF header).
    let (st, _) = call(
        &app,
        Method::POST,
        "/tools/adm/x",
        Some(&admin_cookie),
        Some("http://evil.test"),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = call(
        &app,
        Method::POST,
        "/tools/adm/x",
        Some(&admin_cookie),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = call(
        &app,
        Method::POST,
        "/tools/adm/x",
        Some(&admin_cookie),
        Some(ORIGIN),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // The missing-slash form redirects, and opening the tool was audited.
    let (st, h) = call(&app, Method::GET, "/tools/adm", Some(&admin_cookie), None).await;
    assert_eq!(st, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(h.get(header::LOCATION).unwrap(), "/tools/adm/");
    assert!(
        store
            .recent_audit(50)
            .unwrap()
            .iter()
            .any(|e| e.action == "database.tool.open")
    );
}
