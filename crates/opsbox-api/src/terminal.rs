//! The terminal (docs/BACKEND_SPEC.md 7.9): `GET /terminal/users` and
//! `WS /stream/terminal?user=&cols=&rows=&confirm=`.
//!
//! The shell itself runs in `priv`. This module only authorizes, relays and audits:
//! keystrokes arrive as binary WebSocket frames and go to `priv`; output goes the other
//! way. A text frame `{"type":"resize","cols","rows"}` resizes the terminal. The server
//! answers with text frames `ready`, `exit` and `error`, and binary frames for output.
//! Only the start and the end of a session are audited, never what was typed.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use opsbox_priv::{PtyReader, PtySession, PtyWriter};
use opsbox_proto::{ErrorCode, PtyFrame, SystemUser, clamp_size};
use opsbox_store::{ActorKind, AuditResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;

use crate::session::{Admin, Client, origin_matches_host};
use crate::{AppState, HttpError, RequestId};

/// Terminals open at once, in total. Each is a real process.
const MAX_OPEN: usize = 8;
/// A browser tab that never sends a key is closed after this long.
pub const DEFAULT_IDLE: Duration = Duration::from_secs(30 * 60);

/// Settings and bookkeeping for terminals. Cheap to clone.
#[derive(Clone, Debug)]
pub struct TerminalSettings {
    pub idle: Duration,
    open: Arc<AtomicUsize>,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            idle: DEFAULT_IDLE,
            open: Arc::default(),
        }
    }
}

impl TerminalSettings {
    pub fn new(idle: Duration) -> Self {
        Self {
            idle,
            open: Arc::default(),
        }
    }

    fn reserve(&self) -> Option<Slot> {
        let n = self.open.fetch_add(1, Ordering::AcqRel);
        if n >= MAX_OPEN {
            self.open.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Slot(self.open.clone()))
    }
}

/// One of the [`MAX_OPEN`] places; freed when dropped.
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// The lowest uid of an ordinary account (`UID_MIN` in `login.defs`; macOS starts at 501).
const FIRST_USER_UID: u32 = if cfg!(target_os = "linux") { 1000 } else { 500 };

/// Users with a real login shell, and the one a terminal opens as by default: the first
/// ordinary account, never root.
fn login_users(all: Vec<SystemUser>) -> (Vec<SystemUser>, Option<String>) {
    let users: Vec<SystemUser> = all
        .into_iter()
        .filter(|u| u.login && (u.uid == 0 || (FIRST_USER_UID..60000).contains(&u.uid)))
        .collect();
    let default = users
        .iter()
        .find(|u| (FIRST_USER_UID..60000).contains(&u.uid))
        .map(|u| u.name.clone());
    (users, default)
}

