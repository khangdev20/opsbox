//! End-to-end tests of `DockerClient` against a real Unix-socket mock Docker Engine API.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use opsbox_docker::{DockerClient, DockerError, DockerPruneKind};
use opsbox_proto::{DockerContainerAction, DockerExecRequest, DockerLogStream};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

fn docker_mux_frame(stream: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.push(stream);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&(u32::try_from(payload.len()).unwrap()).to_be_bytes());
    out.extend_from_slice(payload);
    out
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
    let req_line = lines.next()?.to_owned();
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

#[derive(Default)]
struct DaemonState {
    actions: Vec<String>,
    container_state: HashMap<String, String>,
    created_container_bodies: Vec<serde_json::Value>,
    networks_created: Vec<String>,
    images: Vec<String>,
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
    state: Arc<Mutex<DaemonState>>,
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
            let state = state.clone();
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
                    ("GET", "/containers/json") => {
                        let web_state = {
                            let st = state.lock().unwrap();
                            st.container_state
                                .get("web-1")
                                .cloned()
                                .unwrap_or_else(|| "running".into())
                        };
                        let all = query.contains("all=true");
                        let mut list = vec![json!({
                            "Id": "11112222333344445555666677778888",
                            "Names": ["/web-1"],
                            "Image": "nginx:1.27",
                            "Created": 1700000000,
                            "State": web_state,
                            "Status": if web_state == "running" { "Up 2 hours" } else { "Exited (0) 1 minute ago" },
                            "Ports": [{"IP": "0.0.0.0", "PrivatePort": 80, "PublicPort": 8080, "Type": "tcp"}],
                            "Mounts": [{
                                "Type": "volume",
                                "Name": "web_data",
                                "Source": "/var/lib/docker/volumes/web_data/_data",
                                "Destination": "/data"
                            }],
                            "NetworkSettings": {"Networks": {"bridge": {"IPAddress": "172.17.0.2"}}},
                            "Labels": {"opsbox.service": "web", "opsbox.deploy": "d1"}
                        })];
                        if all {
                            list.push(json!({
                                "Id": "aaaa2222333344445555666677778888",
                                "Names": ["/old-worker"],
                                "Image": "alpine:3.20",
                                "Created": 1699990000,
                                "State": "exited",
                                "Status": "Exited (0) 1 day ago",
                                "SizeRw": 4096,
                                "Ports": [],
                                "Labels": {}
                            }));
                        }
                        write_http_json(&mut stream, 200, "OK", &json!(list)).await;
                    }
                    ("GET", "/containers/web-1/json") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": "11112222333344445555666677778888",
                                "Name": "/web-1",
                                "Created": "2026-11-18T01:00:00Z",
                                "RestartCount": 2,
                                "State": {
                                    "Status": "running",
                                    "Running": true,
                                    "Paused": false,
                                    "Restarting": false,
                                    "OOMKilled": false,
                                    "Pid": 1234,
                                    "ExitCode": 0,
                                    "StartedAt": "2026-11-18T01:00:01Z",
                                    "FinishedAt": ""
                                },
                                "Config": {
                                    "Image": "nginx:1.27",
                                    "Cmd": ["nginx", "-g", "daemon off;"],
                                    "Env": ["PORT=80", "API_SECRET=topsecret"],
                                    "Labels": {"opsbox.service": "web"}
                                },
                                "Mounts": [{
                                    "Type": "volume",
                                    "Name": "web_data",
                                    "Source": "/var/lib/docker/volumes/web_data/_data",
                                    "Destination": "/data",
                                    "Mode": "rw",
                                    "RW": true
                                }],
                                "NetworkSettings": {
                                    "Ports": {
                                        "80/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8080"}]
                                    },
                                    "Networks": {
                                        "bridge": {"IPAddress": "172.17.0.2", "Gateway": "172.17.0.1", "MacAddress": "02:42:ac:11:00:02"}
                                    }
                                }
                            }),
                        )
                        .await;
                    }
                    ("GET", "/containers/missing/json") => {
                        write_http_json(
                            &mut stream,
                            404,
                            "Not Found",
                            &json!({"message": "No such container: missing"}),
                        )
                        .await;
                    }
                    ("POST", "/containers/web-1/stop") => {
                        {
                            let mut st = state.lock().unwrap();
                            st.actions.push("stop:web-1".into());
                            st.container_state.insert("web-1".into(), "exited".into());
                        }
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", "/containers/web-1/start") => {
                        {
                            let mut st = state.lock().unwrap();
                            st.actions.push("start:web-1".into());
                            st.container_state.insert("web-1".into(), "running".into());
                        }
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", "/containers/web-1/restart") => {
                        state.lock().unwrap().actions.push("restart:web-1".into());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    // Start/Restart enforce a memory limit via Docker update before acting.
                    ("POST", p) if p.ends_with("/update") => {
                        state.lock().unwrap().actions.push(format!("update:{p}"));
                        write_http_empty(&mut stream, 200, "OK").await;
                    }
                    ("DELETE", "/containers/web-1") => {
                        write_http_json(
                            &mut stream,
                            409,
                            "Conflict",
                            &json!({"message": "cannot remove a running container"}),
                        )
                        .await;
                    }
                    ("DELETE", "/containers/old-worker") => {
                        state
                            .lock()
                            .unwrap()
                            .actions
                            .push("remove:old-worker".into());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", "/containers/web-1/logs") => {
                        let mut payload =
                            docker_mux_frame(1, b"2026-11-18T05:00:00.000Z server started\n");
                        payload.extend_from_slice(&docker_mux_frame(
                            2,
                            b"2026-11-18T05:00:01.000Z warning: slow boot\n",
                        ));
                        let follow = query.contains("follow=true");
                        let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.docker.multiplexed-stream\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.write_all(&payload).await;
                        let _ = stream.flush().await;
                        if follow {
                            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                            let live = docker_mux_frame(1, b"2026-11-18T05:00:02.000Z live line\n");
                            let _ = stream.write_all(&live).await;
                            let _ = stream.flush().await;
                        }
                    }
                    ("GET", "/containers/11112222333344445555666677778888/stats") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "id": "11112222333344445555666677778888",
                                "name": "/web-1",
                                "cpu_stats": {
                                    "cpu_usage": {"total_usage": 300_000_000u64},
                                    "system_cpu_usage": 2_000_000_000u64,
                                    "online_cpus": 2
                                },
                                "precpu_stats": {
                                    "cpu_usage": {"total_usage": 100_000_000u64},
                                    "system_cpu_usage": 1_000_000_000u64,
                                    "online_cpus": 2
                                },
                                "memory_stats": {
                                    "usage": 50 * 1024 * 1024,
                                    "limit": 1024 * 1024 * 1024,
                                    "stats": {"inactive_file": 10 * 1024 * 1024}
                                },
                                "networks": {"eth0": {"rx_bytes": 1024, "tx_bytes": 2048}},
                                "pids_stats": {"current": 4}
                            }),
                        )
                        .await;
                    }
                    ("POST", "/containers/web-1/exec") => {
                        write_http_json(&mut stream, 201, "Created", &json!({"Id": "exec-1"}))
                            .await;
                    }
                    ("POST", "/exec/exec-1/start") => {
                        let resp = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\nContent-Type: application/vnd.docker.raw-stream\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.write_all(&docker_mux_frame(1, b"ok\n")).await;
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
                    ("GET", "/images/json") => {
                        let dangling = query.contains("dangling");
                        let imgs = if dangling {
                            json!([{
                                "Id": "sha256:dddd22223333",
                                "ParentId": "",
                                "RepoTags": ["<none>:<none>"],
                                "RepoDigests": [],
                                "Created": 1700000000,
                                "Size": 10_000_000,
                                "SharedSize": 0,
                                "Labels": {},
                                "Containers": 0
                            }])
                        } else {
                            json!([
                                {
                                    "Id": "sha256:bbbb22223333",
                                    "ParentId": "",
                                    "RepoTags": ["nginx:1.27"],
                                    "RepoDigests": [],
                                    "Created": 1700000000,
                                    "Size": 190_000_000,
                                    "SharedSize": 0,
                                    "Labels": {},
                                    "Containers": 1
                                },
                                {
                                    "Id": "sha256:dddd22223333",
                                    "ParentId": "",
                                    "RepoTags": ["<none>:<none>"],
                                    "RepoDigests": [],
                                    "Created": 1700000000,
                                    "Size": 10_000_000,
                                    "SharedSize": 0,
                                    "Labels": {},
                                    "Containers": 0
                                }
                            ])
                        };
                        write_http_json(&mut stream, 200, "OK", &imgs).await;
                    }
                    ("GET", "/volumes") => {
                        let vols = if query.contains("dangling") {
                            json!({ "Volumes": [{ "Name": "unused_vol", "Driver": "local", "Mountpoint": "/var/lib/docker/volumes/unused_vol/_data", "Scope": "local", "Labels": {}, "Options": {} }] })
                        } else {
                            json!({ "Volumes": [
                                { "Name": "web_data", "Driver": "local", "Mountpoint": "/var/lib/docker/volumes/web_data/_data", "Scope": "local", "Labels": {}, "Options": {} },
                                { "Name": "unused_vol", "Driver": "local", "Mountpoint": "/var/lib/docker/volumes/unused_vol/_data", "Scope": "local", "Labels": {}, "Options": {} }
                            ] })
                        };
                        write_http_json(&mut stream, 200, "OK", &vols).await;
                    }
                    ("GET", "/volumes/web_data") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({ "Name": "web_data", "Driver": "local", "Mountpoint": "/var/lib/docker/volumes/web_data/_data", "Scope": "local", "Labels": {}, "Options": {} }),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/pause") => {
                        state.lock().unwrap().actions.push(format!("pause:{p}"));
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/unpause") => {
                        state.lock().unwrap().actions.push(format!("unpause:{p}"));
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", "/networks") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!([{ "Id": "net111122223333", "Name": "bridge", "Driver": "bridge", "Scope": "local", "Internal": false }]),
                        )
                        .await;
                    }
                    ("GET", "/system/df") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "BuildCache": [{"InUse": false, "Size": 12345}],
                                "Volumes": [{"Name": "unused_vol", "UsageData": {"RefCount": 0, "Size": 5000}}]
                            }),
                        )
                        .await;
                    }
                    ("POST", "/images/prune") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({"SpaceReclaimed": 10_000_000}),
                        )
                        .await;
                    }
                    ("POST", "/build/prune") => {
                        write_http_json(&mut stream, 200, "OK", &json!({"SpaceReclaimed": 12345}))
                            .await;
                    }
                    ("POST", "/containers/prune") => {
                        write_http_json(&mut stream, 200, "OK", &json!({"SpaceReclaimed": 4096}))
                            .await;
                    }
                    ("POST", "/volumes/prune") => {
                        write_http_json(&mut stream, 200, "OK", &json!({"SpaceReclaimed": 5000}))
                            .await;
                    }
                    ("POST", "/networks/create") => {
                        let parsed: serde_json::Value =
                            serde_json::from_slice(&body).unwrap_or_default();
                        if let Some(name) = parsed["Name"].as_str() {
                            state.lock().unwrap().networks_created.push(name.to_owned());
                        }
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({"Id": "net-new", "Warning": ""}),
                        )
                        .await;
                    }
                    ("POST", "/containers/create") => {
                        let parsed: serde_json::Value =
                            serde_json::from_slice(&body).unwrap_or_default();
                        state.lock().unwrap().created_container_bodies.push(parsed);
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({"Id": "cid-deploy-1", "Warnings": []}),
                        )
                        .await;
                    }
                    ("POST", "/containers/cid-deploy-1/start") => {
                        state
                            .lock()
                            .unwrap()
                            .actions
                            .push("start:cid-deploy-1".into());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", "/containers/cid-deploy-1/json") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": "cid-deploy-1",
                                "Name": "/opsbox-web-d1",
                                "State": { "Running": true, "Restarting": false },
                                "NetworkSettings": {
                                    "Ports": {
                                        "3000/tcp": [{ "HostIp": "127.0.0.1", "HostPort": "45123" }]
                                    }
                                }
                            }),
                        )
                        .await;
                    }
                    ("POST", "/containers/cid-deploy-1/stop") => {
                        state
                            .lock()
                            .unwrap()
                            .actions
                            .push("stop:cid-deploy-1".into());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", "/containers/cid-deploy-1") => {
                        state.lock().unwrap().actions.push("rm:cid-deploy-1".into());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", "/images/create") => {
                        state.lock().unwrap().images.push("pulled:latest".into());
                        let payload = b"{\"status\":\"Pull complete\"}\r\n";
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            payload.len()
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.write_all(payload).await;
                        let _ = stream.flush().await;
                    }
                    ("POST", p) if p.starts_with("/images/") && p.ends_with("/tag") => {
                        state.lock().unwrap().images.push(format!("tag:{query}"));
                        write_http_empty(&mut stream, 201, "Created").await;
                    }
                    ("GET", p) if p.starts_with("/images/") && p.ends_with("/json") => {
                        if p.contains("missing") {
                            write_http_json(
                                &mut stream,
                                404,
                                "Not Found",
                                &json!({"message": "No such image"}),
                            )
                            .await;
                        } else {
                            write_http_json(&mut stream, 200, "OK", &json!({"Id": "sha256:abcd"}))
                                .await;
                        }
                    }
                    ("DELETE", p) if p.starts_with("/images/") => {
                        state.lock().unwrap().actions.push(format!("rmi:{p}"));
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!([{"Deleted": "sha256:abcd"}]),
                        )
                        .await;
                    }
                    _ => {
                        write_http_json(
                            &mut stream,
                            404,
                            "Not Found",
                            &json!({"message": format!("unhandled {method} {path}")}),
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
async fn client_performs_full_lifecycle_and_refuses_bad_refs() {
    let state = Arc::new(Mutex::new(DaemonState::default()));
    let (_dir, sock, server) = spawn_mock_daemon(state.clone()).await;
    let client = DockerClient::new(&sock);

    let (ver, api) = client.ping_version().await.unwrap();
    assert_eq!((ver.as_str(), api.as_str()), ("27.3.1", "1.47"));

    // Containers list (managed and unmanaged)
    let containers = client.list_containers().await.unwrap();
    assert_eq!(containers.len(), 2);
    let web = containers.iter().find(|c| c.name == "web-1").unwrap();
    assert!(web.managed && web.service.as_deref() == Some("web"));
    let worker = containers.iter().find(|c| c.name == "old-worker").unwrap();
    assert!(!worker.managed);

    // Inspect + secret masking + 404 mapping
    let insp = client.inspect_container("web-1").await.unwrap();
    assert_eq!(insp.restart_count, 2);
    assert!(
        insp.env
            .iter()
            .any(|e| e.key == "API_SECRET" && e.masked && e.value.starts_with("••••"))
    );
    assert!(matches!(
        client.inspect_container("missing").await,
        Err(DockerError::NotFound(_))
    ));
    assert!(matches!(
        client.inspect_container("../etc").await,
        Err(DockerError::Invalid(_))
    ));

    // Actions & 409 conflict on removing running container
    client
        .container_action("web-1", DockerContainerAction::Stop)
        .await
        .unwrap();
    client
        .container_action("web-1", DockerContainerAction::Start)
        .await
        .unwrap();
    client
        .container_action("web-1", DockerContainerAction::Restart)
        .await
        .unwrap();
    assert!(matches!(
        client
            .container_action("web-1", DockerContainerAction::Remove)
            .await,
        Err(DockerError::Conflict(_))
    ));
    client
        .container_action("old-worker", DockerContainerAction::Remove)
        .await
        .unwrap();

    // Logs (snapshot + follow)
    let logs = client.logs("web-1", 50).await.unwrap();
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].stream, DockerLogStream::Stdout);
    assert_eq!(logs[1].stream, DockerLogStream::Stderr);

    let mut follow = client.logs_follow("web-1", 50).await.unwrap();
    let mut collected = Vec::new();
    while let Some(item) = follow.next().await {
        collected.push(item.unwrap().text);
    }
    assert_eq!(
        collected,
        vec!["server started", "warning: slow boot", "live line"]
    );

    // Stats (CPU% & memory minus inactive_file)
    let stats = client.stats().await.unwrap();
    assert_eq!(stats.len(), 1);
    assert!((stats[0].cpu_pct - 40.0).abs() < 0.01);
    assert_eq!(stats[0].mem_used_bytes, 40 * 1024 * 1024);

    // Exec
    let exec = client
        .exec(DockerExecRequest {
            id: "web-1".into(),
            cmd: vec!["echo".into(), "ok".into()],
            user: None,
            env: vec![],
            working_dir: None,
            stdin: None,
        })
        .await
        .unwrap();
    assert_eq!(exec.exit_code, 0);
    assert_eq!(exec.stdout, b"ok\n");

    // Streaming exec & volume/pause helpers
    let mut streamed = Vec::new();
    let stream_status = client
        .exec_to_writer(
            DockerExecRequest {
                id: "web-1".into(),
                cmd: vec!["pg_dump".into(), "-Fc".into()],
                user: None,
                env: vec![],
                working_dir: None,
                stdin: None,
            },
            &mut streamed,
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(stream_status.exit_code, 0);
    assert_eq!(stream_status.bytes_written, 3);
    assert_eq!(streamed, b"ok\n");

    assert_eq!(
        client.inspect_volume_mountpoint("web_data").await.unwrap(),
        "/var/lib/docker/volumes/web_data/_data"
    );
    let mounting = client
        .containers_mounting_volumes(&["web_data".into()])
        .await
        .unwrap();
    assert_eq!(mounting, vec!["11112222333344445555666677778888"]);
    client
        .pause_container("11112222333344445555666677778888")
        .await
        .unwrap();
    client
        .unpause_container("11112222333344445555666677778888")
        .await
        .unwrap();

    // Images, volumes, networks
    let imgs = client.list_images().await.unwrap();
    assert_eq!(imgs.len(), 2);
    assert!(imgs.iter().any(|i| i.name == "nginx" && i.used));
    assert!(imgs.iter().any(|i| i.name == "<none>" && !i.used));

    let vols = client.list_volumes().await.unwrap();
    assert!(vols.iter().any(|v| v.name == "web_data" && v.used));
    assert!(vols.iter().any(|v| v.name == "unused_vol" && !v.used));

    let nets = client.list_networks().await.unwrap();
    assert_eq!(nets[0].name, "bridge");
    assert_eq!(nets[0].containers, 1);

    // Cleanup scan, volume sizes, prune
    let scan = client.scan_cleanup().await.unwrap();
    assert_eq!(scan.dangling_images_bytes, 10_000_000);
    assert_eq!(scan.dangling_images_count, 1);
    assert_eq!(scan.build_cache_bytes, 12345);
    assert_eq!(scan.stopped_containers_bytes, 4096);
    assert_eq!(scan.stopped_containers_count, 1);
    assert_eq!(scan.dangling_volumes_count, 1);
    assert_eq!(client.volume_sizes().await.unwrap(), (5000, 1));
    assert_eq!(
        client.prune(DockerPruneKind::Images).await.unwrap(),
        10_000_000
    );
    assert_eq!(
        client.prune(DockerPruneKind::BuildCache).await.unwrap(),
        12345
    );
    assert_eq!(
        client.prune(DockerPruneKind::Containers).await.unwrap(),
        4096
    );
    assert_eq!(client.prune(DockerPruneKind::Volumes).await.unwrap(), 5000);

    server.abort();
}

#[tokio::test]
async fn missing_socket_reports_daemon_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let client = DockerClient::new(dir.path().join("nope.sock"));
    assert!(matches!(
        client.ping_version().await,
        Err(DockerError::DaemonUnavailable(_))
    ));
}

