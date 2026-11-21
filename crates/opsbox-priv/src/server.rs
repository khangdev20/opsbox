use std::future::Future;
use std::io;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use nix::unistd::Group;
use opsbox_proto::frame::{self, FrameError};
use opsbox_proto::{
    PrivError, PrivErrorCode, PrivOp, PrivReply, PrivRequest, PrivResponse, PrivResult,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tracing::{Instrument, debug, info, info_span, warn};

use crate::authorization::{Caller, authorize};
use crate::config::PrivConfig;
use crate::io::{ReadError, read_body, write_frame};
use crate::ops;
use crate::ops::{Env, files::Dirs};
use crate::secrets::Secrets;

/// How long a connection may sit idle between frames.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Simultaneous connections served; further ones wait in the kernel backlog.
const MAX_CONNECTIONS: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("cannot create {path}: {source}")]
    Prepare { path: String, source: io::Error },
    #[error("another opsbox priv is already listening on {0}")]
    AlreadyRunning(String),
    #[error("{0} exists and is not a socket; refusing to remove it")]
    NotASocket(String),
}

pub struct PrivServer {
    listener: UnixListener,
    proxy_listener: Option<UnixListener>,
    config: PrivConfig,
}

impl PrivServer {
    /// Create the socket, set its mode and group, and start listening.
    ///
    /// A stale socket file left by a crashed process is replaced. A socket that
    /// still answers, or any non-socket file, is never touched.
    pub fn bind(config: PrivConfig) -> Result<Self, ServerError> {
        if let Some(proxy) = &config.proxy_socket
            && (proxy.path == config.socket_path || config.allowed_uids.contains(&proxy.uid))
        {
            return Err(ServerError::Prepare {
                path: proxy.path.display().to_string(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "proxy requires a separate socket and UID",
                ),
            });
        }
        let listener = Self::bind_socket(
            &config.socket_path,
            config.socket_mode,
            &config.socket_group,
        )?;
        let proxy_listener = match &config.proxy_socket {
            Some(proxy) => match Self::bind_socket(&proxy.path, 0o660, &proxy.group) {
                Ok(listener) => Some(listener),
                Err(error) => {
                    let _ = std::fs::remove_file(&config.socket_path);
                    return Err(error);
                }
            },
            None => None,
        };
        Ok(Self {
            listener,
            proxy_listener,
            config,
        })
    }

