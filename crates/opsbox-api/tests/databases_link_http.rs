//! End-to-end HTTP integration tests for Prompt 12: Databases part A (link to apps).
//!
//! Verifies:
//! - Safe database port defaults (`internalOnly` / loopback) and `409 confirmation_required`
//!   refusal when exposing DB ports publicly (`0.0.0.0`) without `confirm: true`.
//! - `POST /api/v1/services/{id}/databases` and `DELETE /api/v1/services/{id}/databases/{db}`
//!   inserting/removing `${{ db.KEY }}` reference variables without ever exposing secrets in HTTP.
//! - Umami tool instantiation using `${{ pg.DATABASE_URL }}`.
//! - `GET /api/v1/projects/{id}` graph edges derived from `${{ ... }}` references.
//! - Secret reference resolution strictly inside `opsbox-priv` when app containers start.
//! - Credential rotation (`POST /api/v1/services/{id}/rotate` -> `PrivOp::DatabaseRotate`
//!   running `docker exec`), marking connected apps `needsRedeploy: true`, and picking up
//!   rotated secrets on redeploy.
//! - Invalid/self/cross-service cycle reference rejections and RBAC/token enforcement.

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use opsbox_api::{AppState, DeploySettings, SecureCookies, router};
use opsbox_auth::AuthConfig;
use opsbox_auth::password::HashParams;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::Role;
use opsbox_store::{SessionKind, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::oneshot;
use tower::ServiceExt;

const HOST: &str = "127.0.0.1:8443";
const ORIGIN: &str = "http://127.0.0.1:8443";

async fn read_http_req(stream: &mut UnixStream) -> Option<(String, String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let split = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let mut lines = head.lines();
    let req_line = lines.next()?;
    let mut parts = req_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let content_len: usize = lines
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    let mut body = buf[split + 4..].to_vec();
    while body.len() < content_len {
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_len);
    Some((method, target, body))
}

async fn write_http_json(stream: &mut UnixStream, status: u16, reason: &str, body: &Value) {
    let bytes = serde_json::to_vec(body).unwrap();
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.write_all(&bytes).await;
    let _ = stream.flush().await;
}

async fn write_http_empty(stream: &mut UnixStream, status: u16, reason: &str) {
    let resp =
        format!("HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.flush().await;
}

fn strip_api_prefix(path: &str) -> &str {
    if let Some(rest) = path.strip_prefix("/v")
        && rest.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && let Some(idx) = rest.find('/')
    {
        return &rest[idx..];
    }
    path
}

fn mux_frame(stream: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.push(stream);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&(u32::try_from(payload.len()).unwrap()).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

#[derive(Default)]
struct MockDockerState {
    volumes: HashSet<String>,
    networks: HashSet<String>,
    created_order: Vec<(String, Value)>,
    created: HashMap<String, Value>,
    removed: Vec<String>,
    exec_bodies: Vec<(String, Value)>,
    next_cid: usize,
    loopback_port: u16,
}

async fn send(
    app: &Router,
    method: Method,
    path: &str,
    cookie_or_bearer: &str,
    is_bearer: bool,
    csrf: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, HOST);
    if is_bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {cookie_or_bearer}"));
    } else if !cookie_or_bearer.is_empty() {
        builder = builder
            .header(header::COOKIE, cookie_or_bearer)
            .header(header::ORIGIN, ORIGIN);
        if !csrf.is_empty() {
            builder = builder.header("x-opsbox-csrf", csrf);
        }
    }
    let req_body = if let Some(v) = body {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        Body::from(serde_json::to_vec(&v).unwrap())
    } else {
        Body::empty()
    };
    let resp = app
        .clone()
        .oneshot(builder.body(req_body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

async fn wait_task_done(app: &Router, cookie: &str, csrf: &str, task_id: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..80 {
        let (st, val) = send(
            app,
            Method::GET,
            &format!("/api/v1/tasks/{task_id}"),
            cookie,
            false,
            csrf,
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "task lookup failed: {val}");
        last = val.clone();
        match val["status"].as_str().unwrap_or("") {
            "ok" | "done" => return val,
            "error" | "failed" => panic!("background task failed unexpectedly: {val}"),
            _ => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
    panic!("timed out waiting for task {task_id}: last={last}");
}

#[tokio::test]
async fn databases_link_rotation_and_port_guard_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::create_dir_all(root.join("apps")).unwrap();

    // Local HTTP listener so app & non-internal tool HTTP health probes succeed cleanly.
    let app_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_port = app_listener.local_addr().unwrap().port();
    let app_task = tokio::spawn(async move {
        while let Ok((mut s, _)) = app_listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            });
        }
    });

    // Mock Docker Engine Unix socket.
    let docker_state = Arc::new(Mutex::new(MockDockerState {
        loopback_port: app_port,
        ..Default::default()
    }));
    let ds = docker_state.clone();
    let docker_sock = root.join("docker.sock");
    let docker_listener = UnixListener::bind(&docker_sock).unwrap();
    let docker_task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = docker_listener.accept().await {
            let st = ds.clone();
            tokio::spawn(async move {
                let Some((method, target, body)) = read_http_req(&mut stream).await else {
                    return;
                };
                let (raw_path, _) = target.split_once('?').unwrap_or((&target, ""));
                let path = strip_api_prefix(raw_path);
                match (method.as_str(), path) {
                    ("GET", "/version") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({ "Version": "27.3.1", "ApiVersion": "1.47" }),
                        )
                        .await;
                    }
                    ("POST", "/images/create") => {
                        write_http_json(&mut stream, 200, "OK", &json!({"status": "Pulled"})).await;
                    }
                    ("POST", p) if p.starts_with("/images/") && p.ends_with("/tag") => {
                        write_http_empty(&mut stream, 201, "Created").await;
                    }
                    ("GET", p) if p.starts_with("/volumes/") => {
                        let name = p.trim_start_matches("/volumes/");
                        if st.lock().unwrap().volumes.contains(name) {
                            write_http_json(
                                &mut stream,
                                200,
                                "OK",
                                &json!({
                                    "Name": name,
                                    "Driver": "local",
                                    "Mountpoint": format!("/var/lib/docker/volumes/{name}/_data"),
                                    "Labels": {},
                                    "Scope": "local",
                                    "Options": {}
                                }),
                            )
                            .await;
                        } else {
                            write_http_json(
                                &mut stream,
                                404,
                                "Not Found",
                                &json!({"message": "missing volume"}),
                            )
                            .await;
                        }
                    }
                    ("POST", "/volumes/create") => {
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let name = v["Name"].as_str().unwrap_or("vol").to_owned();
                        st.lock().unwrap().volumes.insert(name.clone());
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({
                                "Name": name,
                                "Driver": "local",
                                "Mountpoint": format!("/var/lib/docker/volumes/{name}/_data"),
                                "Labels": {},
                                "Scope": "local",
                                "Options": {}
                            }),
                        )
                        .await;
                    }
                    ("GET", p) if p.starts_with("/networks/") => {
                        let name = p.trim_start_matches("/networks/");
                        if st.lock().unwrap().networks.contains(name) {
                            write_http_json(&mut stream, 200, "OK", &json!({ "Name": name })).await;
                        } else {
                            write_http_json(
                                &mut stream,
                                404,
                                "Not Found",
                                &json!({"message": "missing"}),
                            )
                            .await;
                        }
                    }
                    ("POST", "/networks/create") => {
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let name = v["Name"].as_str().unwrap_or("net").to_owned();
                        st.lock().unwrap().networks.insert(name.clone());
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({ "Id": format!("net-{name}"), "Warning": "" }),
                        )
                        .await;
                    }
                    ("POST", "/containers/create") => {
                        let parsed: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let cid = {
                            let mut guard = st.lock().unwrap();
                            guard.next_cid += 1;
                            let id = format!("cid-{}", guard.next_cid);
                            guard.created_order.push((id.clone(), parsed.clone()));
                            guard.created.insert(id.clone(), parsed);
                            id
                        };
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({"Id": cid, "Warnings": []}),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/start") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", p) if p.starts_with("/containers/") && p.ends_with("/json") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/json");
                        let (port_map, lp) = {
                            let guard = st.lock().unwrap();
                            let lp = guard.loopback_port;
                            let pm = guard
                                .created
                                .get(cid)
                                .and_then(|v| v["HostConfig"]["PortBindings"].as_object())
                                .map(|obj| {
                                    let mut map = serde_json::Map::new();
                                    for (key, val) in obj {
                                        let host_ip = val
                                            .as_array()
                                            .and_then(|a| a.first())
                                            .and_then(|o| o["HostIp"].as_str())
                                            .unwrap_or("127.0.0.1");
                                        map.insert(
                                            key.clone(),
                                            json!([{ "HostIp": host_ip, "HostPort": lp.to_string() }]),
                                        );
                                    }
                                    Value::Object(map)
                                })
                                .unwrap_or_else(|| json!({}));
                            (pm, lp)
                        };
                        let _ = lp;
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": cid,
                                "State": { "Running": true },
                                "NetworkSettings": { "Ports": port_map, "Networks": {} }
                            }),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/exec") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/exec")
                            .to_owned();
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        st.lock().unwrap().exec_bodies.push((cid, v));
                        write_http_json(&mut stream, 201, "Created", &json!({"Id": "exec-1"}))
                            .await;
                    }
                    ("POST", "/exec/exec-1/start") => {
                        let resp = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\nContent-Type: application/vnd.docker.raw-stream\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream
                            .write_all(&mux_frame(1, b"accepting connections\n"))
                            .await;
                        let _ = stream.flush().await;
                        let _ = stream.shutdown().await;
                    }
                    ("GET", "/exec/exec-1/json") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({"ExitCode": 0, "Running": false}),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/stop") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", p) if p.starts_with("/containers/") => {
                        let cid = p.trim_start_matches("/containers/").to_owned();
                        st.lock().unwrap().removed.push(cid);
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", p) if p.starts_with("/images/") => {
                        write_http_empty(&mut stream, 200, "OK").await;
                    }
                    _ => write_http_empty(&mut stream, 404, "Not Found").await,
                }
            });
        }
    });

    let fake_docker = root.join("docker");
    let script = r#"#!/bin/sh