#[tokio::test]
async fn deploy_container_lifecycle_and_images_work_over_unix_socket() {
    use opsbox_docker::ServiceContainerSpec;

    let state = Arc::new(Mutex::new(DaemonState::default()));
    let (_dir, sock, server) = spawn_mock_daemon(state.clone()).await;
    let client = DockerClient::new(&sock);

    let (cid, host_port) = client
        .create_and_start_service_container(&ServiceContainerSpec {
            project: "default",
            service: "web",
            deploy_id: "dep-1",
            container_name: "opsbox-web-dep1",
            network_name: "opsbox-default",
            image_ref: "opsbox/web:dep1",
            env: &["NODE_ENV=production".into()],
            port: 3000,
            memory_limit_mb: Some(512),
            cpu_limit_millis: Some(1500),
        })
        .await
        .unwrap();
    assert_eq!(cid, "cid-deploy-1");
    assert_eq!(host_port, 45123);

    {
        let st = state.lock().unwrap();
        assert_eq!(st.networks_created, vec!["opsbox-default"]);
        let body = &st.created_container_bodies[0];
        assert_eq!(body["Labels"]["opsbox.service"], "web");
        assert_eq!(body["HostConfig"]["Memory"], 512 * 1024 * 1024);
        assert_eq!(body["HostConfig"]["NanoCpus"], 1_500_000_000);
        assert_eq!(
            body["HostConfig"]["PortBindings"]["3000/tcp"][0]["HostIp"],
            "127.0.0.1"
        );
        assert_eq!(
            body["NetworkingConfig"]["EndpointsConfig"]["opsbox-default"]["Aliases"][0],
            "web"
        );
    }

    assert!(client.is_container_running("cid-deploy-1").await.unwrap());
    client
        .pull_and_tag_image("nginx:1.27-alpine", "opsbox/web:dep2")
        .await
        .unwrap();
    assert!(client.image_exists("opsbox/web:dep1").await.unwrap());
    assert!(!client.image_exists("opsbox/web:missing").await.unwrap());
    assert!(client.remove_image("opsbox/web:dep1").await.unwrap());
    client
        .force_stop_and_remove_container("cid-deploy-1")
        .await
        .unwrap();
    server.abort();
}
