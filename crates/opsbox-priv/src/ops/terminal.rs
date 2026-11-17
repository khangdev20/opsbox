//! The terminal: which accounts can have a shell, and the PTY session itself.
//!
//! A shell is started as the chosen user through `runuser -l`, which sets up the
//! environment, groups and PAM session the way a login does. When `priv` does not run as
//! root (development), only the user it runs as can be used, with that user's own shell.

use std::time::Duration;

use nix::unistd::{Pid, User, geteuid};
use opsbox_proto::{
    MAX_PTY_PAYLOAD, PrivError, PrivErrorCode, PrivReply, PtyFrame, SystemUser, clamp_size,
};
use pty_process::Size;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::{debug, info, warn};

use crate::io::{read_pty_frame, write_pty_frame};

/// How long a shell gets to react to the hangup before it is killed.
const HANGUP_GRACE: Duration = Duration::from_secs(2);

fn err(code: PrivErrorCode, message: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}

/// Parse `/etc/passwd`. Lines that are not valid entries are skipped.
pub(crate) fn parse_passwd(text: &str) -> Vec<SystemUser> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let f: Vec<&str> = line.split(':').collect();
            if f.len() < 7 || f[0].is_empty() {
                return None;
            }
            let shell = f[6].trim();
            Some(SystemUser {
                name: f[0].to_owned(),
                uid: f[2].parse().ok()?,
                gid: f[3].parse().ok()?,
                home: f[5].to_owned(),
                shell: shell.to_owned(),
                login: has_login_shell(shell),
            })
        })
        .collect()
}

/// A shell that allows an interactive login. An empty shell means `/bin/sh`.
fn has_login_shell(shell: &str) -> bool {
    if shell.is_empty() {
        return true;
    }
    let base = shell.rsplit('/').next().unwrap_or(shell);
    !matches!(
        base,
        "nologin" | "false" | "true" | "sync" | "shutdown" | "halt"
    )
}

/// An account from the system's user database (which also knows accounts that are not in
/// `/etc/passwd`, such as directory users).
fn from_database(u: User) -> SystemUser {
    let shell = u.shell.to_string_lossy().into_owned();
    SystemUser {
        name: u.name,
        uid: u.uid.as_raw(),
        gid: u.gid.as_raw(),
        home: u.dir.to_string_lossy().into_owned(),
        login: has_login_shell(&shell),
        shell,
    }
}

fn read_passwd() -> Result<Vec<SystemUser>, PrivError> {
    let text = std::fs::read_to_string("/etc/passwd").map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Could not read the list of users: {e}"),
        )
    })?;
    Ok(parse_passwd(&text))
}

/// `/etc/passwd`, plus the account this process runs as if the file does not have it.
pub(crate) fn list() -> Result<PrivReply, PrivError> {
    let mut users = read_passwd()?;
    if let Ok(Some(me)) = User::from_uid(geteuid())
        && !users.iter().any(|u| u.uid == me.uid.as_raw())
    {
        users.push(from_database(me));
    }
    Ok(PrivReply::Users(users))
}

/// A user name `useradd` would accept; nothing else ever reaches a command line.
pub(crate) fn valid_user_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// Pick the account a shell will run as. Pure so it can be tested without a real system.
pub(crate) fn choose_user(
    users: &[SystemUser],
    name: &str,
    euid: u32,
    current_name: Option<&str>,
) -> Result<SystemUser, PrivError> {
    if !valid_user_name(name) {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a valid user name.",
        ));
    }
    let user = users.iter().find(|u| u.name == name).ok_or_else(|| {
        err(
            PrivErrorCode::NotFound,
            format!("There is no user “{name}”."),
        )
    })?;
    if !user.login {
        return Err(err(
            PrivErrorCode::Invalid,
            format!("“{name}” has no login shell, so a terminal cannot be opened as that user."),
        ));
    }
    if euid != 0 && current_name != Some(name) {
        return Err(err(
            PrivErrorCode::Refused,
            "opsbox is not running as root, so a terminal can only be opened as its own user.",
        ));
    }
    Ok(user.clone())
}

/// A running shell on a PTY.
pub(crate) struct Session {
    pty: pty_process::Pty,
    child: tokio::process::Child,
    pub user: String,
    pub shell: String,
}

pub(crate) fn open(name: &str, cols: u16, rows: u16) -> Result<Session, PrivError> {
    let mut users = read_passwd()?;
    // Accounts that come from somewhere other than /etc/passwd (sssd, LDAP) are found by name.
    if valid_user_name(name)
        && !users.iter().any(|u| u.name == name)
        && let Ok(Some(u)) = User::from_name(name)
    {
        users.push(from_database(u));
    }
    let euid = geteuid();
    let current = User::from_uid(euid).ok().flatten().map(|u| u.name);
    let user = choose_user(&users, name, euid.as_raw(), current.as_deref())?;
    let shell = if user.shell.is_empty() {
        "/bin/sh".to_owned()
    } else {
        user.shell.clone()
    };

    let (cols, rows) = clamp_size(cols, rows);
    let (pty, pts) = pty_process::open().map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Could not allocate a terminal: {e}"),
        )
    })?;
    pty.resize(Size::new(rows, cols)).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("Could not size the terminal: {e}"),
        )
    })?;

    let cmd = if euid.is_root() {
        pty_process::Command::new("runuser").args(["-l", &user.name])
    } else {
        // Development: the user's own shell, as a login shell (a leading dash in argv[0]).
        let base = shell.rsplit('/').next().unwrap_or("sh").to_owned();
        pty_process::Command::new(&shell).arg0(format!("-{base}"))
    };
    let child = cmd
        .env("TERM", "xterm-256color")
        .kill_on_drop(true)
        .spawn(pts)
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not start the shell: {e}"),
            )
        })?;
    info!(user = %user.name, %shell, "terminal opened");
    Ok(Session {
        pty,
        child,
        user: user.name,
        shell,
    })
}

