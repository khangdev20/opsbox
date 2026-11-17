//! End-to-end HTTP, Metrics/Attention, and WebSocket tests for `/api/v1/docker/*`
//! and `/api/v1/stream/containers/{id}/logs`.

mod common;

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{Browser, Opts};
use futures_util::StreamExt;
use opsbox_api::{AppState, FilesSettings, SecureCookies, router};
use opsbox_auth::AuthConfig;
use opsbox_auth::password::HashParams;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::Role;
use opsbox_store::{SessionKind, Store};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

fn docker_mux_frame(stream: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.push(stream);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&(u32::try_from(payload.len()).unwrap()).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

async fn read_http_req(stream: &mut UnixStream) -> Option<(String, String)> {
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
    let req_line = head.lines().next()?;
    let mut parts = req_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let content_len: usize = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    let mut body_len = buf.len() - (split + 4);
    while body_len < content_len {
        let n = stream.read(&mut tmp).await.ok()?;
        if n == 0 {
            break;
        }
        body_len += n;
    }
    Some((method, target))
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

async fn spawn_mock_daemon(
    actions: Arc<Mutex<Vec<String>>>,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    tokio::task::JoinHandle<()>,
) {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("docker.sock");
    let listener = UnixListener::bind(&sock).unwrap();

    let handle = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let actions = actions.clone();
            tokio::spawn(async move {
                let Some((method, target)) = read_http_req(&mut stream).await else {
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
                            &json!({ "Version": "27.4.0", "ApiVersion": "1.47" }),
                        )
                        .await;
                    }
                    ("GET", "/containers/json") => {
                        let restarting_only = query.contains("restarting");
                        let mut list = vec![json!({
                            "Id": "eeee1111ffff22220000333311114444",
                            "Names": ["/flaky-worker"],
                            "Image": "alpine:3.20",
                            "Created": 1_700_000_000,
                            "State": "restarting",
                            "Status": "Restarting (1) 4 seconds ago",
                            "Ports": [],
                            "Labels": {}
                        })];
                        if !restarting_only {
                            list.insert(
                                0,
                                json!({
                                    "Id": "aaaa1111bbbb2222cccc3333dddd4444",
                                    "Names": ["/web-1"],
                                    "Image": "nginx:1.27",
                                    "Created": 1_700_000_000,
                                    "State": "running",
                                    "Status": "Up 1 hour",
                                    "Ports": [{"IP": "0.0.0.0", "PrivatePort": 80, "PublicPort": 8080, "Type": "tcp"}],
                                    "NetworkSettings": {"Networks": {"bridge": {"IPAddress": "172.17.0.2"}}},
                                    "Labels": {"opsbox.service": "web"}
                                }),
                            );
                        }
                        write_http_json(&mut stream, 200, "OK", &json!(list)).await;
                    }
                    ("GET", "/containers/aaaa1111bbbb2222cccc3333dddd4444/stats") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "id": "aaaa1111bbbb2222cccc3333dddd4444",
                                "name": "/web-1",
                                "cpu_stats": {
                                    "cpu_usage": { "total_usage": 300_000_000u64 },
                                    "system_cpu_usage": 2_000_000_000u64,
                                    "online_cpus": 2
                                },
                                "precpu_stats": {
                                    "cpu_usage": { "total_usage": 100_000_000u64 },
                                    "system_cpu_usage": 1_000_000_000u64,
                                    "online_cpus": 2
                                },
                                "memory_stats": {
                                    "usage": 50 * 1024 * 1024,
                                    "limit": 1024 * 1024 * 1024,
                                    "stats": { "inactive_file": 10 * 1024 * 1024 }
                                },
                                "networks": { "eth0": { "rx_bytes": 1024, "tx_bytes": 2048 } },
                                "pids_stats": { "current": 4 }
                            }),
                        )
                        .await;
                    }
                    ("GET", "/containers/web-1/json") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": "aaaa1111bbbb2222cccc3333dddd4444",
                                "Name": "/web-1",
                                "Created": "2026-11-18T01:00:00Z",
                                "RestartCount": 1,
                                "State": {
                                    "Status": "running",
                                    "Running": true,
                                    "Paused": false,
                                    "Restarting": false,
                                    "OOMKilled": false,
                                    "Pid": 123,
                                    "ExitCode": 0,
                                    "StartedAt": "2026-11-18T01:00:01Z",
                                    "FinishedAt": ""
                                },
                                "Config": {
                                    "Image": "nginx:1.27",
                                    "Cmd": ["nginx"],
                                    "Env": ["PORT=80", "DB_PASSWORD=supersecret"],
                                    "Labels": { "opsbox.service": "web" }
                                },
                                "Mounts": [],
                                "NetworkSettings": { "Ports": {}, "Networks": {} }
                            }),
                        )
                        .await;
                    }
                    ("POST", p)
                        if p.starts_with("/containers/")
                            && (p.ends_with("/start")
                                || p.ends_with("/stop")
                                || p.ends_with("/restart")) =>
                    {
                        actions.lock().unwrap().push(format!("POST {p}"));
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", "/containers/flaky-worker") => {
                        actions
                            .lock()
                            .unwrap()
                            .push("DELETE /containers/flaky-worker".into());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", "/containers/web-1/logs") => {
                        let follow = query.contains("follow=true");
                        let mut payload = docker_mux_frame(1, b"2026-11-18T02:00:00Z started\n");
                        payload.extend_from_slice(&docker_mux_frame(
                            2,
                            b"2026-11-18T02:00:01Z stderr warning\n",
                        ));
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.docker.raw-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            payload.len()
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.write_all(&payload).await;
                        if follow {
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        let _ = stream.flush().await;
                    }
                    ("GET", "/images/json") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!([{
                                "Id": "sha256:1234",
                                "ParentId": "",
                                "RepoTags": ["nginx:1.27"],
                                "RepoDigests": [],
                                "Size": 1_073_741_824u64,
                                "SharedSize": 0,
                                "Created": 1_700_000_000,
                                "Labels": {},
                                "Containers": 1
                            }]),
                        )
                        .await;
                    }
                    ("GET", "/volumes") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Volumes": [{
                                    "Name": "web_data",
                                    "Driver": "local",
                                    "Mountpoint": "/var/lib/docker/volumes/web_data/_data",
                                    "Scope": "local",
                                    "Labels": {},
                                    "Options": {},
                                    "UsageData": { "Size": 536_870_912, "RefCount": 1 }
                                }]
                            }),
                        )
                        .await;
                    }
                    ("GET", "/networks") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!([{
                                "Id": "net123",
                                "Name": "bridge",
                                "Driver": "bridge",
                                "Scope": "local",
                                "Internal": false,
                                "Containers": {}
                            }]),
                        )
                        .await;
                    }
                    ("GET", "/system/df") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "BuildCache": [],
                                "Volumes": [{ "Name": "web_data", "UsageData": { "Size": 536_870_912, "RefCount": 1 } }]
                            }),
                        )
                        .await;
                    }
                    _ => {
                        write_http_json(
                            &mut stream,
                            404,
                            "Not Found",
                            &json!({ "message": "not found" }),
                        )
                        .await;
                    }
                }
            });
        }
    });

    (dir, sock, handle)
}

