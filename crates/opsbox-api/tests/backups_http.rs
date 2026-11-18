//! End-to-end HTTP integration tests for Prompt 13 (backups engine):
//! - real `PrivServer` + real `Store` + mock Docker engine + stateful mock `restic`
//! - `GET /api/v1/backups` + real `"same disk"` detection
//! - real template PostgreSQL creation (`POST /api/v1/templates/postgres/instances`) +
//!   `pg_dump -Fc` logical dump streamed via `docker exec` directly to `restic --stdin`
//!   (`PGPASSWORD` inside exec env only, never argv or error output)
//! - Docker volume mountpoint backup (with container pause/unpause)
//! - filesystem path backup + strict refusal of `master.key`, `..`, and remote destinations
//! - `opsbox` self-backup using SQLite online backup API + `/etc/opsbox` minus `master.key`
//! - retention prune (`forget --prune`), checksum check (`check --read-data-subset`), and
//!   failed backup / failed checksum surfacing in `GET /api/v1/attention`
//! - RBAC & token scope enforcement.

use std::collections::HashMap;
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
use opsbox_store::{ReadOnlyStore, SessionKind, Store, TokenScope};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
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

fn docker_mux_frame(stream_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.push(stream_type);
    out.extend_from_slice(&[0, 0, 0]);
    let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    out
}

#[derive(Default)]
struct MockDockerState {
    volumes: HashMap<String, String>,
    networks: std::collections::HashSet<String>,
    containers: HashMap<String, Vec<String>>,
    paused_calls: Vec<String>,
    unpaused_calls: Vec<String>,
    removed_containers: Vec<String>,
    removed_volumes: Vec<String>,
    exec_envs: Vec<Vec<String>>,
    restored_dumps: Vec<Vec<u8>>,
    fail_exec: bool,
    next_cid: u64,
    exec_seq: u64,
    exec_outcomes: HashMap<String, (u8, i64)>,
}

async fn send_with_headers(
    app: &Router,
    method: Method,
    path: &str,
    auth: &str,
    is_bearer: bool,
    csrf: &str,
    body: Option<Value>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let mut builder = Request::builder()
        .method(method.clone())
        .uri(path)
        .header(header::HOST, HOST);
    if is_bearer {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {auth}"));
    } else {
        builder = builder.header(header::COOKIE, auth);
        if !matches!(method, Method::GET | Method::HEAD) {
            builder = builder
                .header(header::ORIGIN, ORIGIN)
                .header("x-opsbox-csrf", csrf);
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
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
    (status, headers, val)
}

async fn send(
    app: &Router,
    method: Method,
    path: &str,
    auth: &str,
    is_bearer: bool,
    csrf: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (st, _, val) = send_with_headers(app, method, path, auth, is_bearer, csrf, body).await;
    (st, val)
}

async fn wait_task_done(app: &Router, cookie: &str, csrf: &str, task_id: &str) -> Value {
    for _ in 0..100 {
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
        assert_eq!(st, StatusCode::OK, "task poll failed: {val}");
        if val["status"].as_str() != Some("running") {
            return val;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("task {task_id} did not finish in time");
}

fn write_stateful_mock_restic(path: &std::path::Path) {
    let script = r#"#!/bin/sh
set -eu
repo=""
pw=""
sub=""
stdin_mode=0
stdin_name=""
restore_target=""
tags=""
keep_daily=""
targets=""
expect=""
for a in "$@"; do
  case "$expect" in
    repo) repo="$a"; expect=""; continue ;;
    pw) pw="$a"; expect=""; continue ;;
    stdin_name) stdin_name="$a"; expect=""; continue ;;
    target) restore_target="$a"; expect=""; continue ;;
    tag) tags="${tags}${tags:+|}$a"; expect=""; continue ;;
    opt|exclude|group-by|keep-weekly|read-data-subset) expect=""; continue ;;
    keep-daily) keep_daily="$a"; expect=""; continue ;;
  esac
  case "$a" in
    -r) expect="repo" ;;
    --password-file) expect="pw" ;;
    --stdin-filename) expect="stdin_name" ;;
    --target) expect="target" ;;
    -o) expect="opt" ;;
    --tag) expect="tag" ;;
    --exclude|--group-by|--keep-weekly) expect="$a"; expect="${expect#--}" ;;
    --keep-daily) expect="keep-daily" ;;
    --read-data-subset=*) ;;
    --stdin) stdin_mode=1 ;;
    --json|--prune) ;;
    version) sub="version" ;;
    init) sub="init" ;;
    cat) sub="cat" ;;
    backup) sub="backup" ;;
    snapshots) sub="snapshots" ;;
    forget) sub="forget" ;;
    check) sub="check" ;;
    dump) sub="dump" ;;
    restore) sub="restore" ;;
    *) [ -n "$sub" ] && targets="${targets}${targets:+|}$a" ;;
  esac
done

if [ "$sub" = "version" ]; then
  echo "restic 0.19.1 compiled with go1.24.2 on linux/amd64"
  exit 0
fi

[ -n "$pw" ] && [ -f "$pw" ] || { echo "missing password file" >&2; exit 2; }
base_dir="$(dirname "$0")/mock-repos"
case "$repo" in
  s3:*)
    if [ -z "${AWS_ACCESS_KEY_ID:-}" ] || [ -z "${AWS_SECRET_ACCESS_KEY:-}" ]; then
      echo "missing S3 credentials in environment" >&2
      exit 3
    fi
    repo_fs="$base_dir/s3-$(printf '%s' "$repo" | tr -c 'A-Za-z0-9._-' '_')"
    ;;
  sftp:*)
    repo_fs="$base_dir/sftp-$(printf '%s' "$repo" | tr -c 'A-Za-z0-9._-' '_')"
    ;;
  *)
    repo_fs="$repo"
    ;;
