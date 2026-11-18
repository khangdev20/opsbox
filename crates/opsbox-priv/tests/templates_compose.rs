//! End-to-end privileged tests for built-in service templates (`TemplateStart`)
//! and Docker Compose (`ComposeValidate`, `ComposeDeploy`, `ComposeRemoveService`, `ComposeDown`).

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use nix::unistd::Uid;
use opsbox_priv::{ClientError, PrivClient, PrivConfig, PrivServer};
use opsbox_proto::{
    ComposeDeployRequest, ComposeValidateRequest, DatabaseRotateRequest, DeployEnvSpec,
    DeployRefSource, PrivErrorCode, Secret, TemplateStartRequest,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

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
    network_connections: Vec<(String, String, Vec<String>)>,
    created_bodies: Vec<Value>,
    exec_exit_code: i64,
    removed_containers: Vec<String>,
    compose_containers: Vec<(String, String, Option<u16>)>,
    running_containers: HashMap<String, bool>,
    exec_bodies: Vec<(String, Value)>,
}

fn spawn_mock_docker(sock_path: &std::path::Path, state: Arc<Mutex<MockDockerState>>) {
    let listener = UnixListener::bind(sock_path).unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let st = state.clone();
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
                            write_http_json(
                                &mut stream,
                                200,
                                "OK",
                                &json!({ "Name": name, "Id": format!("net-{name}") }),
                            )
                            .await;
                        } else {
                            write_http_json(
                                &mut stream,
                                404,
                                "Not Found",
                                &json!({"message": "missing network"}),
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
                        let aliases = v["EndpointConfig"]["Aliases"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(str::to_owned))
                                    .collect()
                            })
                            .unwrap_or_default();
                        st.lock()
                            .unwrap()
                            .network_connections
                            .push((net, cid, aliases));
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", "/containers/create") => {
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let cid = {
                            let mut guard = st.lock().unwrap();
                            guard.created_bodies.push(v);
                            let id = format!("tpl-cid-{}", guard.created_bodies.len());
                            guard.running_containers.insert(id.clone(), true);
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
                                .filter(|(id, _, _)| !guard.removed_containers.contains(id))
                                .filter(|_| query.contains("com.docker.compose.project"))
                                .map(|(id, svc_name, _)| {
                                    json!({
                                        "Id": id,
                                        "Names": [format!("/{svc_name}")],
                                        "Image": "nginx:1.27-alpine",
                                        "Created": 1700000000,
                                        "State": "running",
                                        "Status": "Up 5 seconds",
                                        "Ports": [],
                                        "Labels": {
                                            "com.docker.compose.project": "opsbox-shop",
                                            "com.docker.compose.service": svc_name
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
                            .trim_end_matches("/json")
                            .to_owned();
                        let (running, compose_port, tpl_port) = {
                            let guard = st.lock().unwrap();
                            let running =
                                guard.running_containers.get(&cid).copied().unwrap_or(true);
                            let cp = guard
                                .compose_containers
                                .iter()
                                .find(|(id, _, _)| id == &cid)
                                .and_then(|(_, _, port)| *port);
                            let tp = cid
                                .strip_prefix("tpl-cid-")
                                .and_then(|n| n.parse::<usize>().ok())
                                .and_then(|idx| guard.created_bodies.get(idx - 1))
                                .and_then(|body| {
                                    body["HostConfig"]["PortBindings"]
                                        .as_object()
                                        .and_then(|m| m.iter().next())
                                        .map(|(k, arr)| {
                                            let ip = arr
                                                .as_array()
                                                .and_then(|a| a.first())
                                                .and_then(|o| o["HostIp"].as_str())
                                                .unwrap_or("127.0.0.1")
                                                .to_owned();
                                            (k.clone(), ip)
                                        })
                                });
                            (running, cp, tp)
                        };
                        let ports = match (compose_port, tpl_port) {
                            (Some(hp), _) => json!({
                                "80/tcp": [{ "HostIp": "127.0.0.1", "HostPort": hp.to_string() }]
                            }),
                            (None, Some((key, ip))) => json!({
                                key: [{ "HostIp": ip, "HostPort": "35432" }]
                            }),
                            (None, None) => json!({}),
                        };
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": cid,
                                "State": { "Running": running },
                                "NetworkSettings": {
                                    "Ports": ports,
                                    "Networks": {}
                                }
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
                        let exit_code = st.lock().unwrap().exec_exit_code;
                        let resp = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\nContent-Type: application/vnd.docker.raw-stream\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let msg = if exit_code == 0 {
                            b"accepting connections\n".as_slice()
                        } else {
                            b"pg fatal failure\n".as_slice()
                        };
                        let _ = stream.write_all(&mux_frame(1, msg)).await;
                        let _ = stream.flush().await;
                        let _ = stream.shutdown().await;
                    }
                    ("GET", "/exec/exec-1/json") => {
                        let code = st.lock().unwrap().exec_exit_code;
                        if code != 0 {
                            // Stop simulated container after first failed health probe so test exits fast.
                            let mut guard = st.lock().unwrap();
                            for v in guard.running_containers.values_mut() {
                                *v = false;
                            }
                        }
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({"ExitCode": code, "Running": false}),
                        )
                        .await;
                    }
                    ("GET", p) if p.starts_with("/containers/") && p.ends_with("/logs") => {
                        let frame = mux_frame(2, b"startup failed password=pg-top-secret-pw\n");
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.docker.raw-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            frame.len()
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.write_all(&frame).await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/stop") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", p) if p.starts_with("/containers/") => {
                        let cid = p.trim_start_matches("/containers/").to_owned();
                        st.lock().unwrap().removed_containers.push(cid);
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    _ => write_http_empty(&mut stream, 404, "Not Found").await,
                }
            });
        }
    });
}

