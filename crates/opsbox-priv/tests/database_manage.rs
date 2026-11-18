//! Privileged tests for managed PostgreSQL administration (`DatabaseInspect`, `DatabaseManage`,
//! `DatabasePooler`, `DatabaseUpgrade` validation). A scripted Docker mock records every
//! `docker exec` (argv, environment and the SQL sent on stdin) so the tests can check what
//! `priv` ran and that no secret ever travelled in argv.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nix::unistd::Uid;
use opsbox_priv::{ClientError, PrivClient, PrivConfig, PrivServer};
use opsbox_proto::{
    BackupDbKind, BackupRunSpec, BackupRunTarget, DatabaseAction, DatabaseInspectRequest,
    DatabaseManageRequest, DatabasePoolerRequest, DatabaseUpgradeRequest, DbTarget, DeployEnvSpec,
    PrivErrorCode, Secret,
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

struct ExecRecord {
    container: String,
    cmd: Vec<String>,
    env: Vec<String>,
    stdin: String,
}

#[derive(Default)]
struct Mock {
    volumes: HashSet<String>,
    networks: HashSet<String>,
    created: Vec<Value>,
    removed: Vec<String>,
    pending_exec: Option<(String, Value)>,
    execs: Vec<ExecRecord>,
    last_exit: i64,
}

const INSPECT_DOC: &str = r#"{"version":"16.4","database":"app","databases":[{"name":"app","owner":"app","size":8000000},{"name":"app_web","owner":"app_web","size":7000000}],"users":[{"name":"app","login":true,"super":true,"conns":1,"dbs":["app","app_web"]},{"name":"app_web","login":true,"super":false,"conns":2,"dbs":["app_web"]}],"grants":[{"role":"app_web","db":"app_web","connect":true,"create":true,"temp":true}],"extensions":[{"name":"pg_trgm","installed":false,"installed_version":null,"default_version":"1.6","comment":"trigrams"}],"tables":[{"schema":"public","table":"orders","bytes":16384,"rows":3}],"conns_now":3,"conns_max":100,"stat_loaded":true,"stat_enabled":false}"#;

fn spawn_mock(sock: &std::path::Path, state: Arc<Mutex<Mock>>) {
    let listener = UnixListener::bind(sock).unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let st = state.clone();
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
                            &json!({"Version":"27.3.1","ApiVersion":"1.47"}),
                        )
                        .await;
                    }
                    ("POST", "/images/create") => {
                        write_http_json(&mut stream, 200, "OK", &json!({"status":"Pulled"})).await;
                    }
                    ("GET", p) if p.starts_with("/volumes/") => {
                        let name = p.trim_start_matches("/volumes/");
                        if st.lock().unwrap().volumes.contains(name) {
                            write_http_json(&mut stream, 200, "OK", &json!({"Name":name,"Driver":"local","Mountpoint":"/x","Labels":{},"Scope":"local","Options":{}})).await;
                        } else {
                            write_http_json(
                                &mut stream,
                                404,
                                "Not Found",
                                &json!({"message":"missing"}),
                            )
                            .await;
                        }
                    }
                    ("POST", "/volumes/create") => {
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let name = v["Name"].as_str().unwrap_or("vol").to_owned();
                        st.lock().unwrap().volumes.insert(name.clone());
                        write_http_json(&mut stream, 201, "Created", &json!({"Name":name,"Driver":"local","Mountpoint":"/x","Labels":{},"Scope":"local","Options":{}})).await;
                    }
                    ("GET", p) if p.starts_with("/networks/") => {
                        let name = p.trim_start_matches("/networks/");
                        if st.lock().unwrap().networks.contains(name) {
                            write_http_json(
                                &mut stream,
                                200,
                                "OK",
                                &json!({"Name":name,"Id":format!("net-{name}")}),
                            )
                            .await;
                        } else {
                            write_http_json(
                                &mut stream,
                                404,
                                "Not Found",
                                &json!({"message":"missing"}),
                            )
                            .await;
                        }
                    }
                    ("POST", "/networks/create") => {
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        st.lock()
                            .unwrap()
                            .networks
                            .insert(v["Name"].as_str().unwrap_or("n").to_owned());
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({"Id":"net-1","Warning":""}),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/networks/") && p.ends_with("/connect") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("POST", "/containers/create") => {
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        let id = {
                            let mut g = st.lock().unwrap();
                            g.created.push(v);
                            format!("new-cid-{}", g.created.len())
                        };
                        write_http_json(
                            &mut stream,
                            201,
                            "Created",
                            &json!({"Id":id,"Warnings":[]}),
                        )
                        .await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/start") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("GET", p) if p.starts_with("/containers/") && p.ends_with("/json") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/json")
                            .to_owned();
                        write_http_json(&mut stream, 200, "OK", &json!({"Id":cid,"State":{"Running":true},"NetworkSettings":{"Ports":{},"Networks":{}}})).await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/exec") => {
                        let cid = p
                            .trim_start_matches("/containers/")
                            .trim_end_matches("/exec")
                            .to_owned();
                        let v: Value = serde_json::from_slice(&body).unwrap_or_default();
                        st.lock().unwrap().pending_exec = Some((cid, v));
                        write_http_json(&mut stream, 201, "Created", &json!({"Id":"exec-1"})).await;
                    }
                    ("POST", "/exec/exec-1/start") => {
                        let resp = "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: tcp\r\nContent-Type: application/vnd.docker.raw-stream\r\n\r\n";
                        let _ = stream.write_all(resp.as_bytes()).await;
                        // The client writes the SQL and half-closes; read it all.
                        let mut stdin = Vec::new();
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            stream.read_to_end(&mut stdin),
                        )
                        .await;
                        let stdin = String::from_utf8_lossy(&stdin).into_owned();
                        let (out, err, code) = {
                            let mut g = st.lock().unwrap();
                            let (cid, v) = g.pending_exec.take().unwrap_or_default();
                            let strs = |k: &str| -> Vec<String> {
                                v[k].as_array()
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(|x| x.as_str().map(str::to_owned))
                                            .collect()
                                    })
                                    .unwrap_or_default()
                            };
                            g.execs.push(ExecRecord {
                                container: cid,
                                cmd: strs("Cmd"),
                                env: strs("Env"),
                                stdin: stdin.clone(),
                            });
                            let (out, err, code) = if stdin.contains("json_build_object") {
                                (INSPECT_DOC.to_owned(), String::new(), 0)
                            } else if stdin.contains("FROM pg_roles WHERE rolname =")
                                && stdin.trim_start().starts_with("SELECT 1")
                            {
                                ("1\n".to_owned(), String::new(), 0)
                            } else if stdin.contains("\"vector\"") {
                                (
                                    String::new(),
                                    "ERROR:  extension \"vector\" is not available\n".to_owned(),
                                    1,
                                )
                            } else {
                                (String::new(), String::new(), 0)
                            };
                            g.last_exit = code;
                            (out, err, code)
                        };
                        let _ = code;
                        if !out.is_empty() {
                            let _ = stream.write_all(&mux_frame(1, out.as_bytes())).await;
                        }
                        if !err.is_empty() {
                            let _ = stream.write_all(&mux_frame(2, err.as_bytes())).await;
                        }
                        let _ = stream.flush().await;
                        let _ = stream.shutdown().await;
                    }
                    ("GET", "/exec/exec-1/json") => {
                        let code = st.lock().unwrap().last_exit;
                        write_http_json(
                            &mut stream,
                            200,
                            "OK",
                            &json!({"ExitCode":code,"Running":false}),
                        )
                        .await;
                    }
                    ("GET", p) if p.starts_with("/containers/") && p.ends_with("/logs") => {
                        write_http_empty(&mut stream, 200, "OK").await;
                    }
                    ("POST", p) if p.starts_with("/containers/") && p.ends_with("/stop") => {
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    ("DELETE", p) if p.starts_with("/containers/") => {
                        st.lock()
                            .unwrap()
                            .removed
                            .push(p.trim_start_matches("/containers/").to_owned());
                        write_http_empty(&mut stream, 204, "No Content").await;
                    }
                    _ => write_http_empty(&mut stream, 404, "Not Found").await,
                }
            });
        }
    });
}

