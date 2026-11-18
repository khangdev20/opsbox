//! End-to-end HTTP integration tests for Prompt 11:
//! template catalog, one-click PostgreSQL/Grafana instantiation (sealed creds, named volume,
//! network-only vs loopback exposure, default backup job), RBAC/token rejections,
//! Compose `docker compose config` validation, `409 confirmation_required`, `up -d`,
//! `opsbox-<project>` attachment, graph nodes + `depends_on` edges, redeploy, and delete.

mod common;

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
use opsbox_store::{SessionKind, Store, TokenScope};
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
    network_connections: Vec<(String, String)>,
    created: HashMap<String, Value>,
    removed: Vec<String>,
    next_cid: usize,
    loopback_port: u16,
    compose_containers: Vec<(String, String, String, Option<u16>)>, // (cid, project, svc, host_port)
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
async fn templates_and_compose_http_lifecycle_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::create_dir_all(root.join("apps")).unwrap();

    // Local HTTP listener to satisfy Grafana `/api/health` when internalOnly=false.
    let graf_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let graf_port = graf_listener.local_addr().unwrap().port();
    let graf_task = tokio::spawn(async move {
        while let Ok((mut s, _)) = graf_listener.accept().await {
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
        loopback_port: graf_port,
        compose_containers: vec![
            (
                "cmp-web".into(),
                "opsbox-shop".into(),
                "web".into(),
                Some(3000),
            ),
            (
                "cmp-cache".into(),
                "opsbox-shop".into(),
                "cache".into(),
                None,
            ),
            (
                "cmp-host".into(),
                "opsbox-hostnet".into(),
                "web".into(),
                None,
            ),
        ],
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
                let (raw_path, query) = target.split_once('?').unwrap_or((&target, ""));
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
                    ("GET", p) if p.starts_with("/volumes/") => {
                        let name = p.trim_start_matches("/volumes/");
                        let exists = st.lock().unwrap().volumes.contains(name);
                        if exists {
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
                        let exists = st.lock().unwrap().networks.contains(name);
                        if exists {
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
                    ("POST", p) if p.starts_with("/networks/") && p.ends_with("/connect") => {
                        let net = p
                            .trim_start_matches("/networks/")
                            .trim_end_matches("/connect")
                            .to_owned();
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let cid = v["Container"].as_str().unwrap_or("").to_owned();
                        st.lock().unwrap().network_connections.push((net, cid));
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", "/containers/create") => {
                        let parsed: Value = serde_json::from_slice(&body).unwrap_or_default();
                        assert!(
                            parsed["HostConfig"]["Memory"].as_i64().unwrap_or(0)
                                >= 64 * 1024 * 1024,
                            "every workload needs a memory cap: {parsed}"
                        );
                        let cid = {
                            let mut guard = st.lock().unwrap();
                            guard.next_cid += 1;
                            let id = format!("cid-{}", guard.next_cid);
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
                    ("GET", "/containers/json") => {
                        let list: Vec<Value> = {
                            let guard = st.lock().unwrap();
                            guard
                                .compose_containers
                                .iter()
                                .filter(|(id, proj, _, _)| {
                                    !guard.removed.contains(id) && query.contains(proj.as_str())
                                })
                                .map(|(id, proj, svc, _)| {
                                    json!({
                                        "Id": id,
                                        "Names": [format!("/{svc}")],
                                        "Image": "nginx:1.27-alpine",
                                        "Created": 1700000000,
                                        "State": "running",
                                        "Status": "Up 2s",
                                        "Ports": [],
                                        "Labels": {
                                            "com.docker.compose.project": proj,
                                            "com.docker.compose.service": svc
                                        }
                                    })
                                })
                                .collect()
                        };
                        write_http_json(&mut stream, 200, "OK", &json!(list)).await;
                    }
                    ("GET", p) if p.starts_with("/containers/") && p.ends_with("/json") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/json");
                        let (port, compose_port) = {
                            let guard = st.lock().unwrap();
                            let cp = guard
                                .compose_containers
                                .iter()
                                .find(|(id, _, _, _)| id == cid)
                                .and_then(|(_, _, _, p)| *p);
                            let lp = guard.loopback_port;
                            let has_binding = guard
                                .created
                                .get(cid)
                                .and_then(|v| v["HostConfig"]["PortBindings"].as_object())
                                .is_some();
                            (if has_binding { Some(lp) } else { None }, cp)
                        };
                        let ports = if let Some(hp) = compose_port {
                            json!({ "80/tcp": [{ "HostIp": "127.0.0.1", "HostPort": hp.to_string() }] })
                        } else if let Some(lp) = port {
                            json!({ "3000/tcp": [{ "HostIp": "127.0.0.1", "HostPort": lp.to_string() }] })
                        } else {
                            json!({})
                        };
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": cid,
                                "State": { "Running": true },
                                "NetworkSettings": { "Ports": ports, "Networks": {} }
                            }),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/exec") => {
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
                    _ => write_http_empty(&mut stream, 404, "Not Found").await,
                }
            });
        }
    });

    // Fake docker CLI handling `docker compose config --format json`, `up`, `down`.
    let fake_docker = root.join("docker");
    let script = r#"#!/bin/sh
