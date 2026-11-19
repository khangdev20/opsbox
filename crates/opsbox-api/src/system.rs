//! The System page: updates, reboot, services, journal, jobs, firewall, swap, host name
//! and time zone (docs/BACKEND_SPEC.md 7.8). Everything is a typed operation in `priv`;
//! this module authorizes, shapes JSON for the UI and writes the audit entries.
//!
//! Reading needs a signed-in account; every change needs an admin.

use std::net::IpAddr;
use std::time::Duration;

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Path, Query, State};
use axum::response::Response;
use opsbox_auth::Principal;
use opsbox_priv::PrivClient;
use opsbox_proto::{ErrorCode, NewRule, PkgSelect, PkgState, UnitActionKind};
use opsbox_store::{ActorKind, AuditResult};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::session::{Admin, Client, Viewer, origin_matches_host};
use crate::tasks::Outcome;
use crate::{AppState, HttpError, RequestId};

const MB: f64 = (1u64 << 20) as f64;

pub(crate) fn invalid(field: &'static str, msg: &str) -> HttpError {
    HttpError(
        opsbox_proto::ApiError::new(ErrorCode::Validation, msg).with_field(field),
        None,
    )
}

/// A client for operations that can take minutes (installing packages, creating swap).
pub(crate) fn patient(state: &AppState, secs: u64) -> PrivClient {
    state
        .priv_client
        .clone()
        .with_timeout(Duration::from_secs(secs))
}

pub(crate) fn audit(
    state: &AppState,
    who: &Principal,
    ip: &str,
    action: &str,
    target: Option<&str>,
    detail: Value,
) {
    let actor = who.user.actor_label();
    state.auth.audit(
        &actor,
        if who.via_token {
            ActorKind::Token
        } else {
            ActorKind::User
        },
        Some(ip),
        action,
        target,
        AuditResult::Ok,
        detail,
    );
}

/// The address to protect from a firewall lockout. Behind an SSH tunnel or a local proxy
/// every request comes from loopback, which says nothing about where the administrator's
/// SSH session comes from, so loopback counts as "unknown" (the cautious answer).
fn caller_ip(ip: &str) -> Option<String> {
    ip.parse::<IpAddr>()
        .ok()
        .filter(|a| !a.is_loopback() && !a.is_unspecified())
        .map(|a| a.to_string())
}

// ------------------------------------------------------------------ updates

/// Shape of `getUpdates()` in the mock.
pub fn updates_view(p: &PkgState) -> Value {
    let list: Vec<Value> = p
        .updates
        .iter()
        .map(|u| json!({ "pkg": u.name, "cur": u.current, "next": u.available, "sec": u.security, "kernel": u.kernel }))
        .collect();
    json!({
        "list": list,
        "total": p.updates.len(),
        "security": p.updates.iter().filter(|u| u.security).count(),
        "reboot": p.reboot_required,
        "rebootAt": p.reboot_at,
        "auto": p.auto_security,
        "running": p.running_kernel,
        "checkedAt": p.index_time,
        "manager": p.manager,
    })
}

pub(crate) async fn get_updates(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let p = patient(&state, 180).pkg_list(&rid).await?;
    Ok(Json(updates_view(&p)))
}

#[derive(Deserialize)]
pub(crate) struct InstallBody {
    which: String,
    #[serde(default)]
    packages: Vec<String>,
}