struct Fixture {
    client: PrivClient,
    state: Arc<Mutex<Mock>>,
    target: DbTarget,
    stop: tokio::sync::oneshot::Sender<()>,
    _tmp: tempfile::TempDir,
}

async fn seal(client: &PrivClient, ctx: &str, value: &str) -> String {
    client
        .secret_seal("seal", None, ctx, Secret::new(value))
        .await
        .unwrap()
}

async fn fixture() -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let docker_sock = tmp.path().join("docker.sock");
    let state = Arc::new(Mutex::new(Mock::default()));
    spawn_mock(&docker_sock, state.clone());
    let priv_sock = tmp.path().join("priv.sock");
    let mut cfg = PrivConfig::new(&priv_sock, vec![Uid::current().as_raw()]);
    cfg.master_key_path = tmp.path().join("master.key");
    cfg.xfer_dir = tmp.path().join("xfer");
    cfg.trash_dir = tmp.path().join("trash");
    cfg.apps_dir = tmp.path().join("apps");
    cfg.docker_socket = docker_sock;
    let srv = PrivServer::bind(cfg).unwrap();
    let (stop, rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        srv.run(async {
            let _ = rx.await;
        })
        .await;
    });
    let client = PrivClient::new(&priv_sock);
    let mut vars = Vec::new();
    for (key, val, secret) in [
        ("POSTGRES_USER", "app", false),
        ("POSTGRES_DB", "app", false),
        ("POSTGRES_PASSWORD", "admin-top-secret-pw", true),
    ] {
        let ctx = format!("deploy:var:pg-1:{key}");
        vars.push(DeployEnvSpec {
            key: key.into(),
            value_enc: seal(&client, &ctx, val).await,
            seal_context: ctx,
            is_secret: secret,
        });
    }
    Fixture {
        client,
        state,
        target: DbTarget {
            service_id: "pg-1".into(),
            service_name: "pg-main".into(),
            template_id: "postgres".into(),
            container_id: "pg-cid".into(),
            vars,
        },
        stop,
        _tmp: tmp,
    }
}