    fn bind_socket(
        path: &Path,
        mode: u32,
        group: &Option<String>,
    ) -> Result<UnixListener, ServerError> {
        let prep = |source| ServerError::Prepare {
            path: path.display().to_string(),
            source,
        };

        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).map_err(prep)?;
        }
        remove_stale_socket(path)?;

        // std first: binding is synchronous, and `from_std` needs nonblocking set.
        let std_listener = std::os::unix::net::UnixListener::bind(path).map_err(prep)?;
        std_listener.set_nonblocking(true).map_err(prep)?;

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(prep)?;
        if let Some(name) = group {
            match Group::from_name(name) {
                Ok(Some(group)) => {
                    std::os::unix::fs::chown(path, None, Some(group.gid.as_raw())).map_err(prep)?;
                }
                _ => {
                    return Err(prep(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("socket group {name} does not exist"),
                    )));
                }
            }
        }

        let listener = UnixListener::from_std(std_listener).map_err(prep)?;
        Ok(listener)
    }

    pub fn socket_path(&self) -> &Path {
        &self.config.socket_path
    }

    /// Accept connections until `shutdown` completes, then remove the socket file.
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        let allowed: Arc<[u32]> = self.config.allowed_uids.clone().into();
        let env = Arc::new(Env {
            capacity: Default::default(),
            capacity_profile: self.config.capacity_profile.clone(),
            capacity_image_size_mb: self.config.capacity_image_size_mb,
            secrets: Secrets::new(&self.config.master_key_path),
            master_key: self.config.master_key_path.clone(),
            config_path: self.config.config_path.clone(),
            files: Dirs {
                xfer: self.config.xfer_dir.clone(),
                trash: self.config.trash_dir.clone(),
            },
            apps_dir: self.config.apps_dir.clone(),
            db_path: self.config.db_path.clone(),
            backup_dir: self.config.backup_dir.clone(),
            backup_cache_dir: self.config.backup_cache_dir.clone(),
            restic_bin: self.config.restic_bin.clone(),
            docker: opsbox_docker::DockerClient::new(&self.config.docker_socket),
            docker_bin: self.config.docker_bin.clone(),
            allow_test_git: self.config.allow_test_git,
            network_proc_root: self.config.network_proc_root.clone(),
            network_systemctl: self.config.network_systemctl.clone(),
        });
        let proxy_allowed: Arc<[u32]> = self
            .config
            .proxy_socket
            .as_ref()
            .map(|proxy| vec![proxy.uid])
            .unwrap_or_default()
            .into();
        let permits = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        info!(
            socket = %self.config.socket_path.display(),
            allowed_uids = ?allowed,
            "priv listening"
        );
        // Firewall housekeeping: undo a change nobody confirmed, close emergency SSH when
        // its hour is up, and put the nftables ruleset back after a reboot.
        let janitor_env = Arc::clone(&env);
        let janitor = tokio::spawn(async move {
            let _ = tokio::task::spawn_blocking(ops::restore_on_start).await;
            let mut tick = tokio::time::interval(Duration::from_secs(10));
            let mut n = 0u64;
            loop {
                tick.tick().await;
                n += 1;
                let e = Arc::clone(&janitor_env);
                let _ = tokio::task::spawn_blocking(move || {
                    ops::janitor_tick();
                    let _ = ops::capacity::collect(&e);
                    // Once an hour: forget trash older than seven days.
                    if n % 360 == 1 {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
                        ops::files::purge_trash(&e.files.trash, now);
                    }
                })
                .await;
            }
        });
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                () = &mut shutdown => break,
                accepted = self.listener.accept() => {
                    spawn_connection(accepted, Caller::Serve, &allowed, &permits, &env);
                },
                accepted = async {
                    match &self.proxy_listener {
                        Some(listener) => listener.accept().await,
                        None => std::future::pending().await,
                    }
                } => {
                    spawn_connection(accepted, Caller::Proxy, &proxy_allowed, &permits, &env);
                },
            }
        }
        janitor.abort();
        info!("priv shutting down");
        for path in std::iter::once(&self.config.socket_path)
            .chain(self.config.proxy_socket.as_ref().map(|proxy| &proxy.path))
        {
            if let Err(e) = std::fs::remove_file(path) {
                debug!(error = %e, "could not remove socket file");
            }
        }
    }
}

fn spawn_connection(
    accepted: io::Result<(UnixStream, tokio::net::unix::SocketAddr)>,
    caller: Caller,
    allowed: &Arc<[u32]>,
    permits: &Arc<Semaphore>,
    env: &Arc<Env>,
) {
    let origin = match caller {
        Caller::Serve => "panel",
        Caller::Proxy => "proxy",
    };
    match accepted {
        Ok((stream, _)) => {
            let allowed = Arc::clone(allowed);
            let permits = Arc::clone(permits);
            let env = Arc::clone(env);
            tokio::spawn(async move {
                let Ok(_permit) = permits.acquire_owned().await else {
                    return;
                };
                handle_connection(stream, caller, &allowed, &env)
                    .instrument(info_span!("priv_connection", socket_origin = origin))
                    .await;
            });
        }
        Err(e) => warn!(error = %e, socket_origin = origin, "accept failed"),
    }
}

fn remove_stale_socket(path: &Path) -> Result<(), ServerError> {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return Ok(()); // nothing there
    };
    if !meta.file_type().is_socket() {
        return Err(ServerError::NotASocket(path.display().to_string()));
    }
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(ServerError::AlreadyRunning(path.display().to_string()));
    }
    std::fs::remove_file(path).map_err(|source| ServerError::Prepare {
        path: path.display().to_string(),
        source,
    })
}

