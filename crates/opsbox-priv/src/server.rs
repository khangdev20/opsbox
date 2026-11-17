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
    config: PrivConfig,
}

impl PrivServer {
    /// Create the socket, set its mode and group, and start listening.
    ///
    /// A stale socket file left by a crashed process is replaced. A socket that
    /// still answers, or any non-socket file, is never touched.
    pub fn bind(config: PrivConfig) -> Result<Self, ServerError> {
        let path = config.socket_path.as_path();
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

        std::fs::set_permissions(path, std::fs::Permissions::from_mode(config.socket_mode))
            .map_err(prep)?;
        if let Some(name) = &config.socket_group {
            match Group::from_name(name) {
                Ok(Some(group)) => {
                    std::os::unix::fs::chown(path, None, Some(group.gid.as_raw())).map_err(prep)?;
                }
                _ => {
                    warn!(group = %name, "socket group does not exist; leaving the socket's group unchanged")
                }
            }
        }

        let listener = UnixListener::from_std(std_listener).map_err(prep)?;
        Ok(Self { listener, config })
    }

    pub fn socket_path(&self) -> &Path {
        &self.config.socket_path
    }

    /// Accept connections until `shutdown` completes, then remove the socket file.
    pub async fn run(self, shutdown: impl Future<Output = ()>) {
        let allowed: Arc<[u32]> = self.config.allowed_uids.clone().into();
        let env = Arc::new(Env {
            secrets: Secrets::new(&self.config.master_key_path),
            master_key: self.config.master_key_path.clone(),
            config_path: self.config.config_path.clone(),
            files: Dirs {
                xfer: self.config.xfer_dir.clone(),
                trash: self.config.trash_dir.clone(),
            },
            apps_dir: self.config.apps_dir.clone(),
            docker: opsbox_docker::DockerClient::new(&self.config.docker_socket),
            docker_bin: self.config.docker_bin.clone(),
            allow_test_git: self.config.allow_test_git,
        });
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
                accepted = self.listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        let allowed = Arc::clone(&allowed);
                        let permits = Arc::clone(&permits);
                        let env = Arc::clone(&env);
                        tokio::spawn(async move {
                            // Held for the life of the connection.
                            let Ok(_permit) = permits.acquire_owned().await else { return };
                            handle_connection(stream, &allowed, &env).await;
                        });
                    }
                    Err(e) => {
                        warn!(error = %e, "accept failed");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                },
            }
        }
        janitor.abort();
        info!("priv shutting down");
        if let Err(e) = std::fs::remove_file(&self.config.socket_path) {
            debug!(error = %e, "could not remove socket file");
        }
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

async fn handle_connection(mut stream: UnixStream, allowed: &[u32], env: &Arc<Env>) {
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
        if let PrivOp::PtyOpen { user, cols, rows } = &request.op {
            open_terminal(stream, &request, user, *cols, *rows).await;
            return;
        }
        if let PrivOp::DockerLogsFollow { id, tail } = &request.op {
            follow_docker_logs(stream, env, &request, id, *tail).await;
            return;
        }
        if let PrivOp::DeployBuild(req) = &request.op {
            run_deploy_build(stream, env, &request, req).await;
            return;
        }
        let response = process(request, uid, env).await;
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
) {
    let span = info_span!("priv_request", request_id = %request.id, op = "pty_open", actor = request.actor.as_deref().unwrap_or("-"));
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
) {
    let span = info_span!(
        "priv_request",
        request_id = %request.id,
        op = "docker_logs_follow",
        actor = request.actor.as_deref().unwrap_or("-")
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
) {
    let span = info_span!(
        "priv_request",
        request_id = %request.id,
        op = "deploy_build",
        service = %req.service,
        deploy_id = %req.deploy_id,
        actor = request.actor.as_deref().unwrap_or("-")
    );
    ops::deploy::handle_deploy_build_stream(env, &mut stream, request, req)
        .instrument(span)
        .await;
}

/// Run one request.
async fn process(request: PrivRequest, uid: u32, env: &Arc<Env>) -> PrivResponse {
    let span = info_span!(
        "priv_request",
        request_id = %request.id,
        op = request.op.name(),
        actor = request.actor.as_deref().unwrap_or("-"),
        uid
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