fn app_action(rotate: bool, role: &str) -> DatabaseAction {
    let (app_service, db_name, role, var_prefix, pooler_host, pooler_port) = (
        "web".to_owned(),
        role.to_owned(),
        role.to_owned(),
        "OPSBOX_APP_WEB_".to_owned(),
        "pg-main-pooler".to_owned(),
        6432,
    );
    if rotate {
        DatabaseAction::RotateApp {
            app_service,
            db_name,
            role,
            var_prefix,
            pooler_host,
            pooler_port,
        }
    } else {
        DatabaseAction::ProvisionApp {
            app_service,
            db_name,
            role,
            var_prefix,
            pooler_host,
            pooler_port,
        }
    }
}

#[tokio::test]
async fn inspect_runs_psql_in_the_container_and_returns_typed_results() {
    let f = fixture().await;
    let res = f
        .client
        .database_inspect(
            "r1",
            "admin",
            DatabaseInspectRequest {
                target: f.target.clone(),
                database: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(res.server_version, "16.4");
    assert_eq!(res.databases.len(), 2);
    assert_eq!(res.users[1].name, "app_web");
    assert_eq!(res.tables[0].table, "orders");
    assert_eq!((res.connections_current, res.connections_max), (3, 100));
    assert!(res.stat_statements_loaded && !res.stat_statements_enabled);
    let st = f.state.lock().unwrap();
    let ex = &st.execs[0];
    assert_eq!(ex.container, "pg-cid");
    assert_eq!(ex.cmd[0], "psql");
    assert!(
        ex.env
            .contains(&"PGPASSWORD=admin-top-secret-pw".to_owned())
    );
    assert!(
        !ex.cmd.join(" ").contains("admin-top-secret-pw"),
        "secrets never in argv"
    );
    drop(st);
    let _ = f.stop.send(());
}

#[tokio::test]
async fn provision_creates_isolated_login_and_returns_sealed_connection_vars() {
    let f = fixture().await;
    let done = f
        .client
        .database_manage(
            "r2",
            "admin",
            DatabaseManageRequest {
                target: f.target.clone(),
                action: app_action(false, "app_web"),
            },
        )
        .await
        .unwrap();
    assert_eq!(done.vars.len(), 9);
    let url_spec = done
        .vars
        .iter()
        .find(|v| v.key == "OPSBOX_APP_WEB_DATABASE_URL")
        .unwrap();
    assert_eq!(
        url_spec.seal_context,
        "deploy:var:pg-1:OPSBOX_APP_WEB_DATABASE_URL"
    );
    assert!(url_spec.is_secret);
    let url = f
        .client
        .secret_open("o1", None, &url_spec.seal_context, &url_spec.value_enc)
        .await
        .unwrap();
    let url = url.expose().to_owned();
    assert!(
        url.starts_with("postgresql://app_web:") && url.ends_with("@pg-main:5432/app_web"),
        "{url}"
    );
    let pooled_spec = done
        .vars
        .iter()
        .find(|v| v.key == "OPSBOX_APP_WEB_POOLED_DATABASE_URL")
        .unwrap();
    let pooled = f
        .client
        .secret_open(
            "o2",
            None,
            &pooled_spec.seal_context,
            &pooled_spec.value_enc,
        )
        .await
        .unwrap();
    assert!(pooled.expose().ends_with("@pg-main-pooler:6432/app_web"));
    let password = url
        .split(':')
        .nth(2)
        .unwrap()
        .split('@')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(password.len(), 24);
    {
        let st = f.state.lock().unwrap();
        let ex = st.execs.last().unwrap();
        assert!(ex.stdin.contains("CREATE ROLE \"app_web\" LOGIN PASSWORD"));
        assert!(ex.stdin.contains("NOSUPERUSER NOCREATEDB NOCREATEROLE"));
        assert!(
            ex.stdin
                .contains("REVOKE ALL ON DATABASE \"app_web\" FROM PUBLIC")
        );
        assert!(
            ex.stdin
                .contains("REVOKE CONNECT ON DATABASE \"app\" FROM PUBLIC")
        );
        assert!(!ex.cmd.join(" ").contains(&password));
        assert!(!ex.stdin.contains("admin-top-secret-pw"));
        assert!(
            ex.env
                .contains(&"PGPASSWORD=admin-top-secret-pw".to_owned())
        );
    }

    // Rotation changes only the app login's password.
    let rotated = f
        .client
        .database_manage(
            "r3",
            "admin",
            DatabaseManageRequest {
                target: f.target.clone(),
                action: app_action(true, "app_web"),
            },
        )
        .await
        .unwrap();
    assert_eq!(rotated.vars.len(), 9);
    assert!(
        f.state
            .lock()
            .unwrap()
            .execs
            .last()
            .unwrap()
            .stdin
            .contains("ALTER ROLE \"app_web\" PASSWORD")
    );
    let _ = f.stop.send(());
}

#[tokio::test]
async fn app_names_and_extensions_are_validated_in_priv() {
    let f = fixture().await;
    let expect = |e: ClientError, code: PrivErrorCode| match e {
        ClientError::Remote(p) => assert_eq!(p.code, code, "{}", p.message),
        o => panic!("unexpected {o:?}"),
    };
    // Unquoted-unsafe names never reach SQL.
    let e = f
        .client
        .database_manage(
            "r4",
            "admin",
            DatabaseManageRequest {
                target: f.target.clone(),
                action: app_action(false, "web\"; DROP"),
            },
        )
        .await
        .unwrap_err();
    expect(e, PrivErrorCode::Invalid);
    // The administrator login's name is taken.
    let e = f
        .client
        .database_manage(
            "r5",
            "admin",
            DatabaseManageRequest {
                target: f.target.clone(),
                action: app_action(false, "app"),
            },
        )
        .await
        .unwrap_err();
    expect(e, PrivErrorCode::Conflict);
    // Extensions outside the allowlist are refused.
    let bad = DatabaseAction::SetExtension {
        database: "app".into(),
        extension: "plpython3u".into(),
        enable: true,
    };
    let e = f
        .client
        .database_manage(
            "r6",
            "admin",
            DatabaseManageRequest {
                target: f.target.clone(),
                action: bad,
            },
        )
        .await
        .unwrap_err();
    expect(e, PrivErrorCode::Refused);
    // A flavour without the extension gets a sentence pointing at the right template.
    let vector = DatabaseAction::SetExtension {
        database: "app".into(),
        extension: "vector".into(),
        enable: true,
    };
    let e = f
        .client
        .database_manage(
            "r7",
            "admin",
            DatabaseManageRequest {
                target: f.target.clone(),
                action: vector,
            },
        )
        .await
        .unwrap_err();
    match e {
        ClientError::Remote(p) => assert!(p.message.contains("pgvector"), "{}", p.message),
        o => panic!("{o:?}"),
    }
    // Non-PostgreSQL targets and foreign seal contexts are refused.
    let mut redis = f.target.clone();
    redis.template_id = "redis".into();
    let e = f
        .client
        .database_inspect(
            "r8",
            "admin",
            DatabaseInspectRequest {
                target: redis,
                database: None,
            },
        )
        .await
        .unwrap_err();
    expect(e, PrivErrorCode::Invalid);
    let mut foreign = f.target.clone();
    foreign.service_id = "other-1".into();
    let e = f
        .client
        .database_inspect(
            "r9",
            "admin",
            DatabaseInspectRequest {
                target: foreign,
                database: None,
            },
        )
        .await
        .unwrap_err();
    expect(e, PrivErrorCode::Refused);
    let _ = f.stop.send(());
}

#[tokio::test]
async fn pooler_starts_pgbouncer_privately_and_removes_it() {
    let f = fixture().await;
    let req = |enable: bool, existing: Option<String>| DatabasePoolerRequest {
        target: f.target.clone(),
        project: "acme".into(),
        enable,
        pool_mode: "transaction".into(),
        max_client_conn: 200,
        default_pool_size: 20,
        existing_container: existing,
        deploy_id: "pool0001".into(),
    };
    let on = f
        .client
        .database_pooler("p1", "admin", req(true, None))
        .await
        .unwrap();
    let cid = on.container_id.clone().unwrap();
    assert_eq!(on.vars.len(), 1);
    assert_eq!(on.vars[0].key, "OPSBOX_POOLER_PASSWORD");
    {
        let st = f.state.lock().unwrap();
        assert!(
            st.execs
                .last()
                .unwrap()
                .stdin
                .contains("CREATE SCHEMA IF NOT EXISTS opsbox_pooler")
        );
        let body = st.created.last().unwrap();
        assert!(
            body["Image"]
                .as_str()
                .unwrap()
                .starts_with("edoburu/pgbouncer:")
        );
        let env: Vec<&str> = body["Env"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(env.contains(&"DB_HOST=pg-main") && env.contains(&"AUTH_TYPE=scram-sha-256"));
        assert!(env.contains(&"AUTH_QUERY=SELECT uname, phash FROM opsbox_pooler.lookup($1)"));
        assert!(env.contains(&"AUTH_DBNAME=postgres") && env.contains(&"LISTEN_PORT=6432"));
        assert!(
            env.iter().all(|e| !e.contains("admin-top-secret-pw")),
            "admin password must not reach the pooler"
        );
        assert!(
            body["HostConfig"]["PortBindings"].is_null()
                || body["HostConfig"]["PortBindings"]
                    .as_object()
                    .is_none_or(|m| m.is_empty()),
            "pooler is never published on the host"
        );
    }
    let off = f
        .client
        .database_pooler("p2", "admin", req(false, Some(cid.clone())))
        .await
        .unwrap();
    assert!(off.container_id.is_none());
    let st = f.state.lock().unwrap();
    assert!(st.removed.contains(&cid));
    assert!(
        st.execs
            .last()
            .unwrap()
            .stdin
            .contains("DROP SCHEMA IF EXISTS opsbox_pooler CASCADE")
    );
    drop(st);
    let _ = f.stop.send(());
}

#[tokio::test]
async fn upgrade_is_refused_before_any_change_without_a_proper_snapshot_or_a_newer_major() {
    let f = fixture().await;
    let snapshot = |service_id: &str| BackupRunSpec {
        destination_id: "local".into(),
        repo: Default::default(),
        job_id: "job-1".into(),
        job_name: "pg".into(),
        target: BackupRunTarget::Database {
            template: BackupDbKind::Postgres,
            service_id: service_id.into(),
            service_name: "pg-main".into(),
            container_id: "pg-cid".into(),
            vars: vec![],
        },
        retain_daily: 14,
        retain_weekly: 8,
        run_prune: false,
        run_check: false,
    };
    let req = |from: &str, to: &str, snap: BackupRunSpec| DatabaseUpgradeRequest {
        target: f.target.clone(),
        project: "acme".into(),
        deploy_id: "upg0001".into(),
        from_version: from.into(),
        to_version: to.into(),
        image_ref: format!("postgres:{to}"),
        internal_only: true,
        public_port: false,
        confirm_public_port: false,
        memory_limit_mb: None,
        cpu_limit_millis: None,
        old_volume: "opsbox-pg-main-data".into(),
        new_volume: "opsbox-pg-main-data-v17".into(),
        snapshot: snap,
    };
    for (r, what) in [
        (req("16.4", "15.8", snapshot("pg-1")), "downgrade"),
        (req("16.4", "16.3", snapshot("pg-1")), "same major"),
        (
            req("16.4", "17.2", snapshot("someone-else")),
            "snapshot of another database",
        ),
        (req("16.4", "99.1", snapshot("pg-1")), "unknown version"),
    ] {
        let e = f
            .client
            .database_upgrade("u1", "admin", r)
            .await
            .unwrap_err();
        match e {
            ClientError::Remote(p) => {
                assert_eq!(p.code, PrivErrorCode::Invalid, "{what}: {}", p.message)
            }
            o => panic!("{what}: {o:?}"),
        }
    }
    // Nothing touched the database or Docker.
    let st = f.state.lock().unwrap();
    assert!(st.execs.is_empty() && st.created.is_empty() && st.removed.is_empty());
    drop(st);
    let _ = f.stop.send(());
}