async fn handle_connection(
    mut stream: UnixStream,
    caller: Caller,
    allowed: &[u32],
    env: &Arc<Env>,
) {
    // The kernel reports who is on the other end (SO_PEERCRED on Linux). This is
    // checked before anything the peer sent is read or parsed.
    let peer = match stream.peer_cred() {
        Ok(cred) => cred,
        Err(e) => {
            warn!(error = %e, "could not read peer credentials; closing");
            reject(&mut stream, "Could not verify who is connecting.").await;
            return;
        }
    };
    let (uid, pid) = (peer.uid(), peer.pid());
    if !allowed.contains(&uid) {
        warn!(
            uid,
            ?pid,
            "rejected connection from a user that is not allowed"
        );
        reject(&mut stream, "This user may not use the privileged helper.").await;
        return;
    }

    loop {
        let body = match tokio::time::timeout(IDLE_TIMEOUT, read_body(&mut stream)).await {
            Err(_) => {
                debug!(uid, "idle connection closed");
                return;
            }
            Ok(Ok(Some(body))) => body,
            Ok(Ok(None)) => return,
            Ok(Err(ReadError::Frame(e @ FrameError::TooLarge(_)))) => {
                // The stream cannot be resynchronised after an oversized header.
                warn!(uid, error = %e, "oversized frame");
                let _ = write_frame(
                    &mut stream,
                    &PrivResponse::failure("", PrivErrorCode::BadRequest, e.to_string()),
                )
                .await;
                return;
            }
            Ok(Err(e)) => {
                debug!(uid, error = %e, "read failed");
                return;
            }
        };

        let request = match parse(&body, uid) {
            Ok(r) => r,
            Err(response) => {
                if write_frame(&mut stream, &response).await.is_err() {
                    return;
                }
                continue;
            }
        };
        if let Err(response) = authorize(caller, &request, uid, pid) {
            if write_frame(&mut stream, &*response).await.is_err() {
                return;
            }
            continue;
        }
        if let PrivOp::PtyOpen { user, cols, rows } = &request.op {
            open_terminal(stream, &request, user, *cols, *rows, uid, pid).await;
            return;
        }
        if let PrivOp::DockerLogsFollow { id, tail } = &request.op {
            follow_docker_logs(stream, env, &request, id, *tail, uid, pid).await;
            return;
        }
        if let PrivOp::DeployBuild(req) = &request.op {
            run_deploy_build(stream, env, &request, req, uid, pid).await;
            return;
        }
        let response = process(request, uid, pid, env).await;
        if write_frame(&mut stream, &response).await.is_err() {
            return;
        }
    }
}

/// Decode one request. Anything that is not a valid [`PrivRequest`] (bad JSON, unknown
/// operation, unexpected fields) is answered with `bad_request`.
fn parse(body: &[u8], uid: u32) -> Result<PrivRequest, Box<PrivResponse>> {
    let value: serde_json::Value = match frame::decode(body) {
        Ok(v) => v,
        Err(e) => {
            return Err(Box::new(PrivResponse::failure(
                "",
                PrivErrorCode::BadRequest,
                e.to_string(),
            )));
        }
    };
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    serde_json::from_value(value).map_err(|e| {
        warn!(request_id = %id, uid, error = %e, "rejected request that is not a known operation");
        Box::new(PrivResponse::failure(
            id,
            PrivErrorCode::BadRequest,
            "Not a known privileged operation.",
        ))
    })
}

/// A terminal: answer the request, then speak the PTY stream on this connection until
/// the shell or the peer ends it.
async fn open_terminal(
    mut stream: UnixStream,
    request: &PrivRequest,
    user: &str,
    cols: u16,
    rows: u16,
    uid: u32,
    pid: Option<i32>,
) {
    let span = info_span!(
        "priv_request",
        request_id = %request.id,
        op = "pty_open",
        claimed_actor = request.actor.as_deref().unwrap_or("-"),
        peer_uid = uid,
        ?pid
    );
    async {
        let session = ops::terminal::open(user, cols, rows);
        let response = match &session {
            Ok(s) => PrivResponse::success(
                request.id.clone(),
                PrivReply::PtyStarted {
                    user: s.user.clone(),
                    shell: s.shell.clone(),
                },
            ),
            Err(e) => {
                warn!(code = ?e.code, "failed: {}", e.message);
                PrivResponse {
                    id: request.id.clone(),
                    result: PrivResult::Failure { error: e.clone() },
                }
            }
        };
        if write_frame(&mut stream, &response).await.is_err() {
            return;
        }
        if let Ok(session) = session {
            let (read, write) = stream.into_split();
            ops::terminal::run(session, read, write).await;
        }
    }
    .instrument(span)
    .await;
}

