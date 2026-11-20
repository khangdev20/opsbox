//! End-to-end integration tests for Prompt 09 deploy pipeline:
//! create -> clone -> BuildKit build with `--secret` mounts & log redaction ->
//! loopback HTTP health check -> zero-downtime proxy switch -> broken deploy keeps old live ->
//! rollback without rebuild -> deployer & `deploy:<svc>` token scoping -> webhook push auto-deploy.

mod common;

use std::collections::HashMap;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::{Method, StatusCode};
use futures_util::StreamExt;
use hmac::{Hmac, KeyInit, Mac};
use opsbox_api::{AppState, DeploySettings, SecureCookies, router};
use opsbox_auth::AuthConfig;
use opsbox_auth::password::HashParams;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::Role;
use opsbox_proxy::{AcmeDirectory, ProxyClient, ProxyConfig, ProxyServer};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn init_local_git_repo(dir: &std::path::Path, content: &str, msg: &str) -> String {
    Command::new("git")
        .args(["init", "-b", "main"])
        .arg(dir)
        .output()
        .unwrap();
    std::fs::write(
        dir.join("Dockerfile"),
        format!("FROM scratch\n# {content}\n"),
    )
    .unwrap();
    for args in [
        vec!["config", "user.name", "Opsbox Test"],
        vec!["config", "user.email", "test@opsbox.invalid"],
        vec!["add", "."],
        vec!["commit", "-m", msg],
    ] {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let head = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&head.stdout).trim().to_owned()
}

fn commit_local_git_repo(dir: &std::path::Path, content: &str, msg: &str) -> String {
    std::fs::write(
        dir.join("Dockerfile"),
        format!("FROM scratch\n# {content}\n"),
    )
    .unwrap();
    for args in [vec!["add", "."], vec!["commit", "-m", msg]] {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let head = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&head.stdout).trim().to_owned()
}

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

async fn spawn_upstream_app(
    body_text: &'static str,
    status_code: Arc<Mutex<u16>>,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let sc = status_code.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf).await;
                let code = *sc.lock().unwrap();
                let resp = format!(
                    "HTTP/1.1 {code} STATUS\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body_text}",
                    body_text.len()
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });
    (port, task)
}

#[derive(Default)]
struct MockDockerState {
    next_cid: usize,
    /// Map image_ref -> upstream loopback port
    image_ports: HashMap<String, u16>,
    /// Default port if image not explicitly mapped (e.g. during build #1..N)
    ordered_ports: Vec<u16>,
    build_port_cursor: usize,
    containers: HashMap<String, (String, u16)>, // cid -> (image, port)
    stopped: Vec<String>,
    removed_images: Vec<String>,
    pulled_images: Vec<String>,
}

async fn http_get_host(addr: std::net::SocketAddr, host: &str, path: &str) -> (u16, String) {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let raw = String::from_utf8_lossy(&buf).into_owned();
    let status = raw
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

fn sign_webhook(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("sha256={hex}")
}

async fn wait_task_done(base_url: &str, cookie_or_bearer: (&str, bool), task_id: &str) -> Value {
    for _ in 0..100 {
        let r = req_json(
            base_url,
            Method::GET,
            &format!("/api/v1/tasks/{task_id}"),
            cookie_or_bearer,
            None,
        )
        .await;
        assert_eq!(r.0, StatusCode::OK, "{}", r.1);
        if r.1["status"] != "running" {
            return r.1;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("task {task_id} timed out");
}

async fn req_json(
    base_url: &str,
    method: Method,
    path: &str,
    auth: (&str, bool), // (token, is_bearer)
    body: Option<Value>,
) -> (StatusCode, Value) {
    reqwest_like_hyper(base_url, method, path, auth, body).await
}

async fn reqwest_like_hyper(
    addr: &str,
    method: Method,
    path: &str,
    (tok, is_bearer): (&str, bool),
    body: Option<Value>,
) -> (StatusCode, Value) {
    let tcp = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut s = tcp;
    let payload = body
        .map(|v| serde_json::to_vec(&v).unwrap())
        .unwrap_or_default();
    let mut hdr = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if is_bearer {
        hdr.push_str(&format!("Authorization: Bearer {tok}\r\n"));
    } else if !tok.is_empty() {
        hdr.push_str(&format!("Cookie: {tok}\r\nOrigin: http://{addr}\r\n"));
        if let Some((csrf_k, csrf_v)) = tok.split(';').find_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            (k == "opsbox_csrf").then_some((k, v))
        }) {
            let _ = csrf_k;
            hdr.push_str(&format!("X-Opsbox-CSRF: {csrf_v}\r\n"));
        }
    }
    if !payload.is_empty() {
        hdr.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            payload.len()
        ));
    }
    hdr.push_str("\r\n");
    s.write_all(hdr.as_bytes()).await.unwrap();
    if !payload.is_empty() {
        s.write_all(&payload).await.unwrap();
    }
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    let (head, bdy) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let code = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(500);
    let val = serde_json::from_str(bdy).unwrap_or_else(|_| json!({ "raw": bdy }));
    (StatusCode::from_u16(code).unwrap(), val)
}