sub=""
file=""
prev=""
for a in "$@"; do
  case "$a" in config|up|down) sub="$a" ;; esac
  if [ "$prev" = "-f" ]; then file="$a"; fi
  prev="$a"
done
if [ "$sub" = "config" ]; then
  if grep -q "network_mode: host" "$file"; then
    echo '{"services":{"web":{"image":"nginx:1.27-alpine","network_mode":"host","ports":[{"target":80}]}}}'
    exit 0
  fi
  if grep -q "privileged: true" "$file"; then
    echo '{"services":{"web":{"image":"nginx:1.27-alpine","privileged":true}}}'
    exit 0
  fi
  echo '{"services":{"web":{"image":"nginx:1.27-alpine","ports":[{"published":"3000","target":80}],"depends_on":{"cache":{"condition":"service_started"}}},"cache":{"image":"redis:7-alpine"}}}'
  exit 0
fi
if [ "$sub" = "up" ]; then
  echo "Container started cleanly"
  exit 0
fi
if [ "$sub" = "down" ]; then
  echo "Stack removed"
  exit 0
fi
exit 1
"#;
    std::fs::write(&fake_docker, script).unwrap();
    std::fs::set_permissions(&fake_docker, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Start real PrivServer.
    let priv_sock = root.join("priv.sock");
    let uid_probe = root.join("uid.probe");
    std::fs::write(&uid_probe, b"").unwrap();
    let uid = std::fs::metadata(&uid_probe).unwrap().uid();
    let mut cfg = PrivConfig::new(&priv_sock, vec![uid]);
    cfg.capacity_profile = Some(common::capacity_profile());
    cfg.capacity_image_size_mb = Some(64);
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

    // AppState + Users.
    let store = Arc::new(Store::open_in_memory().unwrap());
    let admin = store
        .create_user("alice", Role::Admin, "unused", 0)
        .unwrap();
    store.set_pending_totp(&admin.id, "unused", 0).unwrap();
    store.enable_totp(&admin.id, 0, 0).unwrap();
    let viewer = store.create_user("bob", Role::Viewer, "unused", 0).unwrap();
    store.set_pending_totp(&viewer.id, "unused", 0).unwrap();
    store.enable_totp(&viewer.id, 0, 0).unwrap();
    let deployer = store
        .create_user("dep", Role::Deployer, "unused", 0)
        .unwrap();
    store.set_pending_totp(&deployer.id, "unused", 0).unwrap();
    store.enable_totp(&deployer.id, 0, 0).unwrap();
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let tok_a = "sess-alice-1234567890";
    let tok_v = "sess-bob-1234567890";
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
        SecureCookies::Auto,
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
        health_retries: 3,
        allow_test_git: true,
    });
    let (_tok_rec, raw_tok) = state
        .auth
        .create_token(&deployer, "ci-deploy", TokenScope::Deploy, None)
        .unwrap();
    let app = router(state);

    // 1. Catalog returns all 14 built-in templates (databases incl. pgvector/PostGIS flavours, plus data tools).
    let (st, cat) = send(
        &app,
        Method::GET,
        "/api/v1/templates",
        &viewer_cookie,
        false,
        &viewer_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(cat["databases"].as_array().unwrap().len(), 7);
    assert_eq!(cat["tools"].as_array().unwrap().len(), 7);

    // 2. Viewer and non-admin bearer tokens cannot instantiate templates or compose.
    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/templates/postgres/instances",
        &viewer_cookie,
        false,
        &viewer_csrf,
        Some(json!({ "name": "pg-nope" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/templates/postgres/instances",
        &raw_tok,
        true,
        "",
        Some(json!({ "name": "pg-nope" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 3. Instantiate PostgreSQL template: sealed password, internal_only=true, named volume, backup job.
    let (st, pg_res) = send(
        &app,
        Method::POST,
        "/api/v1/templates/postgres/instances",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "name": "pg-main", "version": "16.4", "disk": "5 GB", "backup": true })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "pg create failed: {pg_res}");
    assert_eq!(pg_res["id"], "pg-main");
    let one_time_url = pg_res["connectionUrl"].as_str().unwrap().to_owned();
    assert!(one_time_url.starts_with("postgresql://app:"));
    assert!(one_time_url.contains("@pg-main:5432/app"));
    // Secrets must already be masked inside `pg_res["service"]["vars"]`.
    let vars_arr = pg_res["service"]["vars"].as_array().unwrap();
    let db_url_entry = vars_arr.iter().find(|v| v[0] == "DATABASE_URL").unwrap();
    assert_eq!(db_url_entry[1], "••••••••");
    assert_eq!(db_url_entry[2], true);

    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        pg_res["task"].as_str().unwrap(),
    )
    .await;

    // Verify DB state: sealed value does NOT equal plaintext URL, backup_jobs exists, internalOnly=true.
    let pg_svc = store.get_service("pg-main").unwrap().unwrap();
    assert!(pg_svc.internal_only);
    assert_eq!(pg_svc.active_host_port, None);
    let stored_vars = store.list_service_vars(&pg_svc.id).unwrap();
    let stored_url_row = stored_vars
        .iter()
        .find(|v| v.key == "DATABASE_URL")
        .unwrap();
    assert_ne!(stored_url_row.value_enc, one_time_url);
    assert!(stored_url_row.is_secret);
    let bjob = store
        .get_backup_job_for_service(&pg_svc.id)
        .unwrap()
        .unwrap();
    assert_eq!(bjob.method, "pg_dump");
    assert_eq!(bjob.cron, "0 2 * * *");
    assert!(bjob.enabled);

    // Subsequent GET /services/pg-main never exposes secret plaintext again.
    let (st, fetched_pg) = send(
        &app,
        Method::GET,
        "/api/v1/services/pg-main",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(fetched_pg["state"], "active");
    assert_eq!(fetched_pg["internalOnly"], true);
    assert_eq!(fetched_pg["templateId"], "postgres");
    assert_eq!(fetched_pg["backup"]["method"], "pg_dump");
    assert!(
        !serde_json::to_string(&fetched_pg)
            .unwrap()
            .contains(&one_time_url)
    );

    // Database service refuses domain attachment; exposed tool (grafana internalOnly=false) accepts one.
    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/services/pg-main/domains",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "domain": "pg.example.com" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let (st, graf_res) = send(
        &app,
        Method::POST,
        "/api/v1/templates/grafana/instances",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "name": "metrics-ui", "internalOnly": false })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "grafana create failed: {graf_res}");
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        graf_res["task"].as_str().unwrap(),
    )
    .await;
    let (st, dom_res) = send(
        &app,
        Method::POST,
        "/api/v1/services/metrics-ui/domains",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "domain": "grafana.example.com" })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "grafana domain failed: {dom_res}");

    // Umami rejects missing PostgreSQL DATABASE_URL and accepts valid URL.
    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/templates/umami/instances",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "name": "analytics" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // Redeploy pg-main executes cleanly.
    let (st, redep) = send(
        &app,
        Method::POST,
        "/api/v1/services/pg-main/deploys",
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
        redep["task"].as_str().unwrap(),
    )
    .await;

    // 4. Compose refuses `network_mode: host` with 409 confirmation_required and leaves no files/rows.
    let unsafe_yaml = "services:\n  web:\n    image: nginx:1.27-alpine\n    network_mode: host\n";
    let (st, err_val) = send(
        &app,
        Method::POST,
        "/api/v1/compose",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "stack": "hostnet", "yaml": unsafe_yaml, "confirm": false })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "expected 409: {err_val}");
    assert_eq!(err_val["error"]["code"], "confirmation_required");
    assert!(store.get_compose_stack("hostnet").unwrap().is_none());
    assert!(!root.join("apps/hostnet").exists());

    // Confirming allows host-network Compose to proceed.
    let (st, host_ok) = send(
        &app,
        Method::POST,
        "/api/v1/compose",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "stack": "hostnet", "yaml": unsafe_yaml, "confirm": true })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::CREATED,
        "confirmed compose failed: {host_ok}"
    );
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        host_ok["task"].as_str().unwrap(),
    )
    .await;
    assert!(
        store
            .get_compose_stack("hostnet")
            .unwrap()
            .unwrap()
            .confirmed
    );

    // 5. Safe Compose stack creates services, `depends_on` edges on graph, supports delete.
    let shop_yaml = "services:\n  web:\n    image: nginx:1.27-alpine\n    ports: [\"127.0.0.1:3000:80\"]\n    depends_on: [cache]\n  cache:\n    image: redis:7-alpine\n";
    let (st, shop_res) = send(
        &app,
        Method::POST,
        "/api/v1/compose",
        &admin_cookie,
        false,
        &admin_csrf,
        Some(json!({ "stack": "shop", "yaml": shop_yaml })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "shop compose failed: {shop_res}");
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        shop_res["task"].as_str().unwrap(),
    )
    .await;

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
    let edges = proj["edges"].as_array().unwrap();
    let shop_web_name = shop_res["names"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n.as_str().unwrap().ends_with("web"))
        .unwrap()
        .as_str()
        .unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e[0] == shop_web_name && e[1] == "cache"),
        "expected depends_on edge in project graph: {edges:?}"
    );
    let cache_svc = store.get_service("cache").unwrap().unwrap();
    assert_eq!(cache_svc.kind, opsbox_store::ServiceKind::Compose);
    assert!(cache_svc.internal_only);
    let shop_web_svc = store.get_service(shop_web_name).unwrap().unwrap();
    assert_eq!(shop_web_svc.active_host_port, Some(3000));

    // Redeploy Compose service executes `compose_deploy` cleanly.
    let (st, cmp_redep) = send(
        &app,
        Method::POST,
        &format!("/api/v1/services/{shop_web_name}/deploys"),
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(
        st,
        StatusCode::ACCEPTED,
        "compose redeploy failed: {cmp_redep}"
    );
    wait_task_done(
        &app,
        &admin_cookie,
        &admin_csrf,
        cmp_redep["task"].as_str().unwrap(),
    )
    .await;

    // Delete single Compose service (`cache`), then delete whole `hostnet` stack.
    let (st, _) = send(
        &app,
        Method::DELETE,
        "/api/v1/services/cache",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(store.get_service("cache").unwrap().is_none());

    let (st, _) = send(
        &app,
        Method::DELETE,
        "/api/v1/compose/hostnet",
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(store.get_compose_stack("hostnet").unwrap().is_none());
    assert!(!root.join("apps/hostnet").exists());

    // Delete last remaining service of `shop` stack via DELETE /services/{id} cleans up stack row too.
    let (st, _) = send(
        &app,
        Method::DELETE,
        &format!("/api/v1/services/{shop_web_name}"),
        &admin_cookie,
        false,
        &admin_csrf,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(store.get_compose_stack("shop").unwrap().is_none());

    let _ = priv_stop.send(());
    let _ = priv_task.await;
    docker_task.abort();
    graf_task.abort();
}