pub(crate) async fn install_updates(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<InstallBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    let which = match body.which.as_str() {
        "security" => PkgSelect::Security,
        "all" => PkgSelect::All,
        "selected" if !body.packages.is_empty() && body.packages.len() <= 200 => PkgSelect::All,
        _ => return Err(invalid("which", "Choose security or all.")),
    };
    let (pc, name, ip, s) = (
        patient(&state, 40 * 60),
        who.user.name.clone(),
        client.ip.clone(),
        state.clone(),
    );
    let task = state.tasks.start("updates.install", async move {
        let out = if body.which == "selected" {
            pc.pkg_install_selected(&rid, &name, &body.packages).await?
        } else {
            pc.pkg_install(&rid, &name, which).await?
        };
        audit(
            &s,
            &who,
            &ip,
            "system.updates.install",
            Some(&body.which),
            json!({ "packages": out.count, "ok": out.ok }),
        );
        if out.ok {
            Ok(Outcome {
                log: out.tail,
                result: json!({ "installed": out.count }),
            })
        } else {
            Err(HttpError::new(
                ErrorCode::Internal,
                format!(
                    "The update did not complete: {}",
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

#[derive(Deserialize)]
pub(crate) struct OnBody {
    pub(crate) on: bool,
}

pub(crate) async fn set_auto_updates(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<OnBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    let pc = patient(&state, 700);
    let s = state.clone();
    let task = state.tasks.start("updates.auto", async move {
        pc.auto_updates_set(&rid, &who.user.name, body.on).await?;
        audit(
            &s,
            &who,
            &client.ip,
            "system.updates.auto",
            None,
            json!({ "on": body.on }),
        );
        Ok(Outcome {
            log: vec![],
            result: json!({ "on": body.on }),
        })
    })?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "task": task })),
    ))
}

pub(crate) async fn get_task(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let task = state.tasks.get(&id).ok_or_else(HttpError::not_found)?;
    Ok(Json(serde_json::to_value(task).unwrap_or_default()))
}

// ------------------------------------------------------------------- reboot

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct RebootBody {
    delay_min: Option<u32>,
}

pub(crate) async fn reboot(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<RebootBody>>,
) -> Result<Json<Value>, HttpError> {
    let delay = body.and_then(|b| b.0.delay_min).unwrap_or(1);
    state
        .priv_client
        .reboot(&rid, &who.user.name, delay)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.reboot",
        None,
        json!({ "delayMin": delay }),
    );
    Ok(Json(json!({ "ok": true, "delayMin": delay })))
}

pub(crate) async fn reboot_cancel(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    state
        .priv_client
        .reboot_cancel(&rid, &who.user.name)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.reboot.cancel",
        None,
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

// ----------------------------------------------------------------- services

pub(crate) async fn get_units(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let units = state.priv_client.unit_list(&rid).await?;
    Ok(Json(json!(units
        .iter()
        .map(|u| json!({ "name": u.name, "state": u.state, "boot": u.boot, "description": u.description, "mem": u.mem_bytes.map_or(0.0, |b| (b as f64 / MB).round()) }))
        .collect::<Vec<_>>())))
}

pub(crate) async fn unit_action(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path((name, action)): Path<(String, String)>,
) -> Result<Json<Value>, HttpError> {
    let kind = match action.as_str() {
        "start" => UnitActionKind::Start,
        "stop" => UnitActionKind::Stop,
        "restart" => UnitActionKind::Restart,
        "enable" => UnitActionKind::Enable,
        "disable" => UnitActionKind::Disable,
        _ => {
            return Err(invalid(
                "action",
                "The action must be start, stop, restart, enable or disable.",
            ));
        }
    };
    patient(&state, 120)
        .unit_action(&rid, &who.user.name, &name, kind)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        &format!("system.unit.{action}"),
        Some(&name),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

// ------------------------------------------------------------------ journal

#[derive(Deserialize)]
pub(crate) struct JournalQuery {
    unit: Option<String>,
    prio: Option<u8>,
    q: Option<String>,
    limit: Option<u32>,
}

fn entry_json(e: &opsbox_proto::JournalEntry) -> Value {
    json!({ "t": e.t, "proc": e.proc, "prio": e.prio, "msg": e.msg, "cursor": e.cursor })
}

pub(crate) async fn get_journal(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(q): Query<JournalQuery>,
) -> Result<Json<Value>, HttpError> {
    let mut entries = patient(&state, 30)
        .journal(
            &rid,
            q.unit.as_deref(),
            q.prio.unwrap_or(6),
            q.q.as_deref(),
            q.limit.unwrap_or(200),
            None,
        )
        .await?;
    // priv returns newest first; the log reads oldest first.
    entries.reverse();
    Ok(Json(json!(
        entries.iter().map(entry_json).collect::<Vec<_>>()
    )))
}

/// `WS /api/v1/stream/journal?unit=&prio=&q=`: the last lines at once, then new ones as
/// they appear, `{ "type": "journal", "entries": [...] }`.
pub(crate) async fn stream_journal(
    Viewer(who): Viewer,
    client: Client,
    State(state): State<AppState>,
    Query(q): Query<JournalQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, HttpError> {
    if !origin_matches_host(&client.headers) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "This request came from another site and was refused.",
        ));
    }
    let slot = crate::session::stream_slot(&state)?;
    Ok(
        crate::session::bounded_socket(ws).on_upgrade(move |socket| async move {
            let _slot = slot;
            tokio::select! {
                () = crate::session::stream_ended(state.clone(), who, client.ip) => {},
                () = follow_journal(socket, state.clone(), q) => {},
            }
        }),
    )
}

async fn follow_journal(mut socket: WebSocket, state: AppState, q: JournalQuery) {
    let pc = patient(&state, 30);
    let (unit, prio, text) = (q.unit, q.prio.unwrap_or(6), q.q);
    let send = |entries: &[opsbox_proto::JournalEntry]| {
        Message::text(json!({ "type": "journal", "entries": entries.iter().map(entry_json).collect::<Vec<_>>() }).to_string())
    };

    let mut cursor = match pc
        .journal(
            "journal-open",
            unit.as_deref(),
            prio,
            text.as_deref(),
            100,
            None,
        )
        .await
    {
        Ok(mut first) => {
            first.reverse();
            let cur = first.last().map(|e| e.cursor.clone());
            if socket.send(send(&first)).await.is_err() {
                return;
            }
            cur
        }
        Err(e) => {
            let _ = socket
                .send(Message::text(
                    json!({ "type": "error", "message": e.to_string() }).to_string(),
                ))
                .await;
            return;
        }
    };
    loop {
        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {
                let Some(c) = cursor.clone() else {
                    // The journal was empty at the start: look again from scratch.
                    if let Ok(mut all) = pc.journal("journal-poll", unit.as_deref(), prio, text.as_deref(), 100, None).await {
                        all.reverse();
                        cursor = all.last().map(|e| e.cursor.clone());
                        if !all.is_empty() && socket.send(send(&all)).await.is_err() { return; }
                    }
                    continue;
                };
                match pc.journal("journal-poll", unit.as_deref(), prio, text.as_deref(), 500, Some(&c)).await {
                    Ok(new) if !new.is_empty() => {
                        cursor = new.last().map(|e| e.cursor.clone());
                        if socket.send(send(&new)).await.is_err() { return; }
                    }
                    _ => {}
                }
            }
            incoming = socket.recv() => match incoming {
                None | Some(Err(_) | Ok(Message::Close(_))) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

// --------------------------------------------------------------------- jobs

fn job_json(j: &opsbox_proto::JobInfo) -> Value {
    json!({ "id": j.id, "cron": j.cron, "name": j.name, "command": j.command, "lastRun": j.last_run, "ok": j.last_ok })
}

pub(crate) async fn get_jobs(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let jobs = patient(&state, 60).job_list(&rid).await?;
    Ok(Json(json!(jobs.iter().map(job_json).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
pub(crate) struct JobBody {
    name: String,
    cron: String,
    command: String,
}

pub(crate) async fn add_job(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<JobBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    let id = state
        .priv_client
        .job_add(&rid, &who.user.name, &b.name, &b.cron, &b.command)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.job.add",
        Some(&id),
        json!({ "name": b.name, "cron": b.cron }),
    );
    Ok((axum::http::StatusCode::CREATED, Json(json!({ "id": id }))))
}

pub(crate) async fn run_job(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    if id.len() != 8 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("id", "That is not a job id."));
    }
    let pc = patient(&state, 16 * 60);
    let s = state.clone();
    let task = state.tasks.start(&format!("job.run:{id}"), async move {
        pc.job_run(&rid, &who.user.name, &id).await?;
        audit(&s, &who, &client.ip, "system.job.run", Some(&id), json!({}));
        Ok(Outcome {
            log: vec![],
            result: json!({ "ok": true }),
        })
    })?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "task": task })),
    ))
}