/// Follow container logs: answer with `DockerLogStreamStarted`, then stream `DockerLogFrame`s
/// on this connection until the container ends or the peer closes.
async fn follow_docker_logs(
    mut stream: UnixStream,
    env: &Arc<Env>,
    request: &PrivRequest,
    id: &str,
    tail: u32,
    uid: u32,
    pid: Option<i32>,
) {
    let span = info_span!(
        "priv_request",
        request_id = %request.id,
        op = "docker_logs_follow",
        claimed_actor = request.actor.as_deref().unwrap_or("-"),
        peer_uid = uid,
        ?pid
    );
    ops::docker::follow_logs_with(&env.docker, &mut stream, request, id, tail)
        .instrument(span)
        .await;
}

/// Stream a deploy fetch + build operation (`DeployBuildStreamStarted`, followed by `DeployBuildFrame`s).
async fn run_deploy_build(
    mut stream: UnixStream,
    env: &Arc<Env>,
    request: &PrivRequest,
    req: &opsbox_proto::DeployBuildRequest,
    uid: u32,
    pid: Option<i32>,
) {
    let span = info_span!(
        "priv_request",
        request_id = %request.id,
        op = "deploy_build",
        service = %req.service,
        deploy_id = %req.deploy_id,
        claimed_actor = request.actor.as_deref().unwrap_or("-"),
        peer_uid = uid,
        ?pid
    );
    ops::deploy::handle_deploy_build_stream(env, &mut stream, request, req)
        .instrument(span)
        .await;
}

/// Run one request.
async fn process(request: PrivRequest, uid: u32, pid: Option<i32>, env: &Arc<Env>) -> PrivResponse {
    let span = info_span!(
        "priv_request",
        request_id = %request.id,
        op = request.op.name(),
        claimed_actor = request.actor.as_deref().unwrap_or("-"),
        peer_uid = uid,
        ?pid
    );
    let op = request.op.clone();
    let env = Arc::clone(env);
    let result = async {
        let outcome = tokio::task::spawn_blocking(move || ops::dispatch(&op, &env))
            .await
            .unwrap_or_else(|_| {
                Err(PrivError {
                    code: PrivErrorCode::Internal,
                    message: "The operation crashed.".into(),
                })
            });
        match &outcome {
            Ok(_) => info!("ok"),
            Err(e) => warn!(code = ?e.code, "failed: {}", e.message),
        }
        outcome
    }
    .instrument(span)
    .await;

    let result = match result {
        Ok(reply) => PrivResult::Success { reply },
        Err(error) => PrivResult::Failure { error },
    };
    PrivResponse {
        id: request.id,
        result,
    }
}