#[tokio::test]
async fn template_start_creates_volume_keeps_internal_only_and_scrubs_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let docker_sock = tmp.path().join("docker.sock");
    let state = Arc::new(Mutex::new(MockDockerState::default()));
    spawn_mock_docker(&docker_sock, state.clone());

    let priv_sock = tmp.path().join("priv.sock");
    let mut cfg = PrivConfig::new(&priv_sock, vec![Uid::current().as_raw()]);
    cfg.capacity_profile = Some(opsbox_capacity::Profile {
        memory_mb: 8192,
        available_mb: 8192,
        cpu_millis: 4000,
        online_cpus: 4,
        cgroup_v2: true,
        disks: vec![opsbox_capacity::Disk {
            path: "/".into(),
            total_mb: 102400,
            free_mb: 80000,
        }],
        ..Default::default()
    });
    cfg.capacity_image_size_mb = Some(64);
    cfg.master_key_path = tmp.path().join("master.key");
    cfg.xfer_dir = tmp.path().join("xfer");
    cfg.trash_dir = tmp.path().join("trash");
    cfg.apps_dir = tmp.path().join("apps");
    cfg.docker_socket = docker_sock;

    let srv = PrivServer::bind(cfg).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let srv_task = tokio::spawn(async move {
        srv.run(async {
            let _ = rx.await;
        })
        .await;
    });

    let client = PrivClient::new(&priv_sock);
    let sealed_pw = client
        .secret_seal(
            "s1",
            None,
            "deploy:var:pg:POSTGRES_PASSWORD",
            Secret::new("pg-top-secret-pw"),
        )
        .await
        .unwrap();
    let sealed_user = client
        .secret_seal(
            "s2",
            None,
            "deploy:var:pg:POSTGRES_USER",
            Secret::new("app"),
        )
        .await
        .unwrap();
    let sealed_pghost = client
        .secret_seal("s3", None, "deploy:var:pg:PGHOST", Secret::new("pg-main"))
        .await
        .unwrap();

    let vars = vec![
        DeployEnvSpec {
            key: "POSTGRES_USER".into(),
            value_enc: sealed_user,
            seal_context: "deploy:var:pg:POSTGRES_USER".into(),
            is_secret: false,
        },
        DeployEnvSpec {
            key: "POSTGRES_PASSWORD".into(),
            value_enc: sealed_pw,
            seal_context: "deploy:var:pg:POSTGRES_PASSWORD".into(),
            is_secret: true,
        },
        DeployEnvSpec {
            key: "PGHOST".into(),
            value_enc: sealed_pghost,
            seal_context: "deploy:var:pg:PGHOST".into(),
            is_secret: false,
        },
    ];

    let started = client
        .template_start(
            "t1",
            "admin",
            TemplateStartRequest {
                project: "acme".into(),
                service: "pg-main".into(),
                deploy_id: "dep-1".into(),
                template_id: "postgres".into(),
                version: "16.4".into(),
                image_ref: "postgres:16.4".into(),
                internal_only: true,
                public_port: false,
                confirm_public_port: false,
                vars: vars.clone(),
                ref_sources: vec![],
                memory_limit_mb: Some(256),
                cpu_limit_millis: None,
                data_volume: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(started.host_port, 0);
    assert_eq!(started.container_id, "tpl-cid-1");

    {
        let st = state.lock().unwrap();
        assert!(st.volumes.contains("opsbox-pg-main-data"));
        assert!(st.networks.contains("opsbox-acme"));
        let create = &st.created_bodies[0];
        assert!(
            create["HostConfig"]["PortBindings"].is_null(),
            "internal_only container must not bind host ports: {create}"
        );
        let binds = create["HostConfig"]["Binds"].as_array().unwrap();
        assert_eq!(binds[0], "opsbox-pg-main-data:/var/lib/postgresql/data");
        let env_arr = create["Env"].as_array().unwrap();
        assert!(
            env_arr
                .iter()
                .any(|e| e == "POSTGRES_PASSWORD=pg-top-secret-pw")
        );
        assert!(
            !env_arr
                .iter()
                .any(|e| e.as_str().unwrap_or("").starts_with("PGHOST=")),
            "client connection helper PGHOST must not be passed to postgres server container"
        );
    }

    // Health check failure must scrub secrets from container logs and clean up container.
    state.lock().unwrap().exec_exit_code = 1;
    let err = client
        .template_start(
            "t2",
            "admin",
            TemplateStartRequest {
                project: "acme".into(),
                service: "pg-main".into(),
                deploy_id: "dep-2".into(),
                template_id: "postgres".into(),
                version: "16.4".into(),
                image_ref: "postgres:16.4".into(),
                internal_only: true,
                public_port: false,
                confirm_public_port: false,
                vars,
                ref_sources: vec![],
                memory_limit_mb: None,
                cpu_limit_millis: None,
                data_volume: None,
            },
        )
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(!msg.contains("pg-top-secret-pw"), "secret leaked: {msg}");
    assert!(
        msg.contains("[REDACTED]"),
        "expected redacted marker: {msg}"
    );
    assert!(
        state
            .lock()
            .unwrap()
            .removed_containers
            .contains(&"tpl-cid-2".to_owned())
    );

    let _ = tx.send(());
    let _ = srv_task.await;
}

#[tokio::test]
async fn compose_enforces_safety_runs_up_attaches_network_and_cleans_up() {
    let tmp = tempfile::tempdir().unwrap();
    let docker_sock = tmp.path().join("docker.sock");
    let state = Arc::new(Mutex::new(MockDockerState {
        compose_containers: vec![
            ("cmp-web-1".into(), "web".into(), Some(3000)),
            ("cmp-cache-1".into(), "cache".into(), None),
        ],
        ..Default::default()
    }));
    spawn_mock_docker(&docker_sock, state.clone());

    // Fake `docker` CLI implementing `docker compose config --format json`, `up -d`, `down`.
    let fake_docker = tmp.path().join("docker");
    let up_log = tmp.path().join("compose_up.log");
    let script = format!(
        r#"#!/bin/sh
# Minimal docker compose stub that handles config --format json / up / down.
sub=""
file=""
for a in "$@"; do
  case "$a" in
    config|up|down) sub="$a" ;;
  esac
done
prev=""
for a in "$@"; do
  if [ "$prev" = "-f" ]; then
    file="$a"
    break
  fi
  prev="$a"
done
if [ "$sub" = "config" ]; then
  if grep -q "network_mode: host" "$file"; then
    echo '{{"services":{{"web":{{"image":"nginx:1.27-alpine","network_mode":"host"}}}}}}'
    exit 0
  fi
  if grep -q "privileged: true" "$file"; then
    echo '{{"services":{{"web":{{"image":"nginx:1.27-alpine","privileged":true}}}}}}'
    exit 0
  fi
  echo '{{"services":{{"web":{{"image":"nginx:1.27-alpine","ports":[{{"published":"3000","target":80}}],"depends_on":{{"cache":{{"condition":"service_started"}}}}}},"cache":{{"image":"redis:7-alpine"}}}}}}'
  exit 0
fi
if [ "$sub" = "up" ]; then
  printf '%s\n' "$@" > "{up_log}"
  echo "Container opsbox-shop-web-1 Started"
  echo "Container opsbox-shop-cache-1 Started"
  exit 0
fi
if [ "$sub" = "down" ]; then
  echo "Network opsbox-shop_default Removed"
  exit 0
fi
exit 1
"#,
        up_log = up_log.display(),
    );
    std::fs::write(&fake_docker, script).unwrap();
    std::fs::set_permissions(&fake_docker, std::fs::Permissions::from_mode(0o755)).unwrap();

    let priv_sock = tmp.path().join("priv.sock");
    let mut cfg = PrivConfig::new(&priv_sock, vec![Uid::current().as_raw()]);
    cfg.capacity_profile = Some(opsbox_capacity::Profile {
        memory_mb: 8192,
        available_mb: 8192,
        cpu_millis: 4000,
        online_cpus: 4,
        cgroup_v2: true,
        disks: vec![opsbox_capacity::Disk {
            path: "/".into(),
            total_mb: 102400,
            free_mb: 80000,
        }],
        ..Default::default()
    });
    cfg.capacity_image_size_mb = Some(64);
    cfg.master_key_path = tmp.path().join("master.key");
    cfg.xfer_dir = tmp.path().join("xfer");
    cfg.trash_dir = tmp.path().join("trash");
    cfg.apps_dir = tmp.path().join("apps");
    cfg.docker_socket = docker_sock;
    cfg.docker_bin = Some(fake_docker);

    let srv = PrivServer::bind(cfg).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let srv_task = tokio::spawn(async move {
        srv.run(async {
            let _ = rx.await;
        })
        .await;
    });

    let client = PrivClient::new(&priv_sock);

    // 1) Host path escape via env_file is refused before touching disk.
    let err = client
        .compose_validate(
            "c1",
            "admin",
            ComposeValidateRequest {
                stack: "shop".into(),
                yaml: "services:\n  web:\n    image: nginx:1.27-alpine\n    env_file: ../outside.env\n"
                    .into(),
                confirm: false,
            },
        )
        .await
        .unwrap_err();
    match err {
        opsbox_priv::ClientError::Remote(r) => assert_eq!(r.code, PrivErrorCode::Invalid),
        other => panic!("unexpected err: {other:?}"),
    }
    assert!(!tmp.path().join("apps/shop").exists());

    // 2) `network_mode: host` requires confirmation and leaves zero files on refusal.
    let err = client
        .compose_validate(
            "c2",
            "admin",
            ComposeValidateRequest {
                stack: "shop".into(),
                yaml: "services:\n  web:\n    image: nginx:1.27-alpine\n    network_mode: host\n"
                    .into(),
                confirm: false,
            },
        )
        .await
        .unwrap_err();
    match err {
        opsbox_priv::ClientError::Remote(r) => assert_eq!(r.code, PrivErrorCode::ConfirmRequired),
        other => panic!("unexpected err: {other:?}"),
    }
    assert!(!tmp.path().join("apps/shop").exists());

    // 3) Dry validation on safe Compose returns service graph and leaves zero files.
    let safe_yaml = r#"
services:
  web:
    image: nginx:1.27-alpine
    ports: ["127.0.0.1:3000:80"]
    depends_on: [cache]
  cache:
    image: redis:7-alpine
"#;
    let valid = client
        .compose_validate(
            "c3",
            "admin",
            ComposeValidateRequest {
                stack: "shop".into(),
                yaml: safe_yaml.into(),
                confirm: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(valid.project_name, "opsbox-shop");
    assert_eq!(valid.services.len(), 2);
    assert!(!tmp.path().join("apps/shop").exists());

    // 4) Full Compose deploy writes compose.yml, runs up -d, and attaches containers to opsbox-<project>.
    let deployed = client
        .compose_deploy(
            "c4",
            "admin",
            ComposeDeployRequest {
                project: "acme".into(),
                stack: "shop".into(),
                yaml: safe_yaml.into(),
                confirm: false,
                vars: vec![],
                ref_sources: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(deployed.project_name, "opsbox-shop");
    assert!(tmp.path().join("apps/shop/compose.yml").is_file());
    let mode = std::fs::metadata(tmp.path().join("apps/shop/compose.yml"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o640);
    let web = deployed.services.iter().find(|s| s.name == "web").unwrap();
    assert_eq!(web.host_port, Some(3000));
    assert_eq!(web.container_id.as_deref(), Some("cmp-web-1"));
    assert_eq!(web.depends_on, vec!["cache".to_owned()]);
    {
        let st = state.lock().unwrap();
        assert_eq!(st.network_connections.len(), 2);
        assert!(
            st.network_connections
                .iter()
                .all(|(net, _, _)| net == "opsbox-acme")
        );
    }

    // 5) Remove single compose service container, then tear down whole stack cleanly.
    client
        .compose_remove_service("c5", "admin", "shop", "cache")
        .await
        .unwrap();
    assert!(
        state
            .lock()
            .unwrap()
            .removed_containers
            .contains(&"cmp-cache-1".to_owned())
    );

    client
        .compose_down("c6", "admin", "shop", true)
        .await
        .unwrap();
    assert!(!tmp.path().join("apps/shop").exists());

    let _ = tx.send(());
    let _ = srv_task.await;
}

#[tokio::test]
async fn template_ref_resolution_public_port_guard_and_db_rotation() {
    let tmp = tempfile::tempdir().unwrap();
    let docker_sock = tmp.path().join("docker.sock");
    let state = Arc::new(Mutex::new(MockDockerState::default()));
    spawn_mock_docker(&docker_sock, state.clone());

    let priv_sock = tmp.path().join("priv.sock");
    let mut cfg = PrivConfig::new(&priv_sock, vec![Uid::current().as_raw()]);
    cfg.capacity_profile = Some(opsbox_capacity::Profile {
        memory_mb: 8192,
        available_mb: 8192,
        cpu_millis: 4000,
        online_cpus: 4,
        cgroup_v2: true,
        disks: vec![opsbox_capacity::Disk {
            path: "/".into(),
            total_mb: 102400,
            free_mb: 80000,
        }],
        ..Default::default()
    });
    cfg.capacity_image_size_mb = Some(64);
    cfg.master_key_path = tmp.path().join("master.key");
    cfg.xfer_dir = tmp.path().join("xfer");
    cfg.trash_dir = tmp.path().join("trash");
    cfg.apps_dir = tmp.path().join("apps");
    cfg.docker_socket = docker_sock;

    let srv = PrivServer::bind(cfg).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let srv_task = tokio::spawn(async move {
        srv.run(async {
            let _ = rx.await;
        })
        .await;
    });
    let client = PrivClient::new(&priv_sock);

    let seal = |ctx: &str, val: &str| {
        let client = &client;
        let ctx = ctx.to_owned();
        let val = val.to_owned();
        async move {
            client
                .secret_seal("s", Some("admin"), &ctx, Secret::new(val))
                .await
                .unwrap()
        }
    };

    let pg_user = seal("deploy:var:pg-1:POSTGRES_USER", "appuser").await;
    let pg_db = seal("deploy:var:pg-1:POSTGRES_DB", "appdb").await;
    let pg_pw = seal("deploy:var:pg-1:POSTGRES_PASSWORD", "old-pg-secret-123").await;
    let pg_pgpw = seal("deploy:var:pg-1:PGPASSWORD", "old-pg-secret-123").await;
    let pg_url = seal(
        "deploy:var:pg-1:DATABASE_URL",
        "postgresql://appuser:old-pg-secret-123@pg-main:5432/appdb",
    )
    .await;
    let pg_specs = vec![
        DeployEnvSpec {
            key: "POSTGRES_USER".into(),
            value_enc: pg_user,
            seal_context: "deploy:var:pg-1:POSTGRES_USER".into(),
            is_secret: false,
        },
        DeployEnvSpec {
            key: "POSTGRES_DB".into(),
            value_enc: pg_db,
            seal_context: "deploy:var:pg-1:POSTGRES_DB".into(),
            is_secret: false,
        },
        DeployEnvSpec {
            key: "POSTGRES_PASSWORD".into(),
            value_enc: pg_pw,
            seal_context: "deploy:var:pg-1:POSTGRES_PASSWORD".into(),
            is_secret: true,
        },
        DeployEnvSpec {
            key: "PGPASSWORD".into(),
            value_enc: pg_pgpw,
            seal_context: "deploy:var:pg-1:PGPASSWORD".into(),
            is_secret: true,
        },
        DeployEnvSpec {
            key: "DATABASE_URL".into(),
            value_enc: pg_url,
            seal_context: "deploy:var:pg-1:DATABASE_URL".into(),
            is_secret: true,
        },
    ];

    // 1) Public port without confirmation MUST be rejected by opsbox-priv with ConfirmRequired.
    let err = client
        .template_start(
            "r1",
            "admin",
            TemplateStartRequest {
                project: "acme".into(),
                service: "pg-main".into(),
                deploy_id: "dep-guard".into(),
                template_id: "postgres".into(),
                version: "16.4".into(),
                image_ref: "postgres:16.4".into(),
                internal_only: false,
                public_port: true,
                confirm_public_port: false,
                vars: pg_specs.clone(),
                ref_sources: vec![],
                memory_limit_mb: None,
                cpu_limit_millis: None,
                data_volume: None,
            },
        )
        .await
        .unwrap_err();
    match err {
        ClientError::Remote(pe) => assert_eq!(pe.code, PrivErrorCode::ConfirmRequired),
        other => panic!("expected ConfirmRequired, got {other:?}"),
    }
    assert!(state.lock().unwrap().created_bodies.is_empty());

    // 2) Confirming public port binds 0.0.0.0 intentionally.
    let started_pub = client
        .template_start(
            "r2",
            "admin",
            TemplateStartRequest {
                project: "acme".into(),
                service: "pg-main".into(),
                deploy_id: "dep-pub".into(),
                template_id: "postgres".into(),
                version: "16.4".into(),
                image_ref: "postgres:16.4".into(),
                internal_only: false,
                public_port: true,
                confirm_public_port: true,
                vars: pg_specs.clone(),
                ref_sources: vec![],
                memory_limit_mb: None,
                cpu_limit_millis: None,
                data_volume: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(started_pub.host_port, 35432);
    {
        let st = state.lock().unwrap();
        let pb = &st.created_bodies[0]["HostConfig"]["PortBindings"]["5432/tcp"];
        assert_eq!(pb[0]["HostIp"], "0.0.0.0");
    }

    // 3) Reference resolution on an app template (`umami`) resolves `${{ pg-main.DATABASE_URL }}`
    // inside `priv` and injects the real connection string into the container Env.
    let umami_db_url = seal(
        "deploy:var:umami-1:DATABASE_URL",
        "${{ pg-main.DATABASE_URL }}",
    )
    .await;
    let umami_secret = seal("deploy:var:umami-1:APP_SECRET", "umami-secret-val").await;
    let umami_started = client
        .template_start(
            "r3",
            "admin",
            TemplateStartRequest {
                project: "acme".into(),
                service: "analytics".into(),
                deploy_id: "dep-umami".into(),
                template_id: "umami".into(),
                version: "postgresql-latest".into(),
                image_ref: "ghcr.io/umami-software/umami:postgresql-latest".into(),
                internal_only: true,
                public_port: false,
                confirm_public_port: false,
                vars: vec![
                    DeployEnvSpec {
                        key: "DATABASE_URL".into(),
                        value_enc: umami_db_url,
                        seal_context: "deploy:var:umami-1:DATABASE_URL".into(),
                        is_secret: false,
                    },
                    DeployEnvSpec {
                        key: "APP_SECRET".into(),
                        value_enc: umami_secret,
                        seal_context: "deploy:var:umami-1:APP_SECRET".into(),
                        is_secret: true,
                    },
                ],
                ref_sources: vec![DeployRefSource {
                    service: "pg-main".into(),
                    vars: pg_specs.clone(),
                }],
                memory_limit_mb: None,
                cpu_limit_millis: None,
                data_volume: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(umami_started.container_id, "tpl-cid-2");
    {
        let st = state.lock().unwrap();
        let env_arr = st.created_bodies[1]["Env"].as_array().unwrap();
        assert!(
            env_arr.iter().any(|e| {
                e == "DATABASE_URL=postgresql://appuser:old-pg-secret-123@pg-main:5432/appdb"
            }),
            "resolved ref must be injected into app env: {env_arr:?}"
        );
    }

    // 4) Credential rotation runs `docker exec` on the live DB container with password via Env
    // (never argv) and returns newly sealed POSTGRES_PASSWORD / PGPASSWORD / DATABASE_URL.
    let rotated = client
        .database_rotate(
            "r4",
            "admin",
            DatabaseRotateRequest {
                service_id: "pg-1".into(),
                service_name: "pg-main".into(),
                template_id: "postgres".into(),
                container_id: started_pub.container_id.clone(),
                vars: pg_specs,
            },
        )
        .await
        .unwrap();
    assert_eq!(rotated.vars.len(), 3);
    {
        let st = state.lock().unwrap();
        let (cid, exec_body) = st.exec_bodies.last().unwrap();
        assert_eq!(cid, "tpl-cid-1");
        let cmd = exec_body["Cmd"].as_array().unwrap();
        let cmd_joined = cmd
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            !cmd_joined.contains("old-pg-secret-123"),
            "old secret must not appear in exec argv"
        );
        assert_eq!(cmd[0], "psql");
        let env_arr = exec_body["Env"].as_array().unwrap();
        assert!(
            env_arr.iter().any(|e| e == "PGPASSWORD=old-pg-secret-123"),
            "expected PGPASSWORD in exec Env: {env_arr:?}"
        );
    }
    let new_url_spec = rotated
        .vars
        .iter()
        .find(|v| v.key == "DATABASE_URL")
        .unwrap();
    let reopened = client
        .secret_open(
            "o1",
            Some("admin"),
            &new_url_spec.seal_context,
            &new_url_spec.value_enc,
        )
        .await
        .unwrap();
    assert!(
        reopened.expose().starts_with("postgresql://appuser:")
            && !reopened.expose().contains("old-pg-secret-123")
            && reopened.expose().ends_with("@pg-main:5432/appdb"),
        "unexpected rotated DATABASE_URL: {}",
        reopened.expose()
    );

    let _ = tx.send(());
    let _ = srv_task.await;
}