esac
db_list="$repo_fs/snapshots.tsv"

case "$sub" in
  cat)
    [ -f "$repo_fs/config" ] || { echo "Fatal: unable to open config file: does not exist" >&2; exit 10; }
    cat "$repo_fs/config"
    ;;
  init)
    mkdir -p "$repo_fs"
    echo "{}" > "$repo_fs/config"
    : > "$db_list"
    echo "0" > "$repo_fs/seq"
    echo "{\"message_type\":\"initialized\",\"id\":\"testrepo\",\"repository\":\"$repo\"}"
    ;;
  backup)
    [ -f "$repo_fs/config" ] || { echo "repo not initialized" >&2; exit 1; }
    seq=$(cat "$repo_fs/seq" 2>/dev/null || echo "0")
    seq=$((seq + 1))
    echo "$seq" > "$repo_fs/seq"
    sid=$(printf "%08x%056x" "$seq" "$seq")
    short=$(printf "%08x" "$seq")
    if [ "$stdin_mode" -eq 1 ]; then
      dump_out="$repo_fs/stdin_${short}_${stdin_name}"
      cat > "$dump_out"
      cp "$dump_out" "$repo_fs/last_stdin_${stdin_name}"
      bytes=$(wc -c < "$dump_out" | tr -d ' ')
      paths_str="/$stdin_name"
    else
      bytes=4096
      paths_str="$targets"
      if [ -f "opsbox.db" ] && [ -d "etc-opsbox" ]; then
        cp "opsbox.db" "$repo_fs/self_${short}_opsbox.db"
        if find "etc-opsbox" -name "master.key" | grep -q .; then
          echo "FORBIDDEN master.key staged" >&2
          exit 9
        fi
        cp -R "etc-opsbox" "$repo_fs/self_${short}_etc"
      else
        snap_dir="$repo_fs/snaps/${sid}"
        mkdir -p "$snap_dir"
        OLD_IFS="$IFS"
        IFS="|"
        for src in $targets; do
          case "$src" in
            /*)
              if [ -e "$src" ]; then
                mkdir -p "$snap_dir$(dirname "$src")"
                cp -R "$src" "$snap_dir$src"
              fi
              ;;
          esac
        done
        IFS="$OLD_IFS"
      fi
    fi
    printf "%s\t%s\t%s\t%s\t%s\n" "$sid" "$short" "$bytes" "$tags" "$paths_str" >> "$db_list"
    echo "{\"message_type\":\"summary\",\"snapshot_id\":\"$sid\",\"total_bytes_processed\":$bytes,\"data_added\":$bytes,\"total_files_processed\":2}"
    ;;
  snapshots)
    [ -f "$db_list" ] || { echo "[]"; exit 0; }
    printf "["
    first=1
    while IFS="$(printf '\t')" read -r sid short bytes tags_raw paths_raw; do
      [ -z "$sid" ] && continue
      [ "$first" -eq 1 ] || printf ","
      first=0
      tags_json=$(printf "%s" "$tags_raw" | tr '|' '\n' | awk 'NF{printf "%s\"%s\"", (i++?",":""), $0}')
      paths_json=$(printf "%s" "$paths_raw" | tr '|' '\n' | awk 'NF{printf "%s\"%s\"", (i++?",":""), $0}')
      printf "{\"id\":\"%s\",\"short_id\":\"%s\",\"time\":\"2026-11-18T02:00:05Z\",\"hostname\":\"vm\",\"paths\":[%s],\"tags\":[%s],\"summary\":{\"total_bytes_processed\":%s}}" "$sid" "$short" "$paths_json" "$tags_json" "$bytes"
    done < "$db_list"
    printf "]\n"
    ;;
  forget)
    [ -f "$db_list" ] || { echo "[]"; exit 0; }
    if [ -z "$keep_daily" ] && [ -n "$targets" ]; then
      grep -v "^${targets}" "$db_list" > "$repo_fs/kept.tmp" || true
      mv "$repo_fs/kept.tmp" "$db_list"
      echo "[{\"keep\":[],\"remove\":[{\"id\":\"$targets\",\"short_id\":\"${targets%????????????????????????????????????????????????????????}\"}]}]"
      exit 0
    fi
    keep="${keep_daily:-1}"
    [ -n "$keep" ] || keep=1
    filter_tag="$tags"
    kept_file="$repo_fs/kept.tmp"
    removed_file="$repo_fs/rem.tmp"
    : > "$kept_file"
    : > "$removed_file"
    total_match=$(awk -F '\t' -v t="$filter_tag" 'BEGIN{c=0} (t=="" || index($4, t)>0){c++} END{print c}' "$db_list")
    drop_n=$((total_match - keep))
    [ "$drop_n" -lt 0 ] && drop_n=0
    awk -F '\t' -v t="$filter_tag" -v drop="$drop_n" -v kf="$kept_file" -v rf="$removed_file" '
      BEGIN{seen=0}
      {
        if (t=="" || index($4, t)>0) {
          seen++
          if (seen <= drop) { print $0 >> rf } else { print $0 >> kf }
        } else {
          print $0 >> kf
        }
      }' "$db_list"
    mv "$kept_file" "$db_list"
    rem_json=$(awk -F '\t' '{printf "%s{\"id\":\"%s\",\"short_id\":\"%s\"}", (NR>1?",":""), $1, $2}' "$removed_file")
    keep_json=$(awk -F '\t' -v t="$filter_tag" 'BEGIN{i=0} (t=="" || index($4, t)>0){printf "%s{\"id\":\"%s\",\"short_id\":\"%s\"}", (i++?",":""), $1, $2}' "$db_list")
    rm -f "$removed_file"
    echo "[{\"keep\":[$keep_json],\"remove\":[$rem_json]}]"
    ;;
  check)
    if [ -f "$repo_fs/fail_check" ]; then
      echo "{\"message_type\":\"summary\",\"num_errors\":1,\"broken_packs\":[\"packdeadbeef\"]}"
      exit 1
    fi
    echo "{\"message_type\":\"summary\",\"num_errors\":0,\"broken_packs\":[]}"
    ;;
  dump)
    if [ -f "$repo_fs/fail_restore_test" ]; then
      echo "corrupted pack header" >&2
      exit 1
    fi
    snap_id="${targets%%|*}"
    dump_path="${targets#*|}"
    base_name="${dump_path#/}"
    short=$(printf "%s" "$snap_id" | cut -c1-8)
    cand="$repo_fs/stdin_${short}_${base_name}"
    [ -f "$cand" ] || cand="$repo_fs/last_stdin_${base_name}"
    if [ -f "$cand" ]; then
      cat "$cand"
    else
      echo "snapshot file not found: $cand" >&2
      exit 1
    fi
    ;;
  restore)
    if [ -f "$repo_fs/fail_restore_test" ]; then
      echo "corrupted snapshot tree" >&2
      exit 1
    fi
    mkdir -p "$restore_target"
    snap_id="${targets%%|*}"
    short=$(printf "%s" "$snap_id" | cut -c1-8)
    if [ -d "$repo_fs/snaps/${snap_id}" ]; then
      cp -R "$repo_fs/snaps/${snap_id}/." "$restore_target/"
    elif [ -f "$repo_fs/self_${short}_opsbox.db" ]; then
      cp "$repo_fs/self_${short}_opsbox.db" "$restore_target/opsbox.db"
      if [ -d "$repo_fs/self_${short}_etc" ]; then
        cp -R "$repo_fs/self_${short}_etc" "$restore_target/etc-opsbox"
      fi
    fi
    ;;
esac
"#;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn backups_end_to_end_db_stream_volume_path_self_prune_check_and_guards() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let etc_dir = root.join("etc-opsbox");
    let var_dir = root.join("var-lib");
    let uploads_dir = root.join("uploads");
    let vol_dir = root.join("vol_redis_data");
    let apps_dir = var_dir.join("apps");
    let backup_dir = root.join("backups");
    let cache_dir = root.join("cache");
    std::fs::create_dir_all(&etc_dir).unwrap();
    std::fs::create_dir_all(&apps_dir).unwrap();
    std::fs::create_dir_all(&uploads_dir).unwrap();
    std::fs::create_dir_all(&vol_dir).unwrap();
    std::fs::write(uploads_dir.join("hello.txt"), b"hello uploads").unwrap();
    std::fs::write(vol_dir.join("dump.rdb"), b"REDIS0011").unwrap();

    let master_key = etc_dir.join("master.key");
    let config_path = etc_dir.join("config.toml");
    std::fs::write(&config_path, "listen = \"127.0.0.1:8443\"\n").unwrap();
    let db_path = var_dir.join("opsbox.db");

    // Mock Docker Unix socket.
    let docker_sock = root.join("docker.sock");
    let docker_state = Arc::new(Mutex::new(MockDockerState::default()));
    {
        let mut guard = docker_state.lock().unwrap();
        guard
            .volumes
            .insert("opsbox-cache-data".into(), vol_dir.display().to_string());
        guard
            .containers
            .insert("cid-cache".into(), vec!["opsbox-cache-data".into()]);
    }
    let d_listener = UnixListener::bind(&docker_sock).unwrap();
    let d_state = Arc::clone(&docker_state);
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = d_listener.accept().await {
            let st = Arc::clone(&d_state);
            tokio::spawn(async move {
                let Some((method, target, body)) = read_http_req(&mut stream).await else {
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
                    ("POST", "/images/create") => {
                        write_http_json(&mut stream, 200, "OK", &json!({ "status": "Pulled" }))
                            .await;
                    }
                    ("GET", p) if p.starts_with("/volumes/") => {
                        let name = p.trim_start_matches("/volumes/");
                        let mp = st.lock().unwrap().volumes.get(name).cloned();
                        if let Some(mp) = mp {
                            write_http_json(
                                &mut stream,
                                200,
                                "OK",
                                &json!({
                                    "Name": name,
                                    "Driver": "local",
                                    "Mountpoint": mp,
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
                                &json!({ "message": "missing volume" }),
                            )
                            .await;
                        }
                    }
                    ("DELETE", p) if p.starts_with("/volumes/") => {
                        let name = p
                            .trim_start_matches("/volumes/")
                            .split('?')
                            .next()
                            .unwrap_or("")
                            .to_owned();
                        {
                            let mut guard = st.lock().unwrap();
                            guard.volumes.remove(&name);
                            guard.removed_volumes.push(name);
                        }
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", "/volumes/create") => {
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let name = v["Name"].as_str().unwrap_or("vol").to_owned();
                        let mp = format!("/tmp/{name}");
                        st.lock().unwrap().volumes.insert(name.clone(), mp.clone());
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({
                                "Name": name,
                                "Driver": "local",
                                "Mountpoint": mp,
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
                                &json!({ "message": "missing network" }),
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
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", "/containers/create") => {
                        let cid = {
                            let mut guard = st.lock().unwrap();
                            guard.next_cid += 1;
                            let id = format!("cid-pg-{}", guard.next_cid);
                            guard.containers.insert(id.clone(), Vec::new());
                            id
                        };
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({ "Id": cid, "Warnings": [] }),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/start") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", "/containers/json") => {
                        let items: Vec<Value> = st
                            .lock()
                            .unwrap()
                            .containers
                            .iter()
                            .map(|(cid, vols)| {
                                let mounts: Vec<Value> = vols
                                    .iter()
                                    .map(|v| {
                                        json!({
                                            "Type": "volume",
                                            "Name": v,
                                            "Source": "/tmp",
                                            "Destination": "/data"
                                        })
                                    })
                                    .collect();
                                json!({
                                    "Id": cid,
                                    "Names": [format!("/{cid}")],
                                    "Image": "postgres:16.4",
                                    "State": "running",
                                    "Status": "Up 1m",
                                    "Mounts": mounts,
                                    "Labels": {}
                                })
                            })
                            .collect();
                        write_http_json(&mut stream, 200, "OK", &json!(items)).await;
                    }
                    ("GET", p) if p.starts_with("/containers/") && p.ends_with("/json") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/json");
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({
                                "Id": cid,
                                "State": { "Running": true },
                                "NetworkSettings": { "Ports": {}, "Networks": {} }
                            }),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/pause") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/pause")
                            .to_owned();
                        st.lock().unwrap().paused_calls.push(cid);
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/unpause") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/unpause")
                            .to_owned();
                        st.lock().unwrap().unpaused_calls.push(cid);
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/stop") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", p) if p.starts_with("/containers/") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .split('?')
                            .next()
                            .unwrap_or("")
                            .to_owned();
                        {
                            let mut guard = st.lock().unwrap();
                            guard.containers.remove(&cid);
                            guard.removed_containers.push(cid);
                        }
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/exec") => {
                        let req_json: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let env_list: Vec<String> = req_json["Env"]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default();
                        let first_cmd = req_json["Cmd"]
                            .as_array()
                            .and_then(|a| a.first())
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let kind = match first_cmd {
                            "pg_dump" => 1u8,
                            "pg_restore" => 2u8,
                            _ => 0u8,
                        };
                        let exec_id = {
                            let mut guard = st.lock().unwrap();
                            guard.exec_seq += 1;
                            let id = format!("exec-{}", guard.exec_seq);
                            if kind == 1 {
                                guard.exec_envs.push(env_list);
                            }
                            let code = if kind == 1 && guard.fail_exec { 1 } else { 0 };
                            guard.exec_outcomes.insert(id.clone(), (kind, code));
                            id
                        };
                        write_http_json(&mut stream, 201, "Created", &json!({ "Id": exec_id }))
                            .await;
                    }
                    ("POST", p) if p.starts_with("/exec/") && p.ends_with("/start") => {
                        let eid = p.trim_start_matches("/exec/").trim_end_matches("/start");
                        let (kind, code) = st
                            .lock()
                            .unwrap()
                            .exec_outcomes
                            .get(eid)
                            .copied()
                            .unwrap_or((0, 0));
                        let resp = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\nContent-Type: application/vnd.docker.raw-stream\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        if kind == 1 {
                            if code == 0 {
                                let _ = stream
                                    .write_all(&docker_mux_frame(1, b"PGDMP\x01\x0ehello-pg-dump"))
                                    .await;
                            } else {
                                let _ = stream
                                    .write_all(&docker_mux_frame(
                                        2,
                                        b"pg_dump: connection refused\n",
                                    ))
                                    .await;
                            }
                        } else if kind == 2 {
                            let mut stdin_buf = Vec::new();
                            let _ = stream.read_to_end(&mut stdin_buf).await;
                            st.lock().unwrap().restored_dumps.push(stdin_buf);
                            let _ = stream
                                .write_all(&docker_mux_frame(1, b"pg_restore ok\n"))
                                .await;
                        } else {
                            let _ = stream
                                .write_all(&docker_mux_frame(1, b"accepting connections\n"))
                                .await;
                        }
                        let _ = stream.flush().await;
                        let _ = stream.shutdown().await;
                    }
                    ("GET", p) if p.starts_with("/exec/") && p.ends_with("/json") => {
                        let eid = p.trim_start_matches("/exec/").trim_end_matches("/json");
                        let (_, code) = st
                            .lock()
                            .unwrap()
                            .exec_outcomes
                            .get(eid)
                            .copied()
                            .unwrap_or((0, 0));
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({ "ExitCode": code, "Running": false }),
                        )
                        .await;
                    }
                    _ => write_http_empty(&mut stream, 404, "Not Found").await,
                }
            });
        }
    });

    let fake_restic = root.join("restic");
    write_stateful_mock_restic(&fake_restic);

    let priv_sock = root.join("priv.sock");
    let uid_probe = root.join("uid.probe");
    std::fs::write(&uid_probe, b"").unwrap();
    let uid = std::fs::metadata(&uid_probe).unwrap().uid();
    let mut cfg = PrivConfig::new(&priv_sock, vec![uid]);
    cfg.master_key_path = master_key.clone();
    cfg.config_path = config_path.clone();
    cfg.db_path = db_path.clone();
    cfg.backup_dir = backup_dir.clone();
    cfg.backup_cache_dir = cache_dir.clone();
    cfg.xfer_dir = var_dir.join("xfer");
    cfg.trash_dir = var_dir.join("trash");
    cfg.apps_dir = apps_dir;
    cfg.docker_socket = docker_sock;
    cfg.restic_bin = Some(fake_restic);
    let srv = PrivServer::bind(cfg).unwrap();
    let (priv_stop, priv_rx) = oneshot::channel::<()>();
    let priv_task = tokio::spawn(async move {
        srv.run(async {
            let _ = priv_rx.await;
        })
        .await;
    });

    let store = Arc::new(Store::open(&db_path).unwrap());
    store
        .set_setting("live_backup_probe", &json!("from_live_sqlite"))
        .unwrap();

    let auth_cfg = AuthConfig {
        hash_params: HashParams {
            memory_kib: 1024,
            iterations: 1,
            parallelism: 1,
        },
        ..AuthConfig::default()
    };
    let admin_hash = opsbox_auth::password::Hasher::new(auth_cfg.hash_params)
        .hash("TestAdminPass123!")
        .unwrap();
    let admin = store
        .create_user("alice", Role::Admin, &admin_hash, 0)
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
    let tok_a = "sess-alice-backups-123";
    let tok_v = "sess-bob-backups-123";
    for (uid_str, tok) in [(&admin.id, tok_a), (&viewer.id, tok_v)] {
        store
            .create_session(
                uid_str,
                &opsbox_auth::tokens::hash_token(tok),
                SessionKind::Web,
                None,
                None,
                now,
            )
            .unwrap();
    }
    let csrf_a = opsbox_auth::tokens::csrf_token(tok_a);
    let csrf_v = opsbox_auth::tokens::csrf_token(tok_v);
    let cookie_a = format!("opsbox_session={tok_a}; opsbox_csrf={csrf_a}");
    let cookie_v = format!("opsbox_session={tok_v}; opsbox_csrf={csrf_v}");

    let state = AppState::new(
        store.clone(),
        PrivClient::new(&priv_sock).with_timeout(Duration::from_secs(15)),
        "0.1.0-test",
        SecureCookies::Never,
        auth_cfg,
    )
    .with_backup_dir(&backup_dir)
    .with_deploy(DeploySettings {
        drain_secs: 0,
        build_timeout_secs: 60,
        health_retries: 2,
        allow_test_git: true,
    });
    let (_tok_rec, dep_token) = state
        .auth
        .create_token(&deployer, "ci-deploy", TokenScope::Deploy, None)
        .unwrap();
    let (_admin_tok_rec, admin_token) = state
        .auth
        .create_token(&admin, "backup-admin-tok", TokenScope::Full, None)
        .unwrap();
    let app = router(state);

    // 1. RBAC checks: Deployer API token denied on /backups; Viewer can GET /backups, denied mutations.
    let (st, _) = send(
        &app,
        Method::GET,
        "/api/v1/backups",
        &dep_token,
        true,
        "",
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, ov) = send(
        &app,
        Method::GET,
        "/api/v1/backups",
        &cookie_v,
        false,
        &csrf_v,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "viewer should read backups: {ov}");
    assert_eq!(ov["sameDisk"], true);
    assert_eq!(ov["localDisk"]["resticInstalled"], true);
    assert_eq!(ov["localDisk"]["resticVersion"], "0.19.1");
    assert_eq!(ov["remoteDestinationsReady"], true);
    assert_eq!(ov["restoreReady"], true);
    assert!(
        ov["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|j| j["id"] == "opsbox-self")
    );

    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/backups/run",
        &cookie_v,
        false,
        &csrf_v,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 2. Refusals: master.key path, ../ traversal, bad cron, remote destination, deleting opsbox-self.
    for bad in [
        json!({ "name": "steal-key", "kind": "path", "paths": [master_key.display().to_string()], "cron": "0 2 * * *" }),
        json!({ "name": "dotdot", "kind": "path", "paths": ["/tmp/../etc"], "cron": "0 2 * * *" }),
        json!({ "name": "bad-cron", "kind": "path", "paths": [uploads_dir.display().to_string()], "cron": "not a cron" }),
    ] {
        let (st, _) = send(
            &app,
            Method::POST,
            "/api/v1/backups/jobs",
            &cookie_a,
            false,
            &csrf_a,
            Some(bad),
        )
        .await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }
    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/backups/jobs",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({
            "name": "remote",
            "kind": "path",
            "paths": [uploads_dir.display().to_string()],
            "cron": "0 2 * * *",
            "destinationId": "s3-prod"
        })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    let (st, _) = send(
        &app,
        Method::DELETE,
        "/api/v1/backups/jobs/opsbox-self",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // 3. Instantiate real PostgreSQL template service (creates sealed PGPASSWORD + container + backup_jobs row).
    let (st, pg_res) = send(
        &app,
        Method::POST,
        "/api/v1/templates/postgres/instances",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "name": "pg-main", "version": "16.4", "disk": "5 GB", "backup": true })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "pg create failed: {pg_res}");
    let pg_create_task =
        wait_task_done(&app, &cookie_a, &csrf_a, pg_res["task"].as_str().unwrap()).await;
    assert_eq!(
        pg_create_task["status"], "ok",
        "pg create task failed: {pg_create_task}"
    );

    let pg_svc = store.get_service("pg-main").unwrap().unwrap();
    let pg_job = store
        .get_backup_job_for_service(&pg_svc.id)
        .unwrap()
        .unwrap();

    // 4. Trigger PostgreSQL logical dump -> streams directly to restic stdin without argv secret.
    let (st, run_pg) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/jobs/{}/run", pg_job.id),
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({})),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED, "run pg job failed: {run_pg}");
    let pg_task = wait_task_done(&app, &cookie_a, &csrf_a, run_pg["task"].as_str().unwrap()).await;
    assert_eq!(pg_task["status"], "ok", "pg backup task failed: {pg_task}");

    let stdin_capture = backup_dir.join("stdin_00000001_pg-main.pg_dump.fc");
    assert_eq!(
        std::fs::read(&stdin_capture).unwrap(),
        b"PGDMP\x01\x0ehello-pg-dump"
    );
    {
        let guard = docker_state.lock().unwrap();
        assert!(
            guard.exec_envs[0]
                .iter()
                .any(|e| e.starts_with("PGPASSWORD=") && e.len() > "PGPASSWORD=".len()),
            "PGPASSWORD must pass inside container exec env: {:?}",
            guard.exec_envs
        );
    }

    // 5. Create Path + Volume schedules, run all enabled jobs (including opsbox-self).
    let (st, path_created) = send(
        &app,
        Method::POST,
        "/api/v1/backups/jobs",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({
            "name": "uploads",
            "kind": "path",
            "paths": [uploads_dir.display().to_string()],
            "cron": "0 3 * * *",
            "retainDaily": 1,
            "retainWeekly": 0
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "create path job: {path_created}");
    let path_job_id = path_created["job"]["id"].as_str().unwrap().to_owned();

    let (st, vol_created) = send(
        &app,
        Method::POST,
        "/api/v1/backups/jobs",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({
            "name": "redis-vol",
            "kind": "volume",
            "volumes": ["opsbox-cache-data"],
            "pauseContainers": true,
            "cron": "0 4 * * *",
            "retainDaily": 7,
            "retainWeekly": 4
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "create vol job: {vol_created}");

    let (st, run_all_res) = send(
        &app,
        Method::POST,
        "/api/v1/backups/run",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let all_task = wait_task_done(
        &app,
        &cookie_a,
        &csrf_a,
        run_all_res["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(all_task["status"], "ok", "run_all task failed: {all_task}");

    // Verify container pause/unpause ran during volume backup!
    {
        let guard = docker_state.lock().unwrap();
        assert_eq!(guard.paused_calls, vec!["cid-cache"]);
        assert_eq!(guard.unpaused_calls, vec!["cid-cache"]);
    }

    // Verify opsbox-self SQLite online hot backup + config without master.key!
    let self_snap = store
        .get_backup_job("opsbox-self")
        .unwrap()
        .unwrap()
        .last_snapshot_id
        .unwrap();
    let short_self = &self_snap[..8];
    let captured_db = backup_dir.join(format!("self_{short_self}_opsbox.db"));
    let ro = ReadOnlyStore::open(&captured_db).unwrap();
    assert_eq!(
        ro.get_setting("live_backup_probe").unwrap(),
        Some(json!("from_live_sqlite"))
    );
    assert!(
        backup_dir
            .join(format!("self_{short_self}_etc/config.toml"))
            .is_file()
    );
    assert!(
        !backup_dir
            .join(format!("self_{short_self}_etc/master.key"))
            .exists()
    );

    // 6. Run uploads a second time, then prune (retainDaily=1) removes the older uploads snapshot!
    let (st, r2) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/jobs/{path_job_id}/run"),
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let t2 = wait_task_done(&app, &cookie_a, &csrf_a, r2["task"].as_str().unwrap()).await;
    assert_eq!(t2["status"], "ok");

    let (_, snaps_before) = send(
        &app,
        Method::GET,
        "/api/v1/backups/snapshots",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    let uploads_before = snaps_before["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["jobId"] == path_job_id)
        .count();
    assert_eq!(uploads_before, 2);

    let (st, prune_res) = send(
        &app,
        Method::POST,
        "/api/v1/backups/prune",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "jobId": path_job_id })),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let prune_t = wait_task_done(
        &app,
        &cookie_a,
        &csrf_a,
        prune_res["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(prune_t["status"], "ok");
    assert_eq!(prune_t["result"]["removed"], 1);

    let (_, snaps_after) = send(
        &app,
        Method::GET,
        "/api/v1/backups/snapshots",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    let uploads_after = snaps_after["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["jobId"] == path_job_id)
        .count();
    assert_eq!(uploads_after, 1);

    // 7. Checksum check pass -> checksum_ok; then simulated failure -> checksum_failed + Attention!
    let (st, chk_res) = send(
        &app,
        Method::POST,
        "/api/v1/backups/check",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "readDataSubset": "10%" })),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let chk_t = wait_task_done(&app, &cookie_a, &csrf_a, chk_res["task"].as_str().unwrap()).await;
    assert_eq!(chk_t["status"], "ok");

    let (_, ov2) = send(
        &app,
        Method::GET,
        "/api/v1/backups",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    let ov2_snaps = ov2["snapshots"].as_array().unwrap();
    assert!(
        !ov2_snaps.is_empty(),
        "expected non-empty snapshots in ov2: {ov2}"
    );
    assert!(ov2_snaps.iter().all(|s| s["check"] == "checksum_ok"));

    std::fs::write(backup_dir.join("fail_check"), b"1").unwrap();
    let (st, chk_fail_res) = send(
        &app,
        Method::POST,
        "/api/v1/backups/check",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let chk_fail_t = wait_task_done(
        &app,
        &cookie_a,
        &csrf_a,
        chk_fail_res["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(chk_fail_t["status"], "failed");

    // 8. Also trigger failed PG dump and verify failed backup + checksum failure both alert!
    let pg_vars = store.list_service_vars(&pg_svc.id).unwrap();
    let _ = pg_vars;
    docker_state.lock().unwrap().fail_exec = true;
    let (st, pg_fail_res) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/jobs/{}/run", pg_job.id),
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let pg_fail_t = wait_task_done(
        &app,
        &cookie_a,
        &csrf_a,
        pg_fail_res["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(pg_fail_t["status"], "failed");

    let (st, att) = send(
        &app,
        Method::GET,
        "/api/v1/attention",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let att_items = att.as_array().unwrap();
    assert!(
        att_items.iter().any(|i| {
            i["kind"] == "backup"
                && i["text"]
                    .as_str()
                    .unwrap()
                    .contains("Backup failed for pg-main")
        }),
        "missing failed backup alert: {att}"
    );
    assert!(
        att_items.iter().any(|i| {
            i["kind"] == "backup"
                && i["text"]
                    .as_str()
                    .unwrap()
                    .contains("checksum verification failed")
        }),
        "missing checksum failure alert: {att}"
    );

    // 9. PATCH & DELETE path schedule cleanly.
    let (st, upd) = send(
        &app,
        Method::PATCH,
        &format!("/api/v1/backups/jobs/{path_job_id}"),
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "enabled": false, "cron": "@hourly" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(upd["job"]["enabled"], false);
    assert_eq!(upd["job"]["cron"], "@hourly");

    let (st, del) = send(
        &app,
        Method::DELETE,
        &format!("/api/v1/backups/jobs/{path_job_id}"),
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(del["ok"], true);

    // 10. Prompt 14: providers preset API, offsite R2 + SFTP destinations, sealed creds,
    //     recovery key behind password re-auth + deny_token, restore copy/replace + DB reconnect,
    //     and restore verification.
    docker_state.lock().unwrap().fail_exec = false;
    let _ = std::fs::remove_file(backup_dir.join("fail_check"));

    let (st, provs) = send(
        &app,
        Method::GET,
        "/api/v1/backups/providers",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(provs["providers"].as_array().unwrap().len(), 10);

    // Viewer cannot test or save destinations.
    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/backups/destinations/test",
        &cookie_v,
        false,
        &csrf_v,
        Some(json!({ "provider": "r2", "values": { "account": "acct1", "bucket": "ops", "key": "ak", "secret": "sk" } })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // Pre-save destination test returns 3 passing steps.
    let (st, test_r2) = send(
        &app,
        Method::POST,
        "/api/v1/backups/destinations/test",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({
            "provider": "r2",
            "values": {
                "account": "acct999",
                "bucket": "opsbox-bk",
                "prefix": "prod",
                "key": "AKIA_PRE_TEST",
                "secret": "SK_PRE_SECRET"
            }
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "R2 test failed: {test_r2}");
    assert_eq!(test_r2["ok"], true);
    assert_eq!(test_r2["steps"].as_array().unwrap().len(), 3);

    // Save R2 destination -> secrets sealed (never plaintext), switches existing 'local' jobs.
    let (st, save_r2) = send(
        &app,
        Method::POST,
        "/api/v1/backups/destinations",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({
            "provider": "r2",
            "name": "Offsite R2",
            "values": {
                "account": "acct999",
                "bucket": "opsbox-bk",
                "prefix": "prod",
                "key": "AKIA_SAVED",
                "secret": "SK_SAVED_SECRET"
            }
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "R2 save failed: {save_r2}");
    let r2_id = save_r2["destination"]["id"].as_str().unwrap().to_owned();
    let r2_row = store.get_backup_destination(&r2_id).unwrap().unwrap();
    let enc = r2_row.secrets_enc.as_deref().unwrap();
    assert!(enc.starts_with("enc:v1:"));
    assert!(!enc.contains("SK_SAVED_SECRET"));
    assert_eq!(
        store
            .get_backup_job(&pg_job.id)
            .unwrap()
            .unwrap()
            .destination_id,
        r2_id
    );

    // Save SFTP destination -> generates Ed25519 keypair, exposes public key only, unverified until tested.
    let (st, save_sftp) = send(
        &app,
        Method::POST,
        "/api/v1/backups/destinations",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({
            "provider": "sftp",
            "name": "Vault SFTP",
            "values": {
                "host": "127.0.0.1",
                "port": "2222",
                "user": "opsbox",
                "path": "/srv/backups"
            }
        })),
    )
    .await;
    assert_eq!(st, StatusCode::CREATED, "SFTP save failed: {save_sftp}");
    assert!(
        save_sftp["publicKey"]
            .as_str()
            .unwrap()
            .starts_with("ssh-ed25519 ")
    );
    let sftp_id = save_sftp["destination"]["id"].as_str().unwrap().to_owned();
    let sftp_row = store.get_backup_destination(&sftp_id).unwrap().unwrap();
    assert!(sftp_row.verified_at.is_none());
    assert!(
        sftp_row
            .secrets_enc
            .as_deref()
            .unwrap()
            .starts_with("enc:v1:")
    );

    // Recovery key: token refused (403), wrong password refused (403, not 401), valid password succeeds.
    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/backups/recovery-key",
        &admin_token,
        true,
        "",
        Some(json!({ "password": "TestAdminPass123!" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, _) = send(
        &app,
        Method::POST,
        "/api/v1/backups/recovery-key",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "password": "WrongPassword999!" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, hdrs, rec_res) = send_with_headers(
        &app,
        Method::POST,
        "/api/v1/backups/recovery-key",
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "password": "TestAdminPass123!" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(
        hdrs.get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let expected_pw = opsbox_backup::derive_repo_password(
        &std::fs::read(&master_key).unwrap().try_into().unwrap(),
    );
    assert_eq!(rec_res["password"].as_str().unwrap(), expected_pw.as_str());

    // Run pg-main backup onto R2 offsite destination.
    let (st, run_r2_pg) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/jobs/{}/run", pg_job.id),
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let run_r2_t = wait_task_done(
        &app,
        &cookie_a,
        &csrf_a,
        run_r2_pg["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(run_r2_t["status"], "ok", "R2 backup failed: {run_r2_t}");

    // Locate the R2 pg-main snapshot in overview.
    let (st, ov2) = send(
        &app,
        Method::GET,
        "/api/v1/backups",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(ov2["remoteDestinationsReady"], true);
    assert_eq!(ov2["restoreReady"], true);
    let r2_pg_snap = ov2["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["destinationId"] == r2_id && s["kind"] == "database")
        .expect("missing R2 pg snapshot");
    let r2_pg_snap_id = r2_pg_snap["id"].as_str().unwrap().to_owned();

    // Restore DB snapshot as COPY -> starts new managed DB service & streams pg_restore dump.
    let (st, rst_copy) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/snapshots/{r2_pg_snap_id}/restore"),
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "mode": "copy", "copyTarget": "pg-restored" })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::ACCEPTED,
        "DB copy restore failed: {rst_copy}"
    );
    let rst_copy_t =
        wait_task_done(&app, &cookie_a, &csrf_a, rst_copy["task"].as_str().unwrap()).await;
    assert_eq!(
        rst_copy_t["status"], "ok",
        "DB copy task failed: {rst_copy_t}"
    );
    assert_eq!(rst_copy_t["result"]["targetSummary"], "pg-restored");
    assert!(!docker_state.lock().unwrap().restored_dumps.is_empty());

    // Restored DB copy is a full managed DB service that an app can connect to!
    let (proj, env_rec) = store.ensure_default_project(now).unwrap();
    let web_app = store
        .create_service(opsbox_store::NewService {
            project_id: &proj.id,
            env_id: &env_rec.id,
            name: "shop-web",
            kind: opsbox_store::ServiceKind::App,
            owner_id: Some(&admin.id),
            source_kind: opsbox_store::SourceKind::DockerImage,
            repo: "nginx:alpine",
            branch: "latest",
            build_method: "image",
            dockerfile_path: None,
            start_cmd: None,
            port: 80,
            health_path: "/",
            domain: None,
            image_ref: Some("nginx:alpine"),
            deploy_key_id: None,
            auto_deploy: false,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 0,
            graph_y: 0,
            note: None,
            now,
        })
        .unwrap();
    let (st, conn_res) = send(
        &app,
        Method::POST,
        &format!("/api/v1/services/{}/databases", web_app.id),
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "database": "pg-restored" })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "connecting app to restored DB copy failed: {conn_res}"
    );

    // Replace-in-place WITHOUT exact "replace" confirmation must be refused with 409 confirmation_required.
    let (st, err_confirm) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/snapshots/{r2_pg_snap_id}/restore"),
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "mode": "replace", "confirm": "wrong" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
    assert_eq!(err_confirm["error"]["code"], "confirmation_required");

    // Replace-in-place WITH exact "replace" takes a live safety snapshot FIRST, then restores.
    let (st, rst_rep) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/snapshots/{r2_pg_snap_id}/restore"),
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "mode": "replace", "confirm": "replace" })),
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let rst_rep_t =
        wait_task_done(&app, &cookie_a, &csrf_a, rst_rep["task"].as_str().unwrap()).await;
    assert_eq!(
        rst_rep_t["status"], "ok",
        "DB replace task failed: {rst_rep_t}"
    );
    assert!(rst_rep_t["result"]["safetySnapshotId"].as_str().is_some());

    // Opsbox-self snapshot refuses replace-in-place even with "replace", allows copy restore + restore test.
    let self_snap_id = ov2["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["kind"] == "opsbox")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (st, _) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/snapshots/{self_snap_id}/restore"),
        &cookie_a,
        false,
        &csrf_a,
        Some(json!({ "mode": "replace", "confirm": "replace" })),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    let (st, rtest_res) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/snapshots/{r2_pg_snap_id}/test"),
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let rtest_t = wait_task_done(
        &app,
        &cookie_a,
        &csrf_a,
        rtest_res["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(rtest_t["status"], "ok", "restore test failed: {rtest_t}");
    assert_eq!(rtest_t["result"]["checkState"], "restore_tested");
    assert_eq!(
        store
            .get_backup_snapshot(&r2_pg_snap_id)
            .unwrap()
            .unwrap()
            .check_state,
        opsbox_proto::BackupCheckState::RestoreTested
    );

    // Deleting built-in local destination is refused; deleting R2 moves pg-main back to 'local'.
    let (st, _) = send(
        &app,
        Method::DELETE,
        "/api/v1/backups/destinations/local",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    let (st, del_r2) = send(
        &app,
        Method::DELETE,
        &format!("/api/v1/backups/destinations/{r2_id}"),
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(del_r2["ok"], true);
    assert_eq!(
        store
            .get_backup_job(&pg_job.id)
            .unwrap()
            .unwrap()
            .destination_id,
        "local"
    );

    // Failed restore test surfaces alert via GET /api/v1/attention.
    std::fs::write(backup_dir.join("fail_restore_test"), b"1").unwrap();
    let (st, rt_fail_res) = send(
        &app,
        Method::POST,
        &format!("/api/v1/backups/snapshots/{self_snap_id}/test"),
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::ACCEPTED);
    let rt_fail_t = wait_task_done(
        &app,
        &cookie_a,
        &csrf_a,
        rt_fail_res["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(rt_fail_t["status"], "failed");
    let (st, att2) = send(
        &app,
        Method::GET,
        "/api/v1/attention",
        &cookie_a,
        false,
        &csrf_a,
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        att2.as_array().unwrap().iter().any(|i| {
            i["kind"] == "backup"
                && i["text"]
                    .as_str()
                    .unwrap()
                    .contains("Monthly backup restore test failed")
        }),
        "missing restore-test failure alert: {att2}"
    );

    let _ = priv_stop.send(());
    let _ = priv_task.await;
}