struct DockerHarness {
    app: axum::Router,
    state: AppState,
    store: Arc<Store>,
    _dir: tempfile::TempDir,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl DockerHarness {
    fn with_docker_socket(docker_sock: &Path) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("apps")).unwrap();
        let socket = dir.path().join("priv.sock");
        let probe = dir.path().join("probe");
        std::fs::write(&probe, b"").unwrap();
        let uid = std::fs::metadata(&probe).unwrap().uid();

        let mut cfg = PrivConfig::new(&socket, vec![uid]);
        cfg.master_key_path = dir.path().join("master.key");
        cfg.xfer_dir = root.join("xfer");
        cfg.trash_dir = root.join("trash");
        cfg.docker_socket = docker_sock.to_path_buf();
        let server = PrivServer::bind(cfg).unwrap();
        let (stop, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(server.run(async {
            let _ = rx.await;
        }));

        let store = Arc::new(Store::open_in_memory().unwrap());
        let u = store
            .create_user("__seed_admin", Role::Admin, "unused-hash", 0)
            .unwrap();
        store.set_pending_totp(&u.id, "unused", 0).unwrap();
        store.enable_totp(&u.id, 0, 0).unwrap();

        let state = AppState::new(
            store.clone(),
            PrivClient::new(&socket).with_timeout(Duration::from_secs(5)),
            "9.9.9-test",
            SecureCookies::Auto,
            AuthConfig {
                hash_params: HashParams {
                    memory_kib: 8,
                    iterations: 1,
                    parallelism: 1,
                },
                ..AuthConfig::default()
            },
        )
        .with_files(FilesSettings {
            apps_dir: root.join("apps"),
            xfer_dir: root.join("xfer"),
            max_upload: 2 << 20,
        });

        Self {
            app: router(state.clone()),
            state,
            store,
            _dir: dir,
            stop,
            task,
        }
    }

