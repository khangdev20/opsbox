//! End-to-end tests of `PrivServer` <-> `PrivClient` deploy build streaming,
//! BuildKit `--secret` mounts, secret scrubbing, and loopback HTTP health checks.

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nix::unistd::Uid;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::{
    DeployBuildFrame, DeployBuildRequest, DeployEnvSpec, DeployGitSource, DeployStartRequest,
    Secret,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixListener, UnixStream};

fn init_local_git_repo(dir: &std::path::Path, files: &[(&str, &str)]) -> String {
    Command::new("git")
        .args(["init", "-b", "main"])
        .arg(dir)
        .output()
        .unwrap();
    for (name, body) in files {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, body).unwrap();
    }
    for args in [
        vec!["config", "user.name", "Opsbox Test"],
        vec!["config", "user.email", "test@opsbox.invalid"],
        vec!["add", "."],
        vec!["commit", "-m", "Initial deploy commit"],
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

fn mux_frame(stream: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.push(stream);
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&(u32::try_from(payload.len()).unwrap()).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

#[tokio::test]
async fn build_streams_scrubs_secrets_uses_secret_mount_and_health_checks_loopback() {
    let tmp = tempfile::tempdir().unwrap();
    let repo_dir = tmp.path().join("repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let sha = init_local_git_repo(&repo_dir, &[("index.html", "<h1>Hello Opsbox</h1>\n")]);

    let args_log = tmp.path().join("docker_args.txt");
    let secret_seen = tmp.path().join("secret_seen.txt");
    let dockerfile_seen = tmp.path().join("dockerfile_seen.txt");
    let fake_docker = tmp.path().join("docker");
    let script = format!(
        r#"#!/bin/sh
if [ "$1" = "buildx" ] && {{ [ "$2" = "create" ] || [ "$2" = "rm" ]; }}; then exit 0; fi
if [ "$1" = "buildx" ] && [ "$2" = "version" ]; then
  echo "github.com/docker/buildx v0.17.1"
  exit 0
fi
if [ "$3" = "--help" ] || [ "$2" = "--help" ]; then
  echo "  --progress string"
  echo "  --secret stringArray"
  echo "  --build-arg stringArray"
  exit 0
fi
printf '%s\n' "$@" > "{args_log}"
next_df=""
for a in "$@"; do
  if [ "$next_df" = "1" ]; then
    cp "$a" "{dockerfile_seen}"
    next_df=""
    continue
  fi
  case "$a" in
    -f) next_df="1" ;;
    id=DB_PASS,src=*)
      src_file="${{a#id=DB_PASS,src=}}"
      cat "$src_file" > "{secret_seen}"
      ;;
  esac
done
echo "Step 1: using secret value super-secret-pw-9988"
echo "Successfully tagged opsbox/web:dep1"
exit 0
"#,
        args_log = args_log.display(),
        dockerfile_seen = dockerfile_seen.display(),
        secret_seen = secret_seen.display(),
    );
    std::fs::write(&fake_docker, script).unwrap();
    std::fs::set_permissions(&fake_docker, std::fs::Permissions::from_mode(0o755)).unwrap();

    // Local TCP HTTP listener simulating container mapped port on 127.0.0.1.
    let app_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_port = app_listener.local_addr().unwrap().port();
    let health_status = Arc::new(Mutex::new(200u16));
    let hs_srv = health_status.clone();
    let app_task = tokio::spawn(async move {
        while let Ok((mut s, _)) = app_listener.accept().await {
            let hs = hs_srv.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf).await;
                let code = *hs.lock().unwrap();
                let resp = format!(
                    "HTTP/1.1 {code} STATUS\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                );
                let _ = s.write_all(resp.as_bytes()).await;
            });
        }
    });

    // Mock Docker Engine Unix socket.
    let docker_sock = tmp.path().join("docker.sock");
    let listener = UnixListener::bind(&docker_sock).unwrap();
    let removed = Arc::new(Mutex::new(Vec::<String>::new()));
    let rem_srv = removed.clone();
    let docker_task = tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let rem = rem_srv.clone();
            tokio::spawn(async move {
                let Some((method, target, _body)) = read_http_req(&mut stream).await else {
                    return;
                };
                let path = strip_api_prefix(target.split('?').next().unwrap_or(&target));
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
                            &json!({"Id": "net-1", "Warning": ""}),
                        )
                        .await;
                    }
                    ("POST", "/containers/create") => {
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({"Id": "cid-1", "Warnings": []}),
                        )
                        .await;
                    }
                    ("POST", "/containers/cid-1/start") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", "/containers/cid-1/json") => {
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": "cid-1",
                                "State": { "Running": true },
                                "NetworkSettings": {
                                    "Ports": {
                                        "80/tcp": [{ "HostIp": "127.0.0.1", "HostPort": app_port.to_string() }]
                                    }
                                }
                            }),
                        )
                        .await;
                    }
                    ("GET", "/containers/cid-1/logs") => {
                        let frame = mux_frame(2, b"fatal with super-secret-pw-9988\n");
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.docker.raw-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            frame.len()
                        );
                        let _ = stream.write_all(resp.as_bytes()).await;
                        let _ = stream.write_all(&frame).await;
                    }
                    ("POST", "/containers/cid-1/stop") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", "/containers/cid-1") => {
                        rem.lock().unwrap().push("cid-1".into());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    _ => write_http_empty(&mut stream, 404, "Not Found").await,
                }
            });
        }
    });

    let priv_sock = tmp.path().join("priv.sock");
    let uid = Uid::current().as_raw();
    let mut cfg = PrivConfig::new(&priv_sock, vec![uid]);
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
    cfg.allow_test_git = true;

    let srv = PrivServer::bind(cfg).unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let srv_task = tokio::spawn(async move {
        srv.run(async {
            let _ = rx.await;
        })
        .await;
    });

    let client = PrivClient::new(&priv_sock);
    let sealed_secret = client
        .secret_seal(
            "r1",
            None,
            "deploy:var:svc1:DB_PASS",
            Secret::new("super-secret-pw-9988"),
        )
        .await
        .unwrap();
    let sealed_plain = client
        .secret_seal(
            "r2",
            None,
            "deploy:var:svc1:APP_ENV",
            Secret::new("production"),
        )
        .await
        .unwrap();

    let vars = vec![
        DeployEnvSpec {
            key: "DB_PASS".into(),
            value_enc: sealed_secret,
            seal_context: "deploy:var:svc1:DB_PASS".into(),
            is_secret: true,
        },
        DeployEnvSpec {
            key: "APP_ENV".into(),
            value_enc: sealed_plain,
            seal_context: "deploy:var:svc1:APP_ENV".into(),
            is_secret: false,
        },
    ];

    let mut session = client
        .deploy_build(
            "r3",
            "admin",
            DeployBuildRequest {
                service: "web".into(),
                deploy_id: "dep1".into(),
                source_kind: DeployGitSource::GitUrl,
                repo_url: format!("file://{}", repo_dir.display()),
                branch: "main".into(),
                commit_sha: None,
                build_method: "auto".into(),
                dockerfile_path: None,
                port: 80,
                github_token: None,
                deploy_key_enc: None,
                deploy_key_context: None,
                image_tag: "opsbox/web:dep1".into(),
                cache_from: None,
                vars: vars.clone(),
                ref_sources: vec![],
                timeout_secs: 60,
                memory_limit_mb: Some(512),
            },
        )
        .await
        .unwrap();

    let mut logs = Vec::new();
    let mut done = None;
    while let Some(frame) = session.recv().await.unwrap() {
        match frame {
            DeployBuildFrame::Log { line } => logs.push(line),
            DeployBuildFrame::Done(res) => {
                done = Some(res);
                break;
            }
            DeployBuildFrame::Error(e) => panic!("unexpected build error: {}", e.message),
        }
    }
    let done = done.unwrap();
    assert_eq!(done.commit_sha, sha);
    assert_eq!(done.commit_message, "Initial deploy commit");
    assert_eq!(done.detected_build_method, "static site");
    assert!(tmp.path().join("apps/web/src").join(&sha).is_dir());

    let joined = logs.join("\n");
    assert!(
        !joined.contains("super-secret-pw-9988"),
        "secret leaked: {joined}"
    );
    assert!(
        joined.contains("[REDACTED]"),
        "expected redacted marker: {joined}"
    );

    let docker_args = std::fs::read_to_string(&args_log).unwrap();
    assert!(docker_args.contains("APP_ENV=production"), "{docker_args}");
    assert!(
        !docker_args.contains("super-secret-pw-9988"),
        "{docker_args}"
    );
    assert_eq!(
        std::fs::read_to_string(&secret_seen).unwrap(),
        "super-secret-pw-9988"
    );
    let df = std::fs::read_to_string(&dockerfile_seen).unwrap();
    assert!(df.contains("FROM nginx:1.27-alpine"));

    // Start + healthy HTTP probe succeeds.
    let started = client
        .deploy_start_and_check(
            "r4",
            "admin",
            DeployStartRequest {
                project: "default".into(),
                service: "web".into(),
                deploy_id: "dep1".into(),
                image_ref: "opsbox/web:dep1".into(),
                port: 80,
                health_path: "/".into(),
                health_retries: 2,
                vars: vars.clone(),
                ref_sources: vec![],
                memory_limit_mb: None,
                cpu_limit_millis: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(started.container_id, "cid-1");
    assert_eq!(started.host_port, app_port);

    // Unhealthy HTTP 500 cleans up container and redacts secret from logs.
    *health_status.lock().unwrap() = 500;
    let err = client
        .deploy_start_and_check(
            "r5",
            "admin",
            DeployStartRequest {
                project: "default".into(),
                service: "web".into(),
                deploy_id: "dep2".into(),
                image_ref: "opsbox/web:dep1".into(),
                port: 80,
                health_path: "/healthz".into(),
                health_retries: 1,
                vars,
                ref_sources: vec![],
                memory_limit_mb: None,
                cpu_limit_millis: None,
            },
        )
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("HTTP 500"), "{msg}");
    assert!(
        !msg.contains("super-secret-pw-9988"),
        "secret leaked in error: {msg}"
    );
    assert!(msg.contains("[REDACTED]"), "{msg}");
    assert_eq!(removed.lock().unwrap().as_slice(), ["cid-1"]);

    let _ = tx.send(());
    let _ = tokio::time::timeout(Duration::from_secs(2), srv_task).await;
    docker_task.abort();
    app_task.abort();
}