pub(crate) async fn remove_job(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    state
        .priv_client
        .job_remove(&rid, &who.user.name, &id)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.job.remove",
        Some(&id),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

// ----------------------------------------------------------------- firewall

pub(crate) async fn get_firewall(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let mut f = patient(&state, 60).firewall_list(&rid).await?;
    if !f.protected_ports.contains(&state.listen.port()) {
        f.protected_ports.push(state.listen.port());
        f.protected_ports.sort_unstable();
    }
    Ok(Json(json!({
        "backend": f.backend,
        "rules": f.rules.iter().map(|r| json!({ "id": r.id, "port": r.port.to_string(), "proto": r.proto, "src": r.src, "note": r.note })).collect::<Vec<_>>(),
        "sshOpen": f.ssh_open_until.is_some(),
        "sshUntil": f.ssh_open_until,
        "revertAt": f.revert_at,
        "protectedPorts": f.protected_ports,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuleBody {
    port: Value,
    #[serde(default = "tcp")]
    proto: String,
    #[serde(default = "anywhere")]
    src: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    confirm: bool,
}

fn tcp() -> String {
    "tcp".into()
}
fn anywhere() -> String {
    "anywhere".into()
}

/// The UI sends the port as typed (a string); accept numbers too.
fn parse_port(v: &Value) -> Result<u16, HttpError> {
    let n = match v {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse::<u64>().ok(),
        _ => None,
    };
    n.filter(|n| (1..=65535).contains(n))
        .map(|n| n as u16)
        .ok_or_else(|| invalid("port", "Port must be between 1 and 65535."))
}

pub(crate) async fn add_rule(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<RuleBody>,
) -> Result<Json<Value>, HttpError> {
    let rule = NewRule {
        port: parse_port(&b.port)?,
        proto: b.proto,
        src: if b.src.trim().is_empty() {
            "anywhere".into()
        } else {
            b.src.trim().to_owned()
        },
        note: b.note.trim().to_owned(),
    };
    let label = format!("{}/{} from {}", rule.port, rule.proto, rule.src);
    let change = patient(&state, 90)
        .firewall_add(
            &rid,
            &who.user.name,
            rule,
            caller_ip(&client.ip).as_deref(),
            b.confirm,
        )
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.firewall.add",
        Some(&label),
        json!({ "revertAt": change.revert_at }),
    );
    Ok(Json(json!({ "ok": true, "revertAt": change.revert_at })))
}

#[derive(Deserialize)]
pub(crate) struct ConfirmQuery {
    #[serde(default)]
    confirm: bool,
}

pub(crate) async fn remove_rule(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    Query(q): Query<ConfirmQuery>,
) -> Result<Json<Value>, HttpError> {
    let change = patient(&state, 90)
        .firewall_remove(
            &rid,
            &who.user.name,
            &id,
            caller_ip(&client.ip).as_deref(),
            q.confirm,
            state.listen.port(),
        )
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.firewall.remove",
        Some(&id),
        json!({ "revertAt": change.revert_at }),
    );
    Ok(Json(json!({ "ok": true, "revertAt": change.revert_at })))
}

pub(crate) async fn confirm_firewall(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    state
        .priv_client
        .firewall_confirm(&rid, &who.user.name)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.firewall.confirm",
        None,
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct SshBody {
    ip: Option<String>,
}

/// `POST /api/v1/ssh/emergency`: open SSH to one address for an hour. Through a tunnel the
/// panel only sees loopback, so the person gives the address they connect from.
pub(crate) async fn ssh_emergency(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<SshBody>>,
) -> Result<Json<Value>, HttpError> {
    let asked = body
        .and_then(|b| b.0.ip)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let ip = match asked {
        Some(a) => a.parse::<IpAddr>().map(|a| a.to_string()).map_err(|_| invalid("ip", "That is not an IP address."))?,
        None => caller_ip(&client.ip).ok_or_else(|| invalid("ip", "opsbox only sees this machine's own address here (you are probably using an SSH tunnel). Enter the public address you connect to SSH from."))?,
    };
    let until = state
        .priv_client
        .clone()
        .with_timeout(Duration::from_secs(60))
        .ssh_emergency(&rid, &who.user.name, &ip)
        .await?;
    let actor = who.user.actor_label();
    state.auth.audit(
        &actor,
        ActorKind::User,
        Some(&client.ip),
        "system.ssh.emergency",
        Some(&ip),
        AuditResult::Ok,
        json!({ "until": until }),
    );
    Ok(Json(json!({ "until": until * 1000, "ip": ip })))
}

// ------------------------------------------------------------ swap, host, zone

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SwapBody {
    size_mb: u64,
}

pub(crate) async fn set_swap(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<SwapBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    let (pc, name, ip, s) = (
        patient(&state, 20 * 60),
        who.user.name.clone(),
        client.ip.clone(),
        state.clone(),
    );
    let task = state.tasks.start("system.swap", async move {
        pc.swap_set(&rid, &name, b.size_mb).await?;
        audit(
            &s,
            &who,
            &ip,
            "system.swap",
            None,
            json!({ "sizeMb": b.size_mb }),
        );
        Ok(Outcome {
            log: vec![],
            result: json!({ "sizeMb": b.size_mb }),
        })
    })?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "task": task })),
    ))
}

