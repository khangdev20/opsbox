//! Docker Engine management & live log streaming (docs/BACKEND_SPEC.md 7.4).
//!
//! `serve` never touches `/var/run/docker.sock` or `opsbox-docker`; every query
//! and action goes to `priv` (`opsbox-priv`) as a typed `PrivOp`.

use std::collections::HashMap;
use std::time::Duration;

use axum::Json;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Path, Query, State};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use opsbox_priv::{ClientError, DockerLogSession};
use opsbox_proto::{
    DockerContainerAction, DockerContainerStat, DockerLogFrame, DockerLogLine, ErrorCode,
    PrivErrorCode, valid_docker_ref,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::session::{Admin, Client, Viewer, origin_matches_host};
use crate::system::{audit, invalid, patient};
use crate::tasks::Outcome;
use crate::{AppState, HttpError, RequestId};

const GB: f64 = (1u64 << 30) as f64;
const MB: f64 = (1u64 << 20) as f64;
const DEFAULT_LOG_TAIL: u32 = 200;
const MAX_LOG_TAIL: u32 = 2000;

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn validate_ref(id: &str) -> Result<(), HttpError> {
    if !valid_docker_ref(id) {
        return Err(invalid("id", "That is not a valid container name or ID."));
    }
    Ok(())
}

/// Map `ClientError` from Docker ops, distinguishing stopped-daemon (503) from generic errors.
fn docker_err(e: ClientError) -> HttpError {
    if let ClientError::Remote(ref r) = e
        && r.code == PrivErrorCode::Internal
        && r.message.contains("Docker daemon is not running")
    {
        return HttpError::new(ErrorCode::Unavailable, r.message.clone());
    }
    HttpError::from(e)
}

fn log_entry_json(l: &DockerLogLine) -> Value {
    json!({
        "ts": l.ts,
        "stream": l.stream,
        "text": l.text,
    })
}

/// `GET /api/v1/docker/status`
pub(crate) async fn status(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let st = state
        .priv_client
        .docker_status(&rid)
        .await
        .map_err(docker_err)?;
    Ok(Json(json!({
        "installed": st.installed,
        "running": st.running,
        "version": st.version,
        "apiVersion": st.api_version,
        "canInstall": st.can_install,
        "installMethod": st.install_method,
        "error": st.error,
    })))
}

/// `POST /api/v1/docker/install`
pub(crate) async fn install(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    let (pc, name, ip, s) = (
        patient(&state, 20 * 60),
        who.user.name.clone(),
        client.ip.clone(),
        state.clone(),
    );
    let task = state.tasks.start("docker.install", async move {
        let out = pc.docker_install(&rid, &name).await.map_err(docker_err)?;
        audit(
            &s,
            &who,
            &ip,
            "docker.install",
            None,
            json!({ "ok": out.ok }),
        );
        if out.ok {
            s.metrics.notify_docker();
            Ok(Outcome {
                log: out.tail,
                result: json!({ "ok": true }),
            })
        } else {
            Err(HttpError::new(
                ErrorCode::Internal,
                format!(
                    "Docker installation did not complete: {}",
                    out.tail.last().cloned().unwrap_or_default()
                ),
            ))
        }
    })?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "task": task })),
    ))
}

/// `GET /api/v1/docker/containers`
pub(crate) async fn list_containers(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let pc = patient(&state, 15);
    let containers = pc.docker_list_containers(&rid).await.map_err(docker_err)?;
    let stats: HashMap<String, DockerContainerStat> =
        if containers.iter().any(|c| c.state == "running") {
            pc.docker_stats(&format!("{rid}-stats"))
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|s| (s.full_id.clone(), s))
                .collect()
        } else {
            HashMap::new()
        };

    let rows: Vec<Value> = containers
        .iter()
        .map(|c| {
            let st = stats.get(&c.full_id);
            json!({
                "id": c.id,
                "fullId": c.full_id,
                "name": c.name,
                "image": c.image,
                "state": c.state,
                "status": c.status,
                "created": c.created,
                "since": c.created.saturating_mul(1000),
                "cpu": st.map_or(0.0, |s| round1(s.cpu_pct)),
                "mem": st.map_or(0.0, |s| (s.mem_used_bytes as f64 / MB).round()),
                "memUsedBytes": st.map_or(0, |s| s.mem_used_bytes),
                "memLimitBytes": st.map_or(0, |s| s.mem_limit_bytes),
                "netRxBytes": st.map_or(0, |s| s.net_rx_bytes),
                "netTxBytes": st.map_or(0, |s| s.net_tx_bytes),
                "pids": st.map_or(0, |s| s.pids),
                "ports": c.ports,
                "portList": c.port_list,
                "labels": c.labels,
                "managed": c.managed,
                "service": c.service,
                "deploy": c.deploy,
            })
        })
        .collect();
    Ok(Json(json!(rows)))
}