/// `GET /terminal/users`
pub(crate) async fn users(
    Admin(who): Admin,
    State(state): State<AppState>,
    rid: axum::Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    crate::session::deny_token(&who)?;
    let all = state.priv_client.users(&rid.0.0).await?;
    let (users, default) = login_users(all);
    Ok(Json(json!({
        "default": default,
        "users": users.iter().map(|u| json!({
            "name": u.name, "uid": u.uid, "home": u.home, "shell": u.shell, "root": u.uid == 0,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub(crate) struct OpenQuery {
    user: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    /// Must be `root` to open a shell as root.
    confirm: Option<String>,
}

/// `WS /stream/terminal`
///
/// Who may open a terminal (role, Origin) is decided with plain HTTP errors before the
/// upgrade. Everything after that (which user, root confirmation, too many terminals, a
/// helper that is down) is reported inside the socket as an `error` message, because a
/// browser cannot read the body of a refused upgrade and the person deserves the reason.
pub(crate) async fn stream(
    // Extractors run in this order: authenticate before looking at the upgrade headers.
    Admin(who): Admin,
    client: Client,
    State(state): State<AppState>,
    rid: axum::Extension<RequestId>,
    Query(q): Query<OpenQuery>,
    ws: WebSocketUpgrade,
) -> Result<Response, HttpError> {
    crate::session::deny_token(&who)?;
    if !origin_matches_host(&client.headers) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "This request came from another site and was refused.",
        ));
    }
    let idle = state.terminal.idle;
    Ok(ws.on_upgrade(move |mut socket| async move {
        let opened = prepare(&state, &who.user.name, &rid.0.0, &q).await;
        let (session, slot) = match opened {
            Ok(v) => v,
            Err(e) => {
                let _ = socket
                    .send(text(&json!({
                        "type": "error",
                        "code": e.0.error.code,
                        "message": e.0.error.message,
                    })))
                    .await;
                let _ = socket
                    .send(Message::Close(Some(CloseFrame {
                        code: 1008,
                        reason: "refused".into(),
                    })))
                    .await;
                return;
            }
        };
        let audit = Audit {
            state: state.clone(),
            who: who.user.name.clone(),
            ip: client.ip.clone(),
            user: session.user.clone(),
        };
        audit.record("terminal.start", json!({}));
        let started = Instant::now();
        let reason = relay(socket, session, idle).await;
        audit.record(
            "terminal.end",
            json!({ "seconds": started.elapsed().as_secs(), "reason": reason.as_str() }),
        );
        drop(slot);
    }))
}

/// Check the request and start the shell.
async fn prepare(
    state: &AppState,
    actor: &str,
    rid: &str,
    q: &OpenQuery,
) -> Result<(PtySession, Slot), HttpError> {
    let slot = state.terminal.reserve().ok_or_else(|| {
        HttpError::new(
            ErrorCode::Conflict,
            "Too many terminals are open. Close one and try again.",
        )
    })?;
    let all = state.priv_client.users(rid).await?;
    let (users, default) = login_users(all);
    let name = match q.user.as_deref().filter(|u| !u.is_empty()) {
        Some(n) => n.to_owned(),
        None => default.ok_or_else(|| {
            HttpError::new(
                ErrorCode::Validation,
                "This server has no ordinary user to open a terminal as. Choose one, or root.",
            )
        })?,
    };
    // Only accounts the page offers can be chosen.
    let chosen = users.iter().find(|u| u.name == name).ok_or_else(|| {
        HttpError::new(
            ErrorCode::Validation,
            format!("A terminal cannot be opened as “{name}”."),
        )
    })?;
    if chosen.uid == 0 && q.confirm.as_deref() != Some("root") {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            "A terminal as root can change anything on this server. Confirm to open it.",
        ));
    }
    let (cols, rows) = clamp_size(q.cols.unwrap_or(80), q.rows.unwrap_or(24));
    let session = state
        .priv_client
        .pty_open(rid, Some(actor), &name, cols, rows)
        .await?;
    Ok((session, slot))
}

struct Audit {
    state: AppState,
    who: String,
    ip: String,
    /// The Linux account the shell runs as.
    user: String,
}

impl Audit {
    fn record(&self, action: &str, mut detail: Value) {
        detail["ip"] = json!(self.ip);
        self.state.auth.audit(
            &self.who,
            ActorKind::User,
            Some(&self.ip),
            action,
            Some(&self.user),
            AuditResult::Ok,
            detail,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EndReason {
    /// The shell exited (the user typed `exit`).
    ShellExited,
    /// The browser closed the connection.
    Closed,
    /// Nothing was typed for the idle timeout.
    Idle,
    /// The helper went away.
    Error,
}

impl EndReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::ShellExited => "shell_exited",
            Self::Closed => "closed",
            Self::Idle => "idle",
            Self::Error => "error",
        }
    }
}

fn text(v: &Value) -> Message {
    Message::text(v.to_string())
}

/// Move bytes both ways until one side ends. Dropping the `priv` ends of the stream is what
/// makes `priv` hang the shell up.
pub(crate) async fn relay(socket: WebSocket, session: PtySession, idle: Duration) -> EndReason {
    let PtySession {
        user,
        shell,
        reader,
        writer,
    } = session;
    let (mut tx, mut rx) = socket.split();
    if tx
        .send(text(
            &json!({ "type": "ready", "user": user, "shell": shell }),
        ))
        .await
        .is_err()
    {
        return EndReason::Closed;
    }

    // Shell → browser. Each send waits for the browser, and nothing more is read from `priv`
    // meanwhile, so a slow tab slows the shell down instead of filling memory here.
    let output = async move {
        let mut reader: PtyReader = reader;
        loop {
            match reader.recv().await {
                Ok(Some(PtyFrame::Data(d))) => {
                    if tx.send(Message::binary(d)).await.is_err() {
                        return (EndReason::Closed, tx);
                    }
                }
                Ok(Some(PtyFrame::Exit { code })) => {
                    let _ = tx
                        .send(text(&json!({ "type": "exit", "code": code })))
                        .await;
                    return (EndReason::ShellExited, tx);
                }
                Ok(Some(PtyFrame::Resize { .. })) => {}
                Ok(None) | Err(_) => {
                    let _ = tx
                        .send(text(&json!({
                            "type": "error",
                            "message": "The connection to the server helper was lost."
                        })))
                        .await;
                    return (EndReason::Error, tx);
                }
            }
        }
    };

    // Browser → shell. Keystrokes count as activity; nothing else does.
    let input = async move {
        let mut writer: PtyWriter = writer;
        let mut deadline = Instant::now() + idle;
        loop {
            let next = tokio::select! {
                m = rx.next() => m,
                () = tokio::time::sleep_until(deadline.into()) => return EndReason::Idle,
            };
            let sent = match next {
                None | Some(Err(_) | Ok(Message::Close(_))) => return EndReason::Closed,
                Some(Ok(Message::Binary(b))) => {
                    deadline = Instant::now() + idle;
                    // Large pastes are cut into pieces `priv` accepts.
                    let mut ok = true;
                    for chunk in b.chunks(opsbox_proto::MAX_PTY_PAYLOAD) {
                        if writer.send(&PtyFrame::Data(chunk.to_vec())).await.is_err() {
                            ok = false;
                            break;
                        }
                    }
                    ok
                }
                Some(Ok(Message::Text(t))) => match serde_json::from_str::<Value>(&t) {
                    Ok(v) if v["type"] == "resize" => {
                        let c = v["cols"].as_u64().and_then(|n| u16::try_from(n).ok());
                        let r = v["rows"].as_u64().and_then(|n| u16::try_from(n).ok());
                        match (c, r) {
                            (Some(cols), Some(rows)) => {
                                let (cols, rows) = clamp_size(cols, rows);
                                writer.send(&PtyFrame::Resize { cols, rows }).await.is_ok()
                            }
                            _ => true,
                        }
                    }
                    _ => true,
                },
                Some(Ok(_)) => true,
            };
            if !sent {
                return EndReason::Error;
            }
        }
    };

    tokio::pin!(output, input);
    tokio::select! {
        (reason, mut tx) = &mut output => {
            // Say goodbye properly so the page sees a clean close after the `exit` message.
            let _ = tx
                .send(Message::Close(Some(CloseFrame { code: 1000, reason: "".into() })))
                .await;
            reason
        }
        reason = &mut input => {
            debug!(?reason, "terminal browser side ended");
            reason
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(name: &str, uid: u32, login: bool) -> SystemUser {
        SystemUser {
            name: name.into(),
            uid,
            gid: uid,
            home: format!("/home/{name}"),
            shell: "/bin/bash".into(),
            login,
        }
    }

    #[test]
    fn the_default_user_is_the_first_ordinary_one_and_never_root() {
        let (users, default) = login_users(vec![
            u("root", 0, true),
            u("daemon", 1, false),
            u("ubuntu", 1000, true),
            u("deploy", 1001, true),
            u("nobody", 65534, true),
            u("locked", 1002, false),
        ]);
        assert_eq!(default.as_deref(), Some("ubuntu"));
        let names: Vec<_> = users.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["root", "ubuntu", "deploy"]);
    }

    #[test]
    fn with_only_root_there_is_no_default() {
        let (users, default) = login_users(vec![u("root", 0, true)]);
        assert_eq!(users.len(), 1);
        assert_eq!(default, None);
    }

    #[test]
    fn at_most_eight_terminals_are_open_at_once() {
        let s = TerminalSettings::default();
        let slots: Vec<_> = (0..MAX_OPEN).map(|_| s.reserve().unwrap()).collect();
        assert!(s.reserve().is_none());
        drop(slots);
        assert!(s.reserve().is_some(), "slots are freed when sessions end");
    }
}