    fn browser(&self) -> Browser {
        Browser {
            app: self.app.clone(),
            jar: BTreeMap::new(),
            set_cookies: Vec::new(),
        }
    }

    fn signed_in(&self, name: &str, role: Role) -> Browser {
        let user = self
            .store
            .create_user(name, role, "unused-hash", 0)
            .unwrap();
        let token = opsbox_auth::tokens::new_token().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        self.store
            .create_session(
                &user.id,
                &opsbox_auth::tokens::hash_token(&token),
                SessionKind::Web,
                None,
                None,
                now,
            )
            .unwrap();
        let mut b = self.browser();
        b.jar.insert("opsbox_session".into(), token.clone());
        b.jar.insert(
            "opsbox_csrf".into(),
            opsbox_auth::tokens::csrf_token(&token),
        );
        b
    }

    async fn shutdown(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }
}

fn cookie_of(b: &Browser) -> String {
    b.jar
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[tokio::test]
async fn docker_routes_enforce_auth_roles_validation_and_end_to_end_flows() {
    let actions = Arc::new(Mutex::new(Vec::new()));
    let (_docker_dir, docker_sock, daemon_task) = spawn_mock_daemon(actions.clone()).await;
    let h = DockerHarness::with_docker_socket(&docker_sock);

    // 1. Unauthenticated requests rejected.
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/docker/status").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        anon.get("/api/v1/docker/containers").await.status,
        StatusCode::UNAUTHORIZED
    );

    // 2. Role & CSRF checks.
    let mut viewer = h.signed_in("viewer1", Role::Viewer);
    let mut deployer = h.signed_in("deployer1", Role::Deployer);
    let mut admin = h.signed_in("admin1", Role::Admin);

    assert_eq!(
        viewer
            .post("/api/v1/docker/install", json!({}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        viewer
            .post("/api/v1/docker/containers/web-1/restart", json!({}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        deployer
            .post("/api/v1/docker/containers/web-1/restart", json!({}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        admin
            .send(
                Method::POST,
                "/api/v1/docker/containers/web-1/restart",
                Some(json!({})),
                Opts::bare(),
            )
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    // 3. Token scopes: Viewer token cannot read docker, Deployer token can read docker (not mutate).
    let read_tok_secret = admin
        .post(
            "/api/v1/tokens",
            json!({ "name": "reader-tok", "scope": "read" }),
        )
        .await
        .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    let deploy_tok_secret = admin
        .post(
            "/api/v1/tokens",
            json!({ "name": "deploy-tok", "scope": "deploy" }),
        )
        .await
        .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();

    let mut read_tok_opts = Opts::bare();
    read_tok_opts
        .extra
        .push(("authorization".into(), format!("Bearer {read_tok_secret}")));
    assert_eq!(
        anon.send(
            Method::GET,
            "/api/v1/docker/containers",
            None,
            read_tok_opts,
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    let mut deploy_tok_opts = Opts::bare();
    deploy_tok_opts.extra.push((
        "authorization".into(),
        format!("Bearer {deploy_tok_secret}"),
    ));
    assert_eq!(
        anon.send(
            Method::GET,
            "/api/v1/docker/containers",
            None,
            deploy_tok_opts.clone(),
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        anon.send(
            Method::POST,
            "/api/v1/docker/containers/web-1/restart",
            Some(json!({})),
            deploy_tok_opts,
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    // 4. Input validation rejections.
    let bad_inspect = viewer.get("/api/v1/docker/containers/-bad").await;
    assert_eq!(bad_inspect.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad_inspect.error_code(), "validation");

    let bad_action = admin
        .post("/api/v1/docker/containers/web-1/nuke", json!({}))
        .await;
    assert_eq!(bad_action.status, StatusCode::BAD_REQUEST);

    // 5. Happy paths over HTTP -> priv -> opsbox-docker -> mock daemon.
    let status = viewer.get("/api/v1/docker/status").await.json();
    assert_eq!(status["installed"], json!(true));
    assert_eq!(status["running"], json!(true));
    assert_eq!(status["version"], json!("27.4.0"));

    let list = viewer.get("/api/v1/docker/containers").await.json();
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    let web = arr.iter().find(|c| c["name"] == "web-1").unwrap();
    let flaky = arr.iter().find(|c| c["name"] == "flaky-worker").unwrap();
    assert_eq!(web["managed"], json!(true));
    assert_eq!(web["service"], json!("web"));
    assert_eq!(web["cpu"], json!(40.0));
    assert_eq!(web["mem"], json!(40.0));
    assert_eq!(flaky["managed"], json!(false));

    let inspect = viewer.get("/api/v1/docker/containers/web-1").await.json();
    let env = inspect["env"].as_array().unwrap();
    let secret = env.iter().find(|e| e["key"] == "DB_PASSWORD").unwrap();
    assert_eq!(secret["masked"], json!(true));
    assert_ne!(secret["value"], json!("supersecret"));

    // Control both managed and unmanaged containers, with audit rows written.
    assert_eq!(
        admin
            .post("/api/v1/docker/containers/web-1/restart", json!({}))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        admin
            .post("/api/v1/docker/containers/flaky-worker/delete", json!({}))
            .await
            .status,
        StatusCode::OK
    );
    assert!(
        actions
            .lock()
            .unwrap()
            .contains(&"DELETE /containers/flaky-worker".to_owned())
    );
    let audit = h.store.recent_audit(10).unwrap();
    assert!(audit.iter().any(|a| a.action == "docker.container.restart"));
    assert!(audit.iter().any(|a| a.action == "docker.container.remove"));

    let logs = viewer
        .get("/api/v1/docker/containers/web-1/logs?tail=50")
        .await
        .json();
    assert_eq!(logs.as_array().unwrap().len(), 2);
    assert_eq!(logs[1]["stream"], json!("stderr"));

    let images = viewer.get("/api/v1/docker/images").await.json();
    assert_eq!(images[0]["name"], json!("nginx"));
    assert_eq!(images[0]["size"], json!(1.0));
    let volumes = viewer.get("/api/v1/docker/volumes").await.json();
    assert_eq!(volumes[0]["name"], json!("web_data"));
    assert_eq!(volumes[0]["size"], json!(0.5));
    let networks = viewer.get("/api/v1/docker/networks").await.json();
    assert_eq!(networks[0]["name"], json!("bridge"));

    // 6. Overview metrics + attention pick up running containers & restarting containers.
    let mut got_container_stats = false;
    let mut got_restarting_attention = false;
    for _ in 0..40 {
        let m = viewer.get("/api/v1/metrics").await;
        if m.status == StatusCode::OK {
            let mv = m.json();
            if mv["containers"]
                .as_array()
                .is_some_and(|c| c.iter().any(|x| x["name"] == "web-1"))
            {
                got_container_stats = true;
            }
        }
        let att = viewer.get("/api/v1/attention").await.json();
        if att
            .as_array()
            .is_some_and(|items| items.iter().any(|a| a["kind"] == "containers"))
        {
            got_restarting_attention = true;
        }
        if got_container_stats && got_restarting_attention {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        got_container_stats,
        "expected web-1 stats in /api/v1/metrics"
    );
    assert!(
        got_restarting_attention,
        "expected restarting container alert in /api/v1/attention"
    );

    // 7. WebSocket live log streaming + cross-origin refusal.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(opsbox_api::serve(
        listener,
        h.state.clone(),
        std::future::pending(),
    ));

    let mut bad_ws_req = format!("ws://{addr}/api/v1/stream/containers/web-1/logs")
        .into_client_request()
        .unwrap();
    bad_ws_req
        .headers_mut()
        .insert("cookie", cookie_of(&viewer).parse().unwrap());
    bad_ws_req
        .headers_mut()
        .insert("origin", "https://evil.example".parse().unwrap());
    let err = tokio_tungstenite::connect_async(bad_ws_req)
        .await
        .expect_err("cross-origin WS must fail");
    assert!(matches!(err, WsError::Http(resp) if resp.status() == StatusCode::FORBIDDEN));

    let mut ok_ws_req = format!("ws://{addr}/api/v1/stream/containers/web-1/logs?tail=10")
        .into_client_request()
        .unwrap();
    ok_ws_req
        .headers_mut()
        .insert("cookie", cookie_of(&viewer).parse().unwrap());
    ok_ws_req
        .headers_mut()
        .insert("origin", format!("http://{addr}").parse().unwrap());
    let (mut ws, resp) = tokio_tungstenite::connect_async(ok_ws_req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SWITCHING_PROTOCOLS);

    let mut got_lines = 0usize;
    let mut got_end = false;
    while let Some(msg) = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .unwrap()
    {
        if let Ok(Message::Text(txt)) = msg {
            let v: Value = serde_json::from_str(&txt).unwrap();
            if v["type"] == "logs" {
                got_lines += v["entries"].as_array().map_or(0, Vec::len);
            } else if v["type"] == "end" {
                got_end = true;
                break;
            }
        }
    }
    assert_eq!(got_lines, 2);
    assert!(got_end);

    h.shutdown().await;
    daemon_task.abort();
}

#[tokio::test]
async fn missing_docker_vs_stopped_daemon_map_to_distinct_status_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let missing_sock = tmp.path().join("missing.sock");
    let h_missing = DockerHarness::with_docker_socket(&missing_sock);
    let mut viewer = h_missing.signed_in("v1", Role::Viewer);

    let st = viewer.get("/api/v1/docker/status").await;
    assert_eq!(st.status, StatusCode::OK);
    assert_eq!(st.json()["installed"], json!(false));
    assert_eq!(st.json()["running"], json!(false));

    let list_missing = viewer.get("/api/v1/docker/containers").await;
    assert_eq!(list_missing.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(list_missing.error_code(), "unsupported");
    h_missing.shutdown().await;

    // A socket path that exists on disk but has no daemon listening behaves like an installed, stopped daemon.
    let dead_sock = tmp.path().join("dead-docker.sock");
    let dead_listener = UnixListener::bind(&dead_sock).unwrap();
    drop(dead_listener);
    let h_stopped = DockerHarness::with_docker_socket(&dead_sock);
    let mut viewer2 = h_stopped.signed_in("v2", Role::Viewer);

    let st2 = viewer2.get("/api/v1/docker/status").await;
    assert_eq!(st2.status, StatusCode::OK);
    assert_eq!(st2.json()["installed"], json!(true));
    assert_eq!(st2.json()["running"], json!(false));

    let list_stopped = viewer2.get("/api/v1/docker/containers").await;
    assert_eq!(list_stopped.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(list_stopped.error_code(), "unavailable");
    h_stopped.shutdown().await;
}