#[tokio::test]
async fn deploy_build_proxy_switch_health_failure_rollback_tokens_and_webhook_e2e() {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::create_dir_all(root.join("apps")).unwrap();

    // Two version upstreams + one unhealthy v3 upstream.
    let s1 = Arc::new(Mutex::new(200u16));
    let s2 = Arc::new(Mutex::new(200u16));
    let s3 = Arc::new(Mutex::new(500u16));
    let (p1, t1) = spawn_upstream_app("version-1", s1).await;
    let (p2, t2) = spawn_upstream_app("version-2", s2).await;
    let (p3, t3) = spawn_upstream_app("broken-3", s3).await;

    let repo_dir = root.join("origin-repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let sha1 = init_local_git_repo(&repo_dir, "v1", "Release v1");

    // Fake docker CLI tracking how many times `docker build` runs and checking BuildKit secret mounts.
    let build_count_file = root.join("build_count.txt");
    std::fs::write(&build_count_file, "").unwrap();
    let args_capture = root.join("last_build_args.txt");
    let fake_docker = root.join("docker");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "buildx" ] && {{ [ "$2" = "create" ] || [ "$2" = "rm" ]; }}; then exit 0; fi
if [ "$1" = "buildx" ] && [ "$2" = "version" ]; then
  echo "buildx v0.17.1"
  exit 0
fi
if [ "$3" = "--help" ] || [ "$2" = "--help" ]; then
  echo "  --progress string"
  exit 0
fi
echo "1" >> "{build_count_file}"
printf '%s\n' "$@" > "{args_capture}"
echo "Building with super-secret-db-pass-42"
exit 0
"#,
        build_count_file = build_count_file.display(),
        args_capture = args_capture.display(),
    );
    std::fs::write(&fake_docker, script).unwrap();
    std::fs::set_permissions(&fake_docker, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Mock Docker Engine Unix socket.
    let docker_state = Arc::new(Mutex::new(MockDockerState {
        ordered_ports: vec![p1, p2, p3],
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
                let path = strip_api_prefix(target.split('?').next().unwrap_or(&target));
                match (method.as_str(), path) {
                    ("POST", "/images/create") => {
                        st.lock().unwrap().pulled_images.push(target.clone());
                        write_http_json(&mut stream, 200, "OK", &json!({"status": "pulled"})).await;
                    }
                    ("POST", p) if p.starts_with("/images/") && p.ends_with("/tag") => {
                        write_http_empty(&mut stream, 201, "Created").await;
                    }
                    ("GET", "/version") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({ "Version": "27.3.1", "ApiVersion": "1.47" }),
                        )
                        .await;
                    }
                    ("GET", p) if p.starts_with("/networks/") => {
                        write_http_json(
                            &mut stream,
                            404,
                            "Not Found",
                            &json!({"message": "missing"}),
                        )
                        .await;
                    }
                    ("POST", "/networks/create") => {
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({"Id": "net-default", "Warning": ""}),
                        )
                        .await;
                    }
                    ("POST", "/containers/create") => {
                        let parsed: Value = serde_json::from_slice(&body).unwrap_or_default();
                        assert!(
                            parsed["HostConfig"]["Memory"].as_i64().unwrap_or(0)
                                >= 64 * 1024 * 1024,
                            "every workload needs a memory cap: {parsed}"
                        );
                        let img = parsed["Image"].as_str().unwrap_or("").to_owned();
                        let cid = {
                            let mut guard = st.lock().unwrap();
                            guard.next_cid += 1;
                            let cid = format!("cid-{}", guard.next_cid);
                            let port = if let Some(&existing) = guard.image_ports.get(&img) {
                                existing
                            } else {
                                let idx =
                                    guard.build_port_cursor.min(guard.ordered_ports.len() - 1);
                                guard.build_port_cursor += 1;
                                let p = guard.ordered_ports[idx];
                                guard.image_ports.insert(img.clone(), p);
                                p
                            };
                            guard.containers.insert(cid.clone(), (img, port));
                            cid
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
                        let port = st
                            .lock()
                            .unwrap()
                            .containers
                            .get(cid)
                            .map(|c| c.1)
                            .unwrap_or(80);
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": cid,
                                "State": { "Running": true },
                                "NetworkSettings": {
                                    "Ports": {
                                        "3000/tcp": [{ "HostIp": "127.0.0.1", "HostPort": port.to_string() }]
                                    }
                                }
                            }),
                        )
                        .await;
                    }
                    ("GET", "/containers/json") => {
                        write_http_json(&mut stream, 200, "OK", &json!([])).await;
                    }
                    ("GET", p) if p.starts_with("/containers/") && p.ends_with("/logs") => {
                        write_http_empty(&mut stream, 200, "OK").await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/stop") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/stop")
                            .to_owned();
                        st.lock().unwrap().stopped.push(cid);
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", p) if p.starts_with("/containers/") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", p) if p.starts_with("/images/") && p.ends_with("/json") => {
                        write_http_json(&mut stream, 200, "OK", &json!({"Id": "sha256:retained"}))
                            .await;
                    }
                    ("DELETE", p) if p.starts_with("/images/") => {
                        st.lock().unwrap().removed_images.push(p.to_owned());
                        write_http_json(&mut stream, 200, "OK", &json!([])).await;
                    }
                    _ => write_http_empty(&mut stream, 404, "Not Found").await,
                }
            });
        }
    });

    // Real PrivServer
    let probe = root.join("probe");
    std::fs::write(&probe, b"").unwrap();
    let uid = std::fs::metadata(&probe).unwrap().uid();
    let priv_sock = root.join("priv.sock");
    let mut priv_cfg = PrivConfig::new(&priv_sock, vec![uid]);
    priv_cfg.capacity_profile = Some(common::capacity_profile());
    priv_cfg.capacity_image_size_mb = Some(64);
    priv_cfg.master_key_path = root.join("master.key");
    priv_cfg.xfer_dir = root.join("xfer");
    priv_cfg.trash_dir = root.join("trash");
    priv_cfg.apps_dir = root.join("apps");
    priv_cfg.docker_socket = docker_sock;
    priv_cfg.docker_bin = Some(fake_docker);
    priv_cfg.allow_test_git = true;
    let priv_srv = PrivServer::bind(priv_cfg).unwrap();
    let (priv_stop, priv_rx) = oneshot::channel::<()>();
    let priv_task = tokio::spawn(priv_srv.run(async {
        let _ = priv_rx.await;
    }));

    // Real ProxyServer
    let proxy_sock = root.join("proxy.sock");
    let mut proxy_cfg = ProxyConfig::new(&priv_sock);
    proxy_cfg.listen_http = "127.0.0.1:0".parse().unwrap();
    proxy_cfg.listen_https = "127.0.0.1:0".parse().unwrap();
    proxy_cfg.socket_path = proxy_sock.clone();
    proxy_cfg.state_dir = root.join("proxy-state");
    proxy_cfg.acme_directory = AcmeDirectory::Staging;
    let proxy_srv = ProxyServer::bind(proxy_cfg).await.unwrap();
    let proxy_http_addr = proxy_srv.http_addr();
    let (proxy_stop, proxy_rx) = oneshot::channel::<()>();
    let proxy_task = tokio::spawn(async move {
        proxy_srv
            .run(async {
                let _ = proxy_rx.await;
            })
            .await
            .unwrap();
    });

    // AppState + real TCP Axum server
    let store = Arc::new(opsbox_store::Store::open_in_memory().unwrap());
    let u = store.create_user("boss", Role::Admin, "unused", 0).unwrap();
    store.set_pending_totp(&u.id, "unused", 0).unwrap();
    store.enable_totp(&u.id, 0, 0).unwrap();

    let priv_client = PrivClient::new(&priv_sock).with_timeout(Duration::from_secs(10));
    let auth_cfg = AuthConfig {
        hash_params: HashParams {
            memory_kib: 1024,
            iterations: 1,
            parallelism: 1,
        },
        ..Default::default()
    };
    let state = AppState::new(
        store.clone(),
        priv_client,
        "0.1.0-test",
        SecureCookies::Auto,
        auth_cfg,
    )
    .with_files(opsbox_api::FilesSettings {
        apps_dir: root.join("apps"),
        xfer_dir: root.join("xfer"),
        max_upload: 2 << 20,
    })
    .with_proxy(
        ProxyClient::new(&proxy_sock).with_timeout(Duration::from_secs(5)),
        None,
        vec![],
    )
    .with_deploy(DeploySettings {
        drain_secs: 0,
        build_timeout_secs: 30,
        health_retries: 2,
        allow_test_git: true,
    });
    let admin = store
        .create_user("alice", Role::Admin, "unused", 0)
        .unwrap();
    store.set_pending_totp(&admin.id, "unused", 0).unwrap();
    store.enable_totp(&admin.id, 0, 0).unwrap();
    let d1 = store
        .create_user("d1", Role::Deployer, "unused", 0)
        .unwrap();
    let d2 = store
        .create_user("d2", Role::Deployer, "unused", 0)
        .unwrap();

    let (t_full_raw, t_admin_full) = state
        .auth
        .create_token(&admin, "test-full", opsbox_store::TokenScope::Full, None)
        .unwrap();
    let _ = (t_full_raw, t_admin_full, u);

    let now_epoch = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap();
    let tok_a = "sess-alice-1234567890";
    let tok_d1 = "sess-dep1-1234567890";
    let tok_d2 = "sess-dep2-1234567890";
    for (uid_ref, tok) in [(&admin.id, tok_a), (&d1.id, tok_d1), (&d2.id, tok_d2)] {
        let th = opsbox_auth::tokens::hash_token(tok);
        store
            .create_session(
                uid_ref,
                &th,
                opsbox_store::SessionKind::Web,
                None,
                None,
                now_epoch,
            )
            .unwrap();
    }
    let cookie_a = format!(
        "opsbox_session={tok_a}; opsbox_csrf={}",
        opsbox_auth::tokens::csrf_token(tok_a)
    );
    let cookie_d1 = format!(
        "opsbox_session={tok_d1}; opsbox_csrf={}",
        opsbox_auth::tokens::csrf_token(tok_d1)
    );
    let cookie_d2 = format!(
        "opsbox_session={tok_d2}; opsbox_csrf={}",
        opsbox_auth::tokens::csrf_token(tok_d2)
    );

    let app = router(state.clone());
    let api_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_addr = api_listener.local_addr().unwrap().to_string();
    let api_task = tokio::spawn(async move {
        let svc = app.into_make_service_with_connect_info::<std::net::SocketAddr>();
        let _ = axum::serve(api_listener, svc).await;
    });

    // Wait: verify session cookie actually uses `store.create_session(uid, kind, ip, ua, now)` signature!
    // Check `store.create_session` vs `store.session_by_hash` below after compiling once if needed!

    // 1. Deployer d1 creates service `web` with secret + plain var + domain `app.example.com`.
    let (st_create, created) = req_json(
        &api_addr,
        Method::POST,
        "/api/v1/services",
        (&cookie_d1, false),
        Some(json!({
            "name": "web",
            "sourceKind": "git_url",
            "repo": format!("file://{}", repo_dir.display()),
            "branch": "main",
            "port": 3000,
            "domain": "app.example.com",
            "vars": [
                ["SECRET_KEY", "super-secret-db-pass-42", true],
                ["APP_MODE", "live", false]
            ]
        })),
    )
    .await;
    assert_eq!(st_create, StatusCode::CREATED, "{created}");
    let d1_id = created["deployId"].as_str().unwrap().to_owned();
    let t1_id = created["task"].as_str().unwrap().to_owned();

    // Connect WebSocket to stream live deploy log!
    let mut ws_req = format!("ws://{api_addr}/api/v1/stream/deploys/{d1_id}")
        .into_client_request()
        .unwrap();
    ws_req
        .headers_mut()
        .insert("Origin", format!("http://{api_addr}").parse().unwrap());
    ws_req
        .headers_mut()
        .insert("Cookie", cookie_d1.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_req).await.unwrap();
    let mut ws_lines = Vec::new();
    let mut ws_done_status = String::new();
    while let Some(msg) = tokio::time::timeout(Duration::from_secs(5), ws.next())
        .await
        .unwrap()
    {
        let txt = msg.unwrap().into_text().unwrap();
        let val: Value = serde_json::from_str(&txt).unwrap();
        if val["type"] == "log" {
            ws_lines.push(val["line"].as_str().unwrap().to_owned());
        } else if val["type"] == "done" {
            ws_done_status = val["status"].as_str().unwrap().to_owned();
            break;
        }
    }
    assert_eq!(ws_done_status, "live");
    let joined_ws = ws_lines.join("\n");
    assert!(
        !joined_ws.contains("super-secret-db-pass-42"),
        "secret leaked via WS: {joined_ws}"
    );
    assert!(joined_ws.contains("[REDACTED]"), "{joined_ws}");

    let t1_res = wait_task_done(&api_addr, (&cookie_d1, false), &t1_id).await;
    assert_eq!(t1_res["status"], "ok");

    // Verify BuildKit secret mount args & non-secret build args.
    let b_args = std::fs::read_to_string(&args_capture).unwrap();
    assert!(
        b_args.contains("--secret") && b_args.contains("id=SECRET_KEY,src="),
        "{b_args}"
    );
    assert!(b_args.contains("APP_MODE=live"), "{b_args}");
    assert!(!b_args.contains("super-secret-db-pass-42"), "{b_args}");

    // Proxy serves version-1 on app.example.com!
    let (ps1, pb1) = http_get_host(proxy_http_addr, "app.example.com", "/").await;
    assert_eq!((ps1, pb1.as_str()), (200, "version-1"));

    // 2. Push commit v2 & redeploy -> proxy switches cleanly to version-2, cid-1 stopped.
    let sha2 = commit_local_git_repo(&repo_dir, "v2", "Release v2");
    let (st_red, red) = req_json(
        &api_addr,
        Method::POST,
        "/api/v1/services/web/deploys",
        (&cookie_d1, false),
        None,
    )
    .await;
    assert_eq!(st_red, StatusCode::ACCEPTED, "{red}");
    let d2_id = red["id"].as_str().unwrap().to_owned();
    let t2_res = wait_task_done(
        &api_addr,
        (&cookie_d1, false),
        red["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(t2_res["status"], "ok");
    let (ps2, pb2) = http_get_host(proxy_http_addr, "app.example.com", "/").await;
    assert_eq!((ps2, pb2.as_str()), (200, "version-2"));
    assert!(
        docker_state
            .lock()
            .unwrap()
            .stopped
            .contains(&"cid-1".to_owned())
    );

    // Verify commit SHA updated on d2 & project view masks SECRET_KEY.
    let (_, proj_view) = req_json(
        &api_addr,
        Method::GET,
        "/api/v1/projects/default",
        (&cookie_d1, false),
        None,
    )
    .await;
    assert_eq!(proj_view["services"][0]["deploys"][0]["commit"], sha2);
    assert_eq!(
        proj_view["services"][0]["vars"][1],
        json!(["SECRET_KEY", "••••••••", true])
    );
    assert_eq!(
        proj_view["services"][0]["vars"][0],
        json!(["APP_MODE", "live", false])
    );

    // 3. Broken deploy v3 fails health check -> version-2 stays live & Attention item raised!
    commit_local_git_repo(&repo_dir, "v3", "Broken v3");
    let (st_bad, bad) = req_json(
        &api_addr,
        Method::POST,
        "/api/v1/services/web/deploys",
        (&cookie_d1, false),
        None,
    )
    .await;
    assert_eq!(st_bad, StatusCode::ACCEPTED);
    let t3_res = wait_task_done(
        &api_addr,
        (&cookie_d1, false),
        bad["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(t3_res["status"], "failed");
    assert!(
        docker_state
            .lock()
            .unwrap()
            .stopped
            .contains(&"cid-3".to_owned())
    );
    let (ps_still, pb_still) = http_get_host(proxy_http_addr, "app.example.com", "/").await;
    assert_eq!((ps_still, pb_still.as_str()), (200, "version-2"));

    let (_, attn) = req_json(
        &api_addr,
        Method::GET,
        "/api/v1/attention",
        (&cookie_d1, false),
        None,
    )
    .await;
    assert!(
        attn["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == "deploy:web"
                && i["text"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Deploy failed for web")),
        "{attn}"
    );

    // 4. Create `deploy:web` scoped token and use it to roll back `web` to `d1_id` WITHOUT rebuilding!
    let (st_tok, tok_created) = req_json(
        &api_addr,
        Method::POST,
        "/api/v1/tokens",
        (&cookie_a, false),
        Some(json!({ "name": "ci-web", "scope": "deploy:web" })),
    )
    .await;
    assert_eq!(st_tok, StatusCode::OK, "{tok_created}");
    let web_token = tok_created["secret"].as_str().unwrap().to_owned();

    // Verify `deploy:web` token cannot hit unauthorized endpoints.
    for (m, p, b) in [
        (Method::GET, "/api/v1/settings", None),
        (
            Method::GET,
            &format!("/api/v1/files?path={}", root.join("apps").display()),
            None,
        ),
        (
            Method::POST,
            "/api/v1/services",
            Some(json!({"name": "evil", "repo": "acme/evil"})),
        ),
        (
            Method::PUT,
            "/api/v1/services/web/vars/FOO",
            Some(json!({"value": "bar"})),
        ),
        (Method::DELETE, "/api/v1/services/web", None),
    ] {
        let (sc, _) = req_json(&api_addr, m, p, (&web_token, true), b).await;
        assert_eq!(sc, StatusCode::FORBIDDEN);
    }

    // Other deployer d2 cannot read/deploy d1's service `web`.
    assert_eq!(
        req_json(
            &api_addr,
            Method::POST,
            "/api/v1/services/web/deploys",
            (&cookie_d2, false),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let builds_before_rb = std::fs::read_to_string(&build_count_file)
        .unwrap()
        .lines()
        .count();
    let (st_rb, rb) = req_json(
        &api_addr,
        Method::POST,
        &format!("/api/v1/services/web/deploys/{d1_id}/rollback"),
        (&web_token, true),
        None,
    )
    .await;
    assert_eq!(st_rb, StatusCode::ACCEPTED, "{rb}");
    let t_rb_res =
        wait_task_done(&api_addr, (&web_token, true), rb["task"].as_str().unwrap()).await;
    assert_eq!(t_rb_res["status"], "ok");
    let builds_after_rb = std::fs::read_to_string(&build_count_file)
        .unwrap()
        .lines()
        .count();
    assert_eq!(
        builds_before_rb, builds_after_rb,
        "rollback must not rebuild image"
    );

    let (ps_rb, pb_rb) = http_get_host(proxy_http_addr, "app.example.com", "/").await;
    assert_eq!((ps_rb, pb_rb.as_str()), (200, "version-1"));
    let (_, attn_after) = req_json(
        &api_addr,
        Method::GET,
        "/api/v1/attention",
        (&cookie_a, false),
        None,
    )
    .await;
    assert!(
        !attn_after["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["text"]
                .as_str()
                .unwrap_or("")
                .contains("Deploy failed for web")),
        "attention should clear after live rollback: {attn_after}"
    );
    let _ = (sha1, d2_id);

    // 5. GitHub signed push webhook triggers auto-deploy for matching service!
    let wh_secret = "wh-secret-test-999";
    let wh_enc = state
        .priv_client
        .secret_seal(
            "whs",
            None,
            "deploy:github_app:webhook_secret",
            opsbox_proto::Secret::new(wh_secret),
        )
        .await
        .unwrap();
    let pk_enc = state
        .priv_client
        .secret_seal(
            "pk",
            None,
            "deploy:github_app:private_key",
            opsbox_proto::Secret::new("dummy"),
        )
        .await
        .unwrap();
    store
        .upsert_github_app(&opsbox_store::NewGithubApp {
            app_id: 1,
            slug: "opsbox-test".into(),
            name: "Opsbox".into(),
            owner_login: "acme".into(),
            owner_type: "Organization".into(),
            html_url: "https://github.com/apps/opsbox-test".into(),
            private_key_enc: pk_enc,
            webhook_secret_enc: wh_enc,
            webhook_url: None,
            webhook_active: true,
            installation_id: None,
            installation_account: None,
            installation_html_url: None,
            repository_selection: None,
            created_by: "alice".into(),
            now: now_epoch,
        })
        .unwrap();
    let _ = store
        .create_service(opsbox_store::NewService {
            project_id: &store.ensure_default_project(10).unwrap().0.id,
            env_id: &store.ensure_default_project(10).unwrap().1.id,
            name: "api",
            kind: opsbox_store::ServiceKind::App,
            owner_id: Some(&d2.id),
            source_kind: opsbox_store::SourceKind::GitUrl,
            repo: "https://github.com/acme/api.git",
            branch: "main",
            build_method: "auto",
            dockerfile_path: None,
            start_cmd: None,
            port: 4000,
            health_path: "/",
            domain: None,
            image_ref: None,
            deploy_key_id: None,
            auto_deploy: true,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 200,
            graph_y: 20,
            note: None,
            now: 10,
        })
        .unwrap();
    // `deploy:web` token cannot trigger or read `api`!
    assert_eq!(
        req_json(
            &api_addr,
            Method::POST,
            "/api/v1/services/api/deploys",
            (&web_token, true),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        req_json(
            &api_addr,
            Method::GET,
            "/api/v1/services/api",
            (&web_token, true),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    let push_payload = serde_json::to_vec(&json!({
        "ref": "refs/heads/main",
        "after": "abcdef1234567",
        "deleted": false,
        "repository": { "full_name": "acme/api" },
        "pusher": { "name": "alice" },
        "head_commit": { "message": "Push API update" }
    }))
    .unwrap();
    let sig = sign_webhook(wh_secret, &push_payload);
    let mut wh_conn = tokio::net::TcpStream::connect(&api_addr).await.unwrap();
    let wh_req = format!(
        "POST /api/v1/github/webhook HTTP/1.1\r\nHost: {api_addr}\r\nX-GitHub-Event: push\r\nX-Hub-Signature-256: {sig}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        push_payload.len()
    );
    wh_conn.write_all(wh_req.as_bytes()).await.unwrap();
    wh_conn.write_all(&push_payload).await.unwrap();
    let mut wh_raw = Vec::new();
    wh_conn.read_to_end(&mut wh_raw).await.unwrap();
    let wh_body: Value = serde_json::from_str(
        String::from_utf8_lossy(&wh_raw)
            .split_once("\r\n\r\n")
            .unwrap()
            .1,
    )
    .unwrap();
    assert_eq!(wh_body["queued"], 1, "{wh_body}");
    let api_task_id = wh_body["deploys"][0]["task"].as_str().unwrap();
    let _ = wait_task_done(&api_addr, (&cookie_d2, false), api_task_id).await;

    // 6. Delete service `web` -> removes from project and proxy returns 404 for app.example.com.
    let (st_del, _) = req_json(
        &api_addr,
        Method::DELETE,
        "/api/v1/services/web",
        (&cookie_d1, false),
        None,
    )
    .await;
    assert_eq!(st_del, StatusCode::OK);
    let (ps_gone, _) = http_get_host(proxy_http_addr, "app.example.com", "/").await;
    assert_eq!(ps_gone, 404);

    // Image-source create -> PATCH tag -> pull the new tag -> retained-image rollback.
    {
        let mut docker = docker_state.lock().unwrap();
        docker.ordered_ports = vec![p1, p2];
        docker.build_port_cursor = 0;
    }
    let (status, created) = req_json(
        &api_addr,
        Method::POST,
        "/api/v1/services",
        (&cookie_a, false),
        Some(
            json!({"name": "image-app", "image": "nginx:1.26", "port": 3000,
            "domain": "image.example.com", "memoryLimitMb": 128}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let task = wait_task_done(
        &api_addr,
        (&cookie_a, false),
        created["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(task["status"], "ok", "{task}");
    let first_deploy = created["deployId"].as_str().unwrap();
    let (status, updated) = req_json(
        &api_addr,
        Method::PATCH,
        "/api/v1/services/image-app",
        (&cookie_a, false),
        Some(json!({"image": "nginx:1.27"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!(updated["imageRef"], "nginx:1.27");
    let (status, deployed) = req_json(
        &api_addr,
        Method::POST,
        "/api/v1/services/image-app/deploys",
        (&cookie_a, false),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{deployed}");
    let task = wait_task_done(
        &api_addr,
        (&cookie_a, false),
        deployed["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(task["status"], "ok", "{task}");
    assert_eq!(
        http_get_host(proxy_http_addr, "image.example.com", "/").await,
        (200, "version-2".into())
    );
    let pulls_before_rollback = docker_state.lock().unwrap().pulled_images.clone();
    assert_eq!(pulls_before_rollback.len(), 2, "{pulls_before_rollback:?}");
    assert!(
        pulls_before_rollback[0].contains("1.26"),
        "{pulls_before_rollback:?}"
    );
    assert!(
        pulls_before_rollback[1].contains("1.27"),
        "{pulls_before_rollback:?}"
    );
    let (status, rolled_back) = req_json(
        &api_addr,
        Method::POST,
        &format!("/api/v1/services/image-app/deploys/{first_deploy}/rollback"),
        (&cookie_a, false),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{rolled_back}");
    let task = wait_task_done(
        &api_addr,
        (&cookie_a, false),
        rolled_back["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(task["status"], "ok", "{task}");
    assert_eq!(
        http_get_host(proxy_http_addr, "image.example.com", "/").await,
        (200, "version-1".into())
    );
    assert_eq!(
        docker_state.lock().unwrap().pulled_images,
        pulls_before_rollback,
        "rollback must use the retained release without pulling"
    );
    assert_eq!(
        store
            .get_service("image-app")
            .unwrap()
            .unwrap()
            .image_ref
            .as_deref(),
        Some("nginx:1.27")
    );

    api_task.abort();
    let _ = proxy_stop.send(());
    proxy_task.await.unwrap();
    let _ = priv_stop.send(());
    priv_task.await.unwrap();
    docker_task.abort();
    t1.abort();
    t2.abort();
    t3.abort();
}