// --------------------------------------------------------------------- disk

/// `GET /api/v1/system/disk`: for `/` and the opsbox data and Docker directories (when they
/// are on other filesystems), what is mounted, how big it is and how much disk is unused.
pub(crate) async fn get_disk(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let d = patient(&state, 60).disk_state(&rid).await?;
    Ok(Json(serde_json::to_value(d).unwrap_or_default()))
}

#[derive(Deserialize)]
pub(crate) struct GrowDiskBody {
    mount: String,
    /// The word `grow`, typed by the person.
    confirm: String,
    /// The account password again (owner re-authentication).
    password: String,
    /// The person agreed to install `growpart` when it is missing.
    #[serde(default)]
    install_tools: bool,
}

fn disk_sizes(v: &opsbox_proto::DiskVolume) -> Value {
    json!({
        "fs_bytes": v.fs_bytes,
        "partition_bytes": v.partition_bytes,
        "disk_bytes": v.disk_bytes,
        "growable_bytes": v.growable_bytes,
    })
}

const DISK_TASK: &str = "system.disk_grow";

/// `POST /api/v1/system/disk/grow`: grow the filesystem mounted at `mount` into the space the
/// enlarged disk gave it. An interactive admin session, the password again and the typed
/// word `grow` are all required. Each step is its own typed operation in `priv`.
pub(crate) async fn grow_disk(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<GrowDiskBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    crate::session::deny_token(&who)?;
    let ok = state
        .auth
        .verify_user_password(&who.user.id, &b.password)
        .await
        .map_err(|_| HttpError::internal())?;
    if !ok {
        let actor = who.user.actor_label();
        state.auth.audit(
            &actor,
            ActorKind::User,
            Some(&client.ip),
            "system.disk.grow",
            Some(&b.mount),
            AuditResult::Denied,
            json!({ "reason": "wrong_password" }),
        );
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "Incorrect account password.",
        ));
    }
    if b.confirm.trim() != "grow" {
        return Err(invalid("confirm", "Type grow to confirm."));
    }
    let pc = patient(&state, 25 * 60);
    let before = pc.disk_state(&rid).await?;
    let vol = before
        .volumes
        .iter()
        .find(|v| v.mount == b.mount)
        .cloned()
        .ok_or_else(|| invalid("mount", "That is not a volume opsbox can grow."))?;
    if let Some(why) = &vol.refusal {
        return Err(HttpError::new(ErrorCode::Conflict, why.clone()));
    }
    if !vol.growable {
        return Err(HttpError::new(
            ErrorCode::Conflict,
            "There is nothing to grow yet. Enlarge the volume at your cloud provider first.",
        ));
    }
    if vol.steps.contains(&opsbox_proto::DiskStep::InstallGrowpart) && !b.install_tools {
        return Err(invalid(
            "install_tools",
            "Growing the partition needs the growpart tool (package cloud-guest-utils). Allow opsbox to install it, or install it yourself.",
        ));
    }
    let (name, actor, ip, s) = (
        who.user.name.clone(),
        who.user.actor_label(),
        client.ip.clone(),
        state.clone(),
    );
    let task = state.tasks.start(DISK_TASK, async move {
        let mut log: Vec<String> = Vec::new();
        for step in &vol.steps {
            log.push(format!("{}…", step.label()));
            s.tasks.progress(DISK_TASK, log.clone());
            if let Err(e) = pc.disk_grow(&rid, &name, &vol.mount, *step).await {
                s.auth.audit(
                    &actor,
                    ActorKind::User,
                    Some(&ip),
                    "system.disk.grow",
                    Some(&vol.mount),
                    AuditResult::Failed,
                    json!({ "step": step.as_str(), "before": disk_sizes(&vol) }),
                );
                return Err(HttpError::from(e));
            }
            if let Some(last) = log.last_mut() {
                *last = step.label().to_owned();
            }
        }
        let after = pc
            .disk_state(&rid)
            .await
            .ok()
            .and_then(|d| d.volumes.into_iter().find(|v| v.mount == vol.mount));
        let steps: Vec<&str> = vol.steps.iter().map(|s| s.as_str()).collect();
        let detail = json!({
            "steps": steps,
            "before": disk_sizes(&vol),
            "after": after.as_ref().map(disk_sizes),
        });
        s.auth.audit(
            &actor,
            ActorKind::User,
            Some(&ip),
            "system.disk.grow",
            Some(&vol.mount),
            AuditResult::Ok,
            detail,
        );
        Ok(Outcome {
            log,
            result: json!({ "mount": vol.mount, "before": disk_sizes(&vol), "after": after.as_ref().map(disk_sizes) }),
        })
    })?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "task": task })),
    ))
}