/// `GET /api/v1/docker/containers/{id}`
pub(crate) async fn inspect_container(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    validate_ref(&id)?;
    let info = patient(&state, 15)
        .docker_inspect(&rid, &id)
        .await
        .map_err(docker_err)?;
    Ok(Json(json!({
        "id": info.id,
        "fullId": info.full_id,
        "name": info.name,
        "image": info.image,
        "created": info.created,
        "state": info.state,
        "running": info.running,
        "restarting": info.restarting,
        "paused": info.paused,
        "oomKilled": info.oom_killed,
        "exitCode": info.exit_code,
        "startedAt": info.started_at,
        "finishedAt": info.finished_at,
        "restartCount": info.restart_count,
        "pid": info.pid,
        "cmd": info.cmd,
        "entrypoint": info.entrypoint,
        "workingDir": info.working_dir,
        "user": info.user,
        "ports": info.ports,
        "mounts": info.mounts,
        "networks": info.networks,
        "env": info.env,
        "labels": info.labels,
        "managed": info.managed,
        "service": info.service,
        "deploy": info.deploy,
    })))
}

/// `POST /api/v1/docker/containers/{id}/{action}`
pub(crate) async fn container_action(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path((id, action)): Path<(String, String)>,
) -> Result<Json<Value>, HttpError> {
    validate_ref(&id)?;
    let kind = match action.as_str() {
        "start" => DockerContainerAction::Start,
        "stop" => DockerContainerAction::Stop,
        "restart" => DockerContainerAction::Restart,
        "delete" | "remove" => DockerContainerAction::Remove,
        _ => {
            return Err(invalid(
                "action",
                "The action must be start, stop, restart, or delete.",
            ));
        }
    };
    patient(&state, 60)
        .docker_container_action(&rid, &who.user.name, &id, kind)
        .await
        .map_err(docker_err)?;
    state.metrics.notify_docker();
    audit(
        &state,
        &who,
        &client.ip,
        &format!("docker.container.{}", kind.as_str()),
        Some(&id),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct LogsQuery {
    tail: Option<u32>,
}

/// `GET /api/v1/docker/containers/{id}/logs`
pub(crate) async fn container_logs(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
) -> Result<Json<Value>, HttpError> {
    validate_ref(&id)?;
    let tail = q.tail.unwrap_or(DEFAULT_LOG_TAIL).clamp(1, MAX_LOG_TAIL);
    let lines = patient(&state, 20)
        .docker_logs(&rid, &id, tail)
        .await
        .map_err(docker_err)?;
    Ok(Json(json!(
        lines.iter().map(log_entry_json).collect::<Vec<_>>()
    )))
}

/// `GET /api/v1/stream/containers/{id}/logs`
pub(crate) async fn stream_logs(
    Viewer(who): Viewer,
    client: Client,
    State(state): State<AppState>,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    Query(q): Query<LogsQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, HttpError> {
    validate_ref(&id)?;
    if !origin_matches_host(&client.headers) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "This request came from another site and was refused.",
        ));
    }
    let tail = q.tail.unwrap_or(DEFAULT_LOG_TAIL).clamp(1, MAX_LOG_TAIL);
    let slot = crate::session::stream_slot(&state)?;
    Ok(
        crate::session::bounded_socket(ws).on_upgrade(move |socket| async move {
            let _slot = slot;
            let actor = who.user.name.clone();
            tokio::select! {
                () = crate::session::stream_ended(state.clone(), who, client.ip) => {},
                () = relay_logs(socket, state, actor, rid, id, tail) => {},
            }
        }),
    )
}