/// Pump bytes between the PTY and the stream until either side ends, then clean up.
///
/// Backpressure: output is read from the PTY only as fast as it is written to the stream,
/// so a slow browser makes the kernel's PTY buffer fill up and the program block, instead
/// of this process buffering without limit.
pub(crate) async fn run<R, W>(session: Session, mut from_peer: R, mut to_peer: W)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let Session { pty, mut child, .. } = session;
    let pid = child.id();
    let (mut pty_out, mut pty_in) = pty.into_split();

    // Shell output → peer. Ends when the shell closes the terminal (read returns 0 or EIO).
    let output = async {
        let mut buf = vec![0u8; MAX_PTY_PAYLOAD];
        loop {
            match pty_out.read(&mut buf).await {
                Ok(0) | Err(_) => return Ok::<(), std::io::Error>(()),
                Ok(n) => write_pty_frame(&mut to_peer, &PtyFrame::Data(buf[..n].to_vec())).await?,
            }
        }
    };

    // Peer → shell. Ends when the peer goes away or breaks the protocol.
    let input = async {
        loop {
            match read_pty_frame(&mut from_peer).await {
                Ok(Some(PtyFrame::Data(bytes))) => {
                    if pty_in.write_all(&bytes).await.is_err() {
                        return;
                    }
                }
                Ok(Some(PtyFrame::Resize { cols, rows })) => {
                    let (c, r) = clamp_size(cols, rows);
                    if let Err(e) = pty_in.resize(Size::new(r, c)) {
                        debug!(error = %e, "resize failed");
                    }
                }
                Ok(Some(PtyFrame::Exit { .. })) | Ok(None) => return,
                Err(e) => {
                    debug!(error = %e, "terminal stream ended");
                    return;
                }
            }
        }
    };

    // The futures borrow the stream halves; they are dropped before the exit message is sent.
    let shell_ended = {
        tokio::pin!(output, input);
        tokio::select! {
            r = &mut output => {
                if let Err(e) = r {
                    debug!(error = %e, "peer stopped reading");
                    false
                } else {
                    true
                }
            }
            () = &mut input => false,
        }
    };

    let code = if shell_ended {
        match tokio::time::timeout(HANGUP_GRACE, child.wait()).await {
            Ok(Ok(status)) => status.code(),
            _ => {
                hang_up(pid, &mut child).await;
                None
            }
        }
    } else {
        hang_up(pid, &mut child).await;
        None
    };
    if shell_ended {
        // The peer may already be gone; nothing to do about it.
        let _ = write_pty_frame(&mut to_peer, &PtyFrame::Exit { code }).await;
    }
    let _ = to_peer.shutdown().await;
    info!(?code, shell_ended, "terminal closed");
}

/// End the shell and everything in its session: hangup first, kill when it lingers.
async fn hang_up(pid: Option<u32>, child: &mut tokio::process::Child) {
    use nix::sys::signal::{Signal, killpg};
    let group = pid.and_then(|p| i32::try_from(p).ok()).map(Pid::from_raw);
    if let Some(g) = group {
        let _ = killpg(g, Signal::SIGHUP);
    }
    if tokio::time::timeout(HANGUP_GRACE, child.wait())
        .await
        .is_err()
    {
        warn!("shell ignored the hangup; killing it");
        if let Some(g) = group {
            let _ = killpg(g, Signal::SIGKILL);
        }
        let _ = child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
ubuntu:x:1000:1000:Ubuntu:/home/ubuntu:/bin/bash
# comment
broken:line
bin:x:2:2:bin:/bin:/bin/false
bare:x:1001:1001::/home/bare:
";

    #[test]
    fn passwd_is_parsed_and_login_shells_detected() {
        let users = parse_passwd(PASSWD);
        let names: Vec<_> = users.iter().map(|u| (u.name.as_str(), u.login)).collect();
        assert_eq!(
            names,
            [
                ("root", true),
                ("daemon", false),
                ("ubuntu", true),
                ("bin", false),
                ("bare", true)
            ]
        );
        assert_eq!(users[2].uid, 1000);
        assert_eq!(users[2].home, "/home/ubuntu");
    }

    #[test]
    fn user_names_are_restricted() {
        for ok in ["ubuntu", "a", "deploy-1", "svc_x", "j.doe"] {
            assert!(valid_user_name(ok), "{ok}");
        }
        for bad in ["", "-r", "a b", "a;b", "a/b", "ü", &"x".repeat(33), "$(id)"] {
            assert!(!valid_user_name(bad), "{bad}");
        }
    }

    #[test]
    fn only_real_users_with_shells_can_be_chosen() {
        let users = parse_passwd(PASSWD);
        assert_eq!(choose_user(&users, "ubuntu", 0, None).unwrap().uid, 1000);
        assert_eq!(choose_user(&users, "root", 0, None).unwrap().uid, 0);
        assert_eq!(
            choose_user(&users, "daemon", 0, None).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            choose_user(&users, "nobody", 0, None).unwrap_err().code,
            PrivErrorCode::NotFound
        );
        assert_eq!(
            choose_user(&users, "-r", 0, None).unwrap_err().code,
            PrivErrorCode::Invalid
        );
    }

    #[test]
    fn without_root_only_the_own_user_is_allowed() {
        let users = parse_passwd(PASSWD);
        assert!(choose_user(&users, "ubuntu", 1000, Some("ubuntu")).is_ok());
        assert_eq!(
            choose_user(&users, "root", 1000, Some("ubuntu"))
                .unwrap_err()
                .code,
            PrivErrorCode::Refused
        );
    }
}