/// Tell a disallowed peer why, then close politely.
async fn reject(stream: &mut UnixStream, message: &str) {
    let resp = PrivResponse::failure("", PrivErrorCode::Forbidden, message);
    let _ = write_frame(stream, &resp).await;
    let _ = stream.shutdown().await;
    // Drain what the peer already sent. Closing a socket with unread data makes
    // the kernel reset the connection, which would hide our reply from the peer.
    let mut sink = [0u8; 512];
    let _ = tokio::time::timeout(Duration::from_millis(250), async {
        while matches!(stream.read(&mut sink).await, Ok(n) if n > 0) {}
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn request_spans_log_peer_credentials_and_claimed_actor() {
        use std::collections::BTreeMap;
        use std::sync::Mutex;
        use tracing::{
            Event, Metadata, Subscriber,
            field::{Field, Visit},
            instrument::WithSubscriber,
            span::{Attributes, Id, Record},
        };

        type Fields = BTreeMap<String, String>;
        struct Capture(Arc<Mutex<Vec<Fields>>>);
        struct Visitor(Fields);
        impl Visit for Visitor {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                self.0.insert(field.name().into(), format!("{value:?}"));
            }
        }
        impl Subscriber for Capture {
            fn enabled(&self, _: &Metadata<'_>) -> bool {
                true
            }
            fn new_span(&self, attrs: &Attributes<'_>) -> Id {
                let mut spans = self.0.lock().unwrap();
                if attrs.metadata().name() == "priv_request" {
                    let mut visitor = Visitor(Fields::new());
                    attrs.record(&mut visitor);
                    spans.push(visitor.0);
                }
                Id::from_u64(spans.len() as u64 + 1)
            }
            fn record(&self, _: &Id, _: &Record<'_>) {}
            fn record_follows_from(&self, _: &Id, _: &Id) {}
            fn enter(&self, _: &Id) {}
            fn exit(&self, _: &Id) {}
            fn event(&self, _: &Event<'_>) {}
        }

        let dir = tempfile::tempdir().unwrap();
        let env = test_env(dir.path());
        for op in [
            serde_json::json!({"type":"ping"}),
            serde_json::json!({"type":"proxy_mode_get"}),
            serde_json::json!({"type":"proxy_mode_set","args":{"mode":"external","confirm":true}}),
            serde_json::json!({"type":"pty_open","args":{"user":"no-such-user-fix29","cols":80,"rows":24}}),
            serde_json::json!({"type":"docker_logs_follow","args":{"id":"","tail":1}}),
            serde_json::json!({"type":"deploy_build","args":{
                "service":"", "deploy_id":"test", "source_kind":"git_url", "repo_url":"invalid",
                "branch":"main", "build_method":"dockerfile", "port":8080,
                "image_tag":"test", "vars":[], "timeout_secs":1
            }}),
        ] {
            let op: PrivOp = serde_json::from_value(op).unwrap();
            // Streaming requests fail validation without opening a shell or contacting Docker/Git.
            for actor in [Some("alice"), None] {
                let (mut client, helper) = UnixStream::pair().unwrap();
                let peer = helper.peer_cred().unwrap();
                assert_eq!(peer.uid(), nix::unistd::getuid().as_raw());
                #[cfg(target_os = "linux")]
                assert_eq!(peer.pid(), Some(std::process::id() as i32));
                let captured = Arc::new(Mutex::new(Vec::new()));
                let server_env = Arc::clone(&env);
                let uid = peer.uid();
                let task = tokio::spawn(
                    async move {
                        handle_connection(helper, Caller::Serve, &[uid], &server_env).await;
                    }
                    .with_subscriber(Capture(Arc::clone(&captured))),
                );
                let request = PrivRequest {
                    id: "audit-29".into(),
                    actor: actor.map(str::to_owned),
                    op: op.clone(),
                };
                write_frame(&mut client, &request).await.unwrap();
                let body = tokio::time::timeout(Duration::from_secs(5), read_body(&mut client))
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                let response: PrivResponse = frame::decode(&body).unwrap();
                assert_eq!(response.id, request.id);
                assert_eq!(
                    matches!(response.result, PrivResult::Success { .. }),
                    matches!(op, PrivOp::Ping)
                );
                drop(client);
                task.await.unwrap();
                let spans = captured.lock().unwrap();
                assert_eq!(spans.len(), 1, "{}: {spans:?}", op.name());
                let fields = &spans[0];
                for (name, expected) in [
                    ("request_id", request.id.clone()),
                    ("op", format!("{:?}", op.name())),
                    ("peer_uid", peer.uid().to_string()),
                    ("pid", format!("{:?}", peer.pid())),
                    ("claimed_actor", format!("{:?}", actor.unwrap_or("-"))),
                ] {
                    assert_eq!(
                        fields.get(name),
                        Some(&expected),
                        "{} missing/wrong {name}: {fields:?}",
                        op.name()
                    );
                }
                assert!(!fields.contains_key("actor"), "{fields:?}");
            }
        }
    }

    fn test_env(root: &Path) -> Arc<Env> {
        let cfg = PrivConfig::new(root.join("unused.sock"), vec![]);
        Arc::new(Env {
            capacity: Default::default(),
            capacity_profile: None,
            capacity_image_size_mb: None,
            secrets: Secrets::new(root.join("master.key")),
            master_key: root.join("master.key"),
            config_path: root.join("config.toml"),
            files: Dirs {
                xfer: root.join("xfer"),
                trash: root.join("trash"),
            },
            apps_dir: root.join("apps"),
            db_path: root.join("db"),
            backup_dir: root.join("backup"),
            backup_cache_dir: root.join("cache"),
            restic_bin: None,
            docker: opsbox_docker::DockerClient::new(&cfg.docker_socket),
            docker_bin: None,
            allow_test_git: false,
            network_proc_root: root.into(),
            network_systemctl: None,
        })
    }

    // Socket pairs exercise real peer credentials and framing without requiring
    // permission to bind a filesystem socket in the development sandbox.
    #[tokio::test]
    async fn connection_policy_precedes_dispatch_and_all_streams() {
        let dir = tempfile::tempdir().unwrap();
        // Successful empty ownership data is distinct from unreadable proc tables.
        std::fs::create_dir_all(dir.path().join("net")).unwrap();
        std::fs::write(dir.path().join("net/tcp"), "").unwrap();
        std::fs::write(dir.path().join("net/tcp6"), "").unwrap();
        let env = test_env(dir.path());
        let (mut client, helper) = UnixStream::pair().unwrap();
        let uid = helper.peer_cred().unwrap().uid();
        let task = tokio::spawn(async move {
            handle_connection(helper, Caller::Proxy, &[uid], &env).await;
        });
        let written = dir.path().join("must-not-exist");
        for op in [
            serde_json::json!({"type":"ping"}),
            serde_json::json!({"type":"job_add","args":{"name":"test","cron":"invalid","command":"true"}}),
            serde_json::json!({"type":"pty_open","args":{"user":"no-such-user-fix27","cols":80,"rows":24}}),
            serde_json::json!({"type":"reboot","args":{"delay_min":999999}}),
            serde_json::json!({"type":"firewall_remove","args":{"id":"","confirm":false}}),
            serde_json::json!({"type":"file_write","args":{"path":written,"content":"forbidden"}}),
            serde_json::json!({"type":"docker_logs_follow","args":{"id":"","tail":1}}),
            serde_json::json!({"type":"deploy_build","args":{
                "service":"test", "deploy_id":"test", "source_kind":"git_url", "repo_url":"invalid",
                "branch":"main", "build_method":"dockerfile", "port":8080,
                "image_tag":"test", "vars":[], "timeout_secs":1
            }}),
            serde_json::json!({"type":"secret_open","args":{"context":"deploy:key","sealed":"invalid"}}),
        ] {
            let op: PrivOp = serde_json::from_value(op).unwrap();
            let request = PrivRequest {
                id: op.name().into(),
                actor: Some("root".into()),
                op,
            };
            write_frame(&mut client, &request).await.unwrap();
            let body = tokio::time::timeout(Duration::from_secs(5), read_body(&mut client))
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let response: PrivResponse = frame::decode(&body).unwrap();
            assert_eq!(response.id, request.id);
            assert!(
                matches!(response.result, PrivResult::Failure { error } if error.code == PrivErrorCode::Forbidden),
                "{}",
                request.op.name()
            );
        }
        assert!(!written.exists());
        for op in [
            PrivOp::PortHolders { ports: vec![] },
            PrivOp::SecretSeal {
                context: "proxy:cert:test.example.com".into(),
                plaintext: opsbox_proto::Secret::new("key"),
            },
        ] {
            let request = PrivRequest {
                id: "allowed".into(),
                actor: None,
                op,
            };
            write_frame(&mut client, &request).await.unwrap();
            let body = read_body(&mut client).await.unwrap().unwrap();
            let response: PrivResponse = frame::decode(&body).unwrap();
            assert!(
                matches!(response.result, PrivResult::Success { .. }),
                "{response:?}"
            );
        }
        drop(client);
        task.await.unwrap();
    }
}