echo '{"services":{"db":{"image":"postgres:16","ports":[{"published":"5432","target":5432}]}}}'
exit 0
"#;
    std::fs::write(&fake_docker, script).unwrap();
    std::fs::set_permissions(&fake_docker, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Real PrivServer.
    let priv_sock = root.join("priv.sock");
    let uid_probe = root.join("uid.probe");
    std::fs::write(&uid_probe, b"").unwrap();
    let uid = std::fs::metadata(&uid_probe).unwrap().uid();
    let mut cfg = PrivConfig::new(&priv_sock, vec![uid]);
    cfg.master_key_path = root.join("master.key");
    cfg.xfer_dir = root.join("xfer");
    cfg.trash_dir = root.join("trash");
    cfg.apps_dir = root.join("apps");
    cfg.docker_socket = docker_sock;
    cfg.docker_bin = Some(fake_docker);
    let srv = PrivServer::bind(cfg).unwrap();
    let (priv_stop, priv_rx) = oneshot::channel::<()>();
    let priv_task = tokio::spawn(async move {
        srv.run(async {
            let _ = priv_rx.await;
        })
        .await;
    });

    // AppState + Users + Token.
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
    let tok_a = "sess-alice-dblink-0001";
    let tok_v = "sess-bob-dblink-0001";
    store
        .create_session(
            &admin.id,
            &opsbox_auth::tokens::hash_token(tok_a),
            SessionKind::Web,
            None,
            None,
            now,
        )
        .unwrap();
    store
        .create_session(
            &viewer.id,
            &opsbox_auth::tokens::hash_token(tok_v),
            SessionKind::Web,
            None,
            None,
            now,
        )
        .unwrap();
    let admin_csrf = opsbox_auth::tokens::csrf_token(tok_a);
    let viewer_csrf = opsbox_auth::tokens::csrf_token(tok_v);
    let admin_cookie = format!("opsbox_session={tok_a}; opsbox_csrf={admin_csrf}");
    let viewer_cookie = format!("opsbox_session={tok_v}; opsbox_csrf={viewer_csrf}");

    let state = AppState::new(
        store.clone(),
        PrivClient::new(&priv_sock).with_timeout(Duration::from_secs(15)),
        "0.1.0-test",
        SecureCookies::Never,
        AuthConfig {
            hash_params: HashParams {
                memory_kib: 1024,
                iterations: 1,
                parallelism: 1,
            },
            ..Default::default()
        },
    )
    .with_files(opsbox_api::FilesSettings {
        apps_dir: root.join("apps"),
        xfer_dir: root.join("xfer"),
        max_upload: 2 << 20,
    })
    .with_deploy(DeploySettings {
        drain_secs: 0,
        build_timeout_secs: 30,
        health_retries: 2,
        allow_test_git: true,
    });
    let app = router(state);

    // 1. Database port safety default: unconfirmed publicPort=true MUST fail with 409 confirmation_required.
    let (st, val) = send(
        &app,
        Method::POST,
        "/api/v1/templates/postgres/instances",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "name": "pg-public-blocked", "publicPort": true })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "expected 409: {val}");
    assert_eq!(val["error"]["code"], "confirmation_required");
    assert!(store.get_service("pg-public-blocked").unwrap().is_none());

    // 2. Create PostgreSQL with safe defaults (`internalOnly: true`), capturing its one-time URL secret.
    let (st, pg_res) = send(
        &app,
        Method::POST,
        "/api/v1/templates/postgres/instances",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "name": "pg-main", "version": "16.4" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "pg create failed: {pg_res}");
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        pg_res["task"].as_str().unwrap(),
    )
    .await;
    let initial_pg_url = pg_res["connectionUrl"].as_str().unwrap().to_owned();
    let initial_pg_pass = initial_pg_url
        .split(':')
        .nth(2)
        .unwrap()
        .split('@')
        .next()
        .unwrap()
        .to_owned();
    assert!(!initial_pg_pass.is_empty());

    // 3. Confirming publicPort=true intentionally creates DB bound to 0.0.0.0.
    let (st, pub_pg_res) = send(
        &app,
        Method::POST,
        "/api/v1/templates/postgres/instances",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "name": "pg-pub", "publicPort": true, "confirm": true })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CREATED,
        "confirmed public pg failed: {pub_pg_res}"
    );
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        pub_pg_res["task"].as_str().unwrap(),
    )
    .await;
    {
        let ds_guard = docker_state.lock().unwrap();
        let pub_create = &ds_guard.created_order[1].1;
        assert_eq!(
            pub_create["HostConfig"]["PortBindings"]["5432/tcp"][0]["HostIp"],
            "0.0.0.0"
        );
    }

    // 4. Create prebuilt Docker app `api` + Umami referencing `${{ pg-main.DATABASE_URL }}`.
    let (st, api_res) = send(
        &app,
        Method::POST,
        "/api/v1/services",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({
            "sourceKind": "docker_image",
            "name": "api",
            "image": "nginx:1.27-alpine",
            "port": 3000
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "api create failed: {api_res}");
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        api_res["task"].as_str().unwrap(),
    )
    .await;

    let (st, umami_res) = send(
        &app,
        Method::POST,
        "/api/v1/templates/umami/instances",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({
            "name": "analytics",
            "databaseUrl": "${{ pg-main.DATABASE_URL }}"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "umami create failed: {umami_res}");
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        umami_res["task"].as_str().unwrap(),
    )
    .await;
    {
        let ds_guard = docker_state.lock().unwrap();
        let (_, umami_body) = ds_guard.created_order.last().unwrap();
        let env_arr = umami_body["Env"].as_array().unwrap();
        assert!(
            env_arr
                .iter()
                .any(|e| e == &format!("DATABASE_URL={initial_pg_url}")),
            "umami container must receive resolved DATABASE_URL: {env_arr:?}"
        );
    }

    // 5. Connect `pg-main` to running `api` via `POST /services/api/databases`.
    let (st, conn_res) = send(
        &app,
        Method::POST,
        "/api/v1/services/api/databases",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "database": "pg-main" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "connect failed: {conn_res}");
    assert_eq!(
        conn_res["vars"],
        json!([["DATABASE_URL", "${{ pg-main.DATABASE_URL }}", true, true]])
    );
    assert_eq!(conn_res["needsRedeploy"], json!(true));
    assert!(!conn_res.to_string().contains(&initial_pg_pass));

    // 6. Inspect project graph & services: edge exists, secret NEVER leaked, needsRedeploy accurate.
    let (st, proj) = send(
        &app,
        Method::GET,
        "/api/v1/projects/default",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let proj_str = proj.to_string();
    assert!(
        !proj_str.contains(&initial_pg_pass),
        "plaintext DB secret must never appear in GET /projects/default: {proj_str}"
    );
    let edges = proj["edges"].as_array().unwrap();
    assert!(edges.contains(&json!(["api", "pg-main"])), "{edges:?}");
    assert!(
        edges.contains(&json!(["analytics", "pg-main"])),
        "{edges:?}"
    );
    let api_svc = proj["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "api")
        .unwrap();
    assert!(api_svc["vars"].as_array().unwrap().contains(&json!([
        "DATABASE_URL",
        "${{ pg-main.DATABASE_URL }}",
        true,
        true
    ])));
    let pg_svc = proj["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "pg-main")
        .unwrap();
    let connected = pg_svc["connectedApps"].as_array().unwrap();
    assert_eq!(connected.len(), 2);
    assert_eq!(
        connected.iter().find(|c| c["id"] == "api").unwrap()["needsRedeploy"],
        json!(true)
    );
    assert_eq!(
        connected.iter().find(|c| c["id"] == "analytics").unwrap()["needsRedeploy"],
        json!(false)
    );

    // 7. Validation checks on invalid/self/missing/cycle references.
    for (bad_val, label) in [
        ("${{ api.PORT }}", "self reference"),
        ("${{ nonexistent.DATABASE_URL }}", "missing service"),
        ("${{ pg-main.NO_SUCH_KEY }}", "missing key"),
    ] {
        let (st, v) = send(
            &app,
            Method::PUT,
            "/api/v1/services/api/vars/BAD_REF",
            &admin_cookie,
            false,
            &admin_csrf,
            Some(json!({ "value": bad_val })),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{label} should fail: {v}");
    }
    // Create cycle test: analytics -> pg-main; ensure pg-main cannot reference analytics (and create B->A check).
    let (st, cyc) = send(
        &app,
        Method::PUT,
        "/api/v1/services/pg-main/vars/CYCLE_TEST",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "value": "${{ analytics.DATABASE_URL }}" })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::BAD_REQUEST,
        "cycle should be blocked: {cyc}"
    );

    // 8. Redeploy `api` -> container receives resolved initial URL and `needsRedeploy` clears.
    let (st, redep1) = send(
        &app,
        Method::POST,
        "/api/v1/services/api/deploys",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "redeploy failed: {redep1}");
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        redep1["task"].as_str().unwrap(),
    )
    .await;
    {
        let ds_guard = docker_state.lock().unwrap();
        let (_, api_body) = ds_guard.created_order.last().unwrap();
        let env_arr = api_body["Env"].as_array().unwrap();
        assert!(
            env_arr
                .iter()
                .any(|e| e == &format!("DATABASE_URL={initial_pg_url}")),
            "api container must get resolved DATABASE_URL: {env_arr:?}"
        );
    }

    // 9. Credential rotation: executes `docker exec` on pg-main, rotates sealed vars, marks connected apps stale.
    let (st, rot) = send(
        &app,
        Method::POST,
        "/api/v1/services/pg-main/rotate",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "rotate failed: {rot}");
    assert_eq!(
        rot["rotatedKeys"],
        json!(["POSTGRES_PASSWORD", "DATABASE_URL", "PGPASSWORD"])
    );
    let affected = rot["affectedApps"].as_array().unwrap();
    assert_eq!(affected.len(), 2);
    assert!(affected.iter().all(|a| a["needsRedeploy"] == json!(true)));
    {
        let ds_guard = docker_state.lock().unwrap();
        let (exec_cid, exec_body) = ds_guard.exec_bodies.last().unwrap();
        assert_eq!(exec_cid, "cid-1");
        assert_eq!(exec_body["Cmd"][0], "psql");
        assert!(
            exec_body["Env"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e == &format!("PGPASSWORD={initial_pg_pass}"))
        );
    }

    // 10. Redeploy `api` again -> receives NEW rotated DATABASE_URL (not the old one!), and clears only `api`'s stale flag.
    let (st, redep2) = send(
        &app,
        Method::POST,
        "/api/v1/services/api/deploys",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        redep2["task"].as_str().unwrap(),
    )
    .await;
    {
        let new_pass = {
            let ds_guard = docker_state.lock().unwrap();
            let (_, api_body2) = ds_guard.created_order.last().unwrap();
            let env_arr = api_body2["Env"].as_array().unwrap();
            let db_env = env_arr
                .iter()
                .filter_map(|v| v.as_str())
                .find(|s| s.starts_with("DATABASE_URL="))
                .expect("DATABASE_URL missing after rotation");
            assert_ne!(db_env, format!("DATABASE_URL={initial_pg_url}"));
            assert!(db_env.contains("@pg-main:5432/"));
            db_env
                .split(':')
                .nth(2)
                .unwrap()
                .split('@')
                .next()
                .unwrap()
                .to_owned()
        };
        assert_ne!(new_pass, initial_pg_pass);

        // Verify GET /projects/default NEVER leaks either old or new password.
        let (st, proj2) = send(
            &app,
            Method::GET,
            "/api/v1/projects/default",
            &admin_cookie,
            false,
            &admin_csrf,
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let p2s = proj2.to_string();
        assert!(!p2s.contains(&new_pass), "new rotated password leaked!");
        assert!(!p2s.contains(&initial_pg_pass), "old password leaked!");
        let pg2 = proj2["services"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "pg-main")
            .unwrap();
        let c2 = pg2["connectedApps"].as_array().unwrap();
        assert_eq!(
            c2.iter().find(|c| c["id"] == "api").unwrap()["needsRedeploy"],
            json!(false)
        );
        assert_eq!(
            c2.iter().find(|c| c["id"] == "analytics").unwrap()["needsRedeploy"],
            json!(true)
        );
    }

    // 11. Network exposure endpoint enforces confirmation when setting DB to `public`.
    let (st, exp_block) = send(
        &app,
        Method::PUT,
        "/api/v1/services/pg-main/exposure",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "mode": "public" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "{exp_block}");
    assert_eq!(exp_block["error"]["code"], "confirmation_required");

    let (st, exp_loop) = send(
        &app,
        Method::PUT,
        "/api/v1/services/pg-main/exposure",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "mode": "loopback" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{exp_loop}");
    assert_eq!(exp_loop["mode"], "loopback");
    assert_eq!(exp_loop["redeployRequired"], json!(true));

    // 12. RBAC + API token restrictions (even `full`-scope API tokens cannot connect/rotate/exposure).
    let (st_tok_create, tok_res) = send(
        &app,
        Method::POST,
        "/api/v1/tokens",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "name": "ci-full", "scope": "full" })),
    )
    .await;
    assert_eq!(st_tok_create, StatusCode::OK, "{tok_res}");
    let full_token = tok_res["secret"].as_str().unwrap().to_owned();

    for (method, path, body) in [
        (
            Method::POST,
            "/api/v1/services/api/databases",
            Some(json!({ "database": "pg-main" })),
        ),
        (
            Method::DELETE,
            "/api/v1/services/api/databases/pg-main",
            None,
        ),
        (Method::POST, "/api/v1/services/pg-main/rotate", None),
        (
            Method::PUT,
            "/api/v1/services/pg-main/exposure",
            Some(json!({ "mode": "internal" })),
        ),
    ] {
        let (st_v, _) = send(
            &app,
            method.clone(),
            path,
            &viewer_cookie,
            false,
            &viewer_csrf,
            body.clone(),
        )
        .await;
        assert_eq!(
            st_v,
            StatusCode::FORBIDDEN,
            "viewer must be forbidden on {path}"
        );
        let (st_t, _) = send(&app, method, path, &full_token, true, "", body).await;
        assert_eq!(
            st_t,
            StatusCode::FORBIDDEN,
            "api token must be forbidden on {path}"
        );
    }

    // 13. Disconnect removes link & edge.
    let (st, disc) = send(
        &app,
        Method::DELETE,
        "/api/v1/services/api/databases/pg-main",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{disc}");
    assert_eq!(disc["removedKeys"], json!(["DATABASE_URL"]));

    let (st, proj3) = send(
        &app,
        Method::GET,
        "/api/v1/projects/default",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let edges3 = proj3["edges"].as_array().unwrap();
    assert!(!edges3.contains(&json!(["api", "pg-main"])));
    assert!(edges3.contains(&json!(["analytics", "pg-main"])));

    // 14. Compose public database port without confirmation is refused with 409 confirmation_required.
    let (st_cmp, cmp_val) = send(
        &app,
        Method::POST,
        "/api/v1/compose",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({
            "stack": "exposed-db",
            "yaml": "services:\n  db:\n    image: postgres:16\n    ports:\n      - \"5432:5432\"\n"
        })),
    )
    .await;
    assert_eq!(st_cmp, StatusCode::CONFLICT, "{cmp_val}");
    assert_eq!(cmp_val["error"]["code"], "confirmation_required");

    let _ = priv_stop.send(());
    let _ = priv_task.await;
    docker_task.abort();
    app_task.abort();
}