async fn relay_logs(
    mut socket: WebSocket,
    state: AppState,
    actor: String,
    rid: String,
    id: String,
    tail: u32,
) {
    let pc = state
        .priv_client
        .clone()
        .with_timeout(Duration::from_secs(10));
    let session = match pc.docker_logs_follow(&rid, Some(&actor), &id, tail).await {
        Ok(s) => s,
        Err(e) => {
            let err = docker_err(e);
            let _ = socket
                .send(Message::text(
                    json!({
                        "type": "error",
                        "code": err.0.error.code,
                        "message": err.0.error.message,
                    })
                    .to_string(),
                ))
                .await;
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: 1008,
                    reason: "docker error".into(),
                })))
                .await;
            return;
        }
    };

    let (mut tx, mut rx) = socket.split();
    let from_priv = async move {
        let mut session: DockerLogSession = session;
        loop {
            match session.recv().await {
                Ok(Some(DockerLogFrame::Line(line))) => {
                    let msg = json!({
                        "type": "logs",
                        "entries": [log_entry_json(&line)],
                    })
                    .to_string();
                    if tx.send(Message::text(msg)).await.is_err() {
                        return;
                    }
                }
                Ok(Some(DockerLogFrame::End)) | Ok(None) => {
                    let _ = tx
                        .send(Message::text(json!({ "type": "end" }).to_string()))
                        .await;
                    let _ = tx.send(Message::Close(None)).await;
                    return;
                }
                Ok(Some(DockerLogFrame::Error { message })) => {
                    let _ = tx
                        .send(Message::text(
                            json!({ "type": "error", "message": message }).to_string(),
                        ))
                        .await;
                    let _ = tx.send(Message::Close(None)).await;
                    return;
                }
                Err(e) => {
                    let err = docker_err(e);
                    let _ = tx
                        .send(Message::text(
                            json!({
                                "type": "error",
                                "code": err.0.error.code,
                                "message": err.0.error.message,
                            })
                            .to_string(),
                        ))
                        .await;
                    let _ = tx.send(Message::Close(None)).await;
                    return;
                }
            }
        }
    };
    let from_ws = async move {
        while let Some(item) = rx.next().await {
            if matches!(item, Err(_) | Ok(Message::Close(_))) {
                break;
            }
        }
    };
    tokio::select! {
        () = from_priv => {},
        () = from_ws => {},
    }
}

/// `GET /api/v1/docker/images`
pub(crate) async fn list_images(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let images = patient(&state, 15)
        .docker_list_images(&rid)
        .await
        .map_err(docker_err)?;
    Ok(Json(json!(
        images
            .iter()
            .map(|i| json!({
                "id": i.id,
                "fullId": i.full_id,
                "name": i.name,
                "tag": i.tag,
                "size": i.size_bytes as f64 / GB,
                "sizeBytes": i.size_bytes,
                "created": i.created,
                "used": i.used,
                "containers": i.containers,
            }))
            .collect::<Vec<_>>()
    )))
}

/// `GET /api/v1/docker/volumes`
pub(crate) async fn list_volumes(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let vols = patient(&state, 15)
        .docker_list_volumes(&rid)
        .await
        .map_err(docker_err)?;
    Ok(Json(json!(
        vols.iter()
            .map(|v| json!({
                "name": v.name,
                "driver": v.driver,
                "mountpoint": v.mountpoint,
                "size": v.size_bytes.map(|b| b as f64 / GB),
                "sizeBytes": v.size_bytes,
                "used": v.used,
                "createdAt": v.created_at,
                "labels": v.labels,
            }))
            .collect::<Vec<_>>()
    )))
}

/// `GET /api/v1/docker/networks`
pub(crate) async fn list_networks(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let nets = patient(&state, 15)
        .docker_list_networks(&rid)
        .await
        .map_err(docker_err)?;
    Ok(Json(json!(
        nets.iter()
            .map(|n| json!({
                "id": n.id,
                "fullId": n.full_id,
                "name": n.name,
                "driver": n.driver,
                "scope": n.scope,
                "internal": n.internal,
                "containers": n.containers,
            }))
            .collect::<Vec<_>>()
    )))
}