#[derive(Deserialize)]
pub(crate) struct HostnameBody {
    name: String,
}

pub(crate) async fn set_hostname(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<HostnameBody>,
) -> Result<Json<Value>, HttpError> {
    state
        .priv_client
        .hostname_set(&rid, &who.user.name, b.name.trim())
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.hostname",
        Some(b.name.trim()),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct ZoneBody {
    zone: String,
}

pub(crate) async fn set_timezone(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(b): ApiJson<ZoneBody>,
) -> Result<Json<Value>, HttpError> {
    state
        .priv_client
        .timezone_set(&rid, &who.user.name, b.zone.trim())
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "system.timezone",
        Some(b.zone.trim()),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use opsbox_proto::PkgUpdate;

    #[test]
    fn loopback_callers_count_as_unknown_for_the_lockout_guard() {
        assert_eq!(caller_ip("203.0.113.5").as_deref(), Some("203.0.113.5"));
        assert_eq!(caller_ip("2001:db8::1").as_deref(), Some("2001:db8::1"));
        for unknown in ["127.0.0.1", "::1", "0.0.0.0", "unknown", "", "not an ip"] {
            assert_eq!(caller_ip(unknown), None, "{unknown}");
        }
    }

    #[test]
    fn ports_are_accepted_as_numbers_or_typed_strings() {
        assert_eq!(parse_port(&json!(5432)).unwrap(), 5432);
        assert_eq!(parse_port(&json!(" 22 ")).unwrap(), 22);
        for bad in [
            json!(0),
            json!(65536),
            json!(-1),
            json!("abc"),
            json!(""),
            json!(null),
            json!(1.5),
            json!([1]),
        ] {
            let e = parse_port(&bad).unwrap_err();
            assert_eq!(e.0.error.field.as_deref(), Some("port"), "{bad}");
        }
    }

    #[test]
    fn the_updates_view_matches_the_mock() {
        let p = PkgState {
            manager: "apt".into(),
            updates: vec![
                PkgUpdate {
                    name: "openssl".into(),
                    current: "1".into(),
                    available: "2".into(),
                    security: true,
                    kernel: false,
                },
                PkgUpdate {
                    name: "curl".into(),
                    current: "3".into(),
                    available: "4".into(),
                    security: false,
                    kernel: false,
                },
            ],
            reboot_required: true,
            reboot_at: None,
            auto_security: Some(true),
            running_kernel: "6.8.0-48-generic".into(),
            index_time: Some(1_700_000_000),
        };
        let v = updates_view(&p);
        assert_eq!(
            (
                v["total"].clone(),
                v["security"].clone(),
                v["reboot"].clone(),
                v["auto"].clone()
            ),
            (json!(2), json!(1), json!(true), json!(true))
        );
        assert_eq!(
            v["list"][0],
            json!({ "pkg": "openssl", "cur": "1", "next": "2", "sec": true, "kernel": false })
        );
        assert_eq!(v["running"], "6.8.0-48-generic");
    }
}
