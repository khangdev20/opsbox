//! End-to-end tests of `PrivServer` + `PrivClient` Docker operations over a Unix socket
//! backed by a real Unix-socket mock Docker Engine API.

use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::{
    DockerContainerAction, DockerExecRequest, DockerLogFrame, DockerLogStream, PrivErrorCode,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;

fn own_uid(dir: &Path) -> u32 {
    let probe = dir.join("uid-probe");
    std::fs::write(&probe, b"").unwrap();
    std::fs::metadata(&probe).unwrap().uid()
}

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

async fn write_http_json(
    stream: &mut UnixStream,
    status: u16,
    reason: &str,
    body: &serde_json::Value,
) {
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
                            "SizeRw": 8192,
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
                                    "Labels": {"opsbox.service": "web"}
                                },
                                "Mounts": [],
                                "NetworkSettings": {"Ports": {}, "Networks": {}}
                            }),
                        )
                        .await;
                    }
                    ("POST", p)
                        if p.starts_with("/containers/web-1/")
                            && matches!(
                                p.strip_prefix("/containers/web-1/"),
                                Some("start" | "stop" | "restart")
                            ) =>
                    {
                        actions.lock().unwrap().push(format!("POST {p}"));
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", "/containers/web-1") => {
                        actions
                            .lock()
                            .unwrap()
                            .push("DELETE /containers/web-1".into());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", "/containers/web-1/logs") => {
                        let follow = query.contains("follow=true");
                        let mut payload = docker_mux_frame(1, b"2026-11-18T02:00:00Z started\n");
                        payload.extend_from_slice(&docker_mux_frame(
                            2,
                            b"2026-11-18T02:00:01Z warning line\n",
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
                                "Size": 100_000,
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
                                    "UsageData": { "Size": 1234, "RefCount": 1 }
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
                    ("POST", "/containers/web-1/exec") => {
                        write_http_json(&mut stream, 201, "Created", &json!({ "Id": "ex1" })).await;
                    }
                    ("POST", "/exec/ex1/start") => {
                        let out = docker_mux_frame(1, b"ok\n");
                        let resp = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\nContent-Type: application/vnd.docker.multiplexed-stream\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.write_all(&out).await;
                        let _ = stream.flush().await;
                    }
                    ("GET", "/exec/ex1/json") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({ "Running": false, "ExitCode": 0 }),
                        )
                        .await;
                    }
                    ("GET", "/system/df") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "LayersSize": 0,
                                "Images": [],
                                "Containers": [],
                                "Volumes": [],
                                "BuildCache": [{ "InUse": false, "Size": 4096 }]
                            }),
                        )
                        .await;
                    }
                    ("POST", "/images/prune") => {
                        actions.lock().unwrap().push("POST /images/prune".into());
                        write_http_json(&mut stream, 200, "OK", &json!({ "SpaceReclaimed": 4096 }))
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

#[tokio::test]
async fn priv_docker_round_trip_and_validation() {
    let actions = Arc::new(Mutex::new(Vec::new()));
    let (_docker_dir, docker_sock, daemon_task) = spawn_mock_daemon(actions.clone()).await;

    let srv_dir = tempfile::tempdir().unwrap();
    let uid = own_uid(srv_dir.path());
    let sock = srv_dir.path().join("priv.sock");
    let mut cfg = PrivConfig::new(&sock, vec![uid]);
    cfg.master_key_path = srv_dir.path().join("master.key");
    cfg.docker_socket = docker_sock;

    let server = PrivServer::bind(cfg).unwrap();
    let (stop, rx) = oneshot::channel::<()>();
    let srv_task = tokio::spawn(server.run(async {
        let _ = rx.await;
    }));
    let client = PrivClient::new(&sock).with_timeout(Duration::from_secs(5));

    // Validation failures happen inside priv before any Docker socket call.
    let bad_inspect = client
        .docker_inspect("r1", "../etc")
        .await
        .expect_err("path traversal ref must fail");
    assert!(matches!(
        bad_inspect,
        opsbox_priv::ClientError::Remote(ref e) if e.code == PrivErrorCode::Invalid
    ));

    let bad_exec = client
        .docker_exec(
            "r2",
            "admin",
            DockerExecRequest {
                id: "web-1".into(),
                cmd: vec![],
                user: None,
                working_dir: None,
                env: vec![],
                stdin: None,
            },
        )
        .await
        .expect_err("empty cmd must fail");
    assert!(matches!(
        bad_exec,
        opsbox_priv::ClientError::Remote(ref e) if e.code == PrivErrorCode::Invalid
    ));

    let bad_stream = client
        .docker_logs_follow("r3", Some("viewer"), "bad;id", 10)
        .await
        .expect_err("bad stream id must fail");
    assert!(matches!(
        bad_stream,
        opsbox_priv::ClientError::Remote(ref e) if e.code == PrivErrorCode::Invalid
    ));

    // Happy-path round-trips through PrivServer -> opsbox-docker -> mock daemon.
    let status = client.docker_status("r4").await.unwrap();
    assert!(status.installed && status.running);
    assert_eq!(status.version.as_deref(), Some("27.4.0"));

    let containers = client.docker_list_containers("r5").await.unwrap();
    assert_eq!(containers.len(), 2);
    let web = containers.iter().find(|c| c.name == "web-1").unwrap();
    let flaky = containers
        .iter()
        .find(|c| c.name == "flaky-worker")
        .unwrap();
    assert!(web.managed);
    assert_eq!(web.service.as_deref(), Some("web"));
    assert!(!flaky.managed);

    let inspect = client.docker_inspect("r6", "web-1").await.unwrap();
    assert_eq!(inspect.name, "web-1");
    let secret = inspect.env.iter().find(|e| e.key == "DB_PASSWORD").unwrap();
    assert!(secret.masked);
    assert_ne!(secret.value, "supersecret");

    client
        .docker_container_action("r7", "admin", "web-1", DockerContainerAction::Restart)
        .await
        .unwrap();
    assert!(
        actions
            .lock()
            .unwrap()
            .iter()
            .any(|a| a == "POST /containers/web-1/restart")
    );

    let logs = client.docker_logs("r8", "web-1", 50).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].stream, DockerLogStream::Stdout);
    assert_eq!(logs[1].stream, DockerLogStream::Stderr);

    let mut session = client
        .docker_logs_follow("r9", Some("viewer"), "web-1", 10)
        .await
        .unwrap();
    let mut streamed = Vec::new();
    while let Some(frame) = session.recv().await.unwrap() {
        match frame {
            DockerLogFrame::Line(l) => streamed.push(l),
            DockerLogFrame::End => break,
            DockerLogFrame::Error { message } => panic!("unexpected stream error: {message}"),
        }
    }
    assert_eq!(streamed.len(), 2);
    drop(session);

    let exec_out = client
        .docker_exec(
            "r10",
            "admin",
            DockerExecRequest {
                id: "web-1".into(),
                cmd: vec!["echo".into(), "ok".into()],
                user: None,
                working_dir: None,
                env: vec![],
                stdin: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(exec_out.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&exec_out.stdout).trim(), "ok");

    assert_eq!(client.docker_list_images("r11").await.unwrap().len(), 1);
    assert_eq!(client.docker_list_volumes("r12").await.unwrap().len(), 1);
    assert_eq!(client.docker_list_networks("r13").await.unwrap().len(), 1);

    let facts = client.system_facts("r14").await.unwrap();
    assert_eq!(facts.restarting_containers, vec!["flaky-worker".to_owned()]);

    let _ = stop.send(());
    let _ = srv_task.await;
    daemon_task.abort();
}

#[tokio::test]
async fn priv_docker_unavailable_reports_unsupported_cleanly() {
    let srv_dir = tempfile::tempdir().unwrap();
    let uid = own_uid(srv_dir.path());
    let sock = srv_dir.path().join("priv.sock");
    let mut cfg = PrivConfig::new(&sock, vec![uid]);
    cfg.master_key_path = srv_dir.path().join("master.key");
    cfg.docker_socket = srv_dir.path().join("nonexistent-docker.sock");

    let server = PrivServer::bind(cfg).unwrap();
    let (stop, rx) = oneshot::channel::<()>();
    let srv_task = tokio::spawn(server.run(async {
        let _ = rx.await;
    }));
    let client = PrivClient::new(&sock).with_timeout(Duration::from_secs(5));

    let st = client.docker_status("s1").await.unwrap();
    assert!(!st.running);

    let err = client
        .docker_list_containers("s2")
        .await
        .expect_err("missing docker should return an error");
    assert!(matches!(err, opsbox_priv::ClientError::Remote(_)));

    let _ = stop.send(());
    let _ = srv_task.await;
}
