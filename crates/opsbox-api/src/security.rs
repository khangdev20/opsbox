//! `GET /security`, `PUT /security/allow-ips`, `POST /security/checks/{id}/fix`
//! (docs/BACKEND_SPEC.md 7.14).
//!
//! The checks are cheap and read-only. A check with a safe automatic fix says so; fixes that
//! could cut the administrator off are guarded in the privileged helper.

use std::net::{IpAddr, SocketAddr};

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::response::{IntoResponse, Response};
use opsbox_proto::{ErrorCode, Role, SecurityScan, SshStatus};
use opsbox_store::{Store, User};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::allowlist::{self, Cidr};
use crate::session::{Admin, Client, Viewer};
use crate::system::{audit, patient};
use crate::{AppState, HttpError, RequestId};

const ALLOW_KEY: &str = "security.allow_ips";

/// The saved allowed-address list (empty when none or unreadable).
pub fn stored_allow_list(store: &Store) -> Vec<Cidr> {
    store
        .get_setting(ALLOW_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str())
                .filter_map(|s| Cidr::parse(s).ok())
                .collect()
        })
        .unwrap_or_default()
}

// ------------------------------------------------------------------ the checks

/// What the checks look at. A `None` means that source could not be read, and the checks
/// that need it are left out rather than guessed.
#[derive(Debug, Default)]
pub struct Inputs {
    pub without_2fa: Vec<String>,
    pub listen: Option<SocketAddr>,
    pub ssh: Option<SshStatus>,
    pub sshd_dropins: bool,
    pub firewall_on: Option<bool>,
    pub security_updates: Option<usize>,
    pub auto_updates: Option<bool>,
    pub world_readable: Option<Vec<String>>,
    pub master_key_private: Option<bool>,
    pub panel_tls_self_signed: Option<bool>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Check {
    pub id: &'static str,
    pub text: String,
    pub ok: bool,
    /// Label of the button that fixes it automatically.
    pub action: Option<&'static str>,
    /// A page that deals with it, when there is no automatic fix.
    pub link: Option<&'static str>,
}

fn check(id: &'static str, ok: bool, text: impl Into<String>) -> Check {
    Check {
        id,
        text: text.into(),
        ok,
        action: None,
        link: None,
    }
}

pub fn build_checks(i: &Inputs) -> Vec<Check> {
    let mut v = Vec::new();
    v.push(if i.without_2fa.is_empty() {
        check("2fa", true, "Two-step sign-in is on for every account")
    } else {
        check(
            "2fa",
            false,
            format!(
                "Two-step sign-in is off for: {}. They set it up at their next sign-in.",
                i.without_2fa.join(", ")
            ),
        )
    });
    v.push(check(
        "lockout",
        true,
        "Repeated wrong passwords lock an account for a while",
    ));
    if let Some(l) = i.listen {
        v.push(if l.ip().is_loopback() {
            check("panel", true, "The panel is reachable only from this server (through an SSH tunnel)")
        } else {
            check("panel", false, format!("The panel listens on {l} without HTTPS. Keep it on 127.0.0.1 and use an SSH tunnel, or put the opsbox proxy in front."))
        });
    }
    if let Some(on) = i.firewall_on {
        let mut c = check(
            "firewall",
            on,
            if on {
                "The firewall is on"
            } else {
                "The firewall is off: every port that something listens on is open"
            },
        );
        if !on {
            c.link = Some("#/system");
        }
        v.push(c);
    }
    if let Some(ssh) = &i.ssh {
        let running = ssh.installed && ssh.active;
        match (running, ssh.password_login) {
            (false, _) => v.push(check(
                "ssh-password",
                true,
                "No SSH server is running, so nothing accepts passwords",
            )),
            (true, Some(false)) => v.push(check(
                "ssh-password",
                true,
                "SSH accepts keys only, not passwords",
            )),
            (true, Some(true)) => {
                let mut c = check(
                    "ssh-password",
                    false,
                    "SSH accepts passwords, which can be guessed. Keys are much safer",
                );
                if i.sshd_dropins {
                    c.action = Some("Turn off");
                } else {
                    c.text
                        .push_str(" (edit sshd_config yourself: PasswordAuthentication no)");
                }
                v.push(c);
            }
            (true, None) => {}
        }
        if running && let Some(root) = &ssh.root_login {
            if root == "yes" {
                let mut c = check(
                    "ssh-root",
                    false,
                    "SSH allows logging in as root with a password",
                );
                if i.sshd_dropins {
                    c.action = Some("Turn off");
                }
                v.push(c);
            } else {
                v.push(check(
                    "ssh-root",
                    true,
                    match root.as_str() {
                        "no" => "SSH does not allow logging in as root",
                        _ => "SSH allows root only with a key",
                    },
                ));
            }
        }
    }
    if let Some(n) = i.security_updates {
        let mut c = check(
            "updates",
            n == 0,
            if n == 0 {
                "No security updates are waiting".to_owned()
            } else {
                format!(
                    "{n} security update{} waiting to be installed",
                    if n == 1 { " is" } else { "s are" }
                )
            },
        );
        if n > 0 {
            c.link = Some("#/system");
        }
        v.push(c);
    }
    if let Some(auto) = i.auto_updates {
        let mut c = check(
            "autoupdates",
            auto,
            if auto {
                "Security updates install themselves"
            } else {
                "Security updates are not installed automatically"
            },
        );
        if !auto {
            c.action = Some("Turn on");
        }
        v.push(c);
    }
    if let Some(list) = &i.world_readable {
        if list.is_empty() {
            v.push(check(
                "secrets",
                true,
                "No secrets files in your apps can be read by everyone",
            ));
        } else {
            let mut c = check(
                "secrets",
                false,
                format!(
                    "{} secrets file{} in your apps can be read by anyone on this server (for example {})",
                    list.len(),
                    if list.len() == 1 { "" } else { "s" },
                    list[0]
                ),
            );
            c.action = Some("Fix permissions");
            v.push(c);
        }
    }
    if let Some(private) = i.master_key_private {
        v.push(check(
            "masterkey",
            private,
            if private {
                "The key that seals saved secrets is private to the server"
            } else {
                "The key that seals saved secrets can be read by other users (it must be mode 0600)"
            },
        ));
    }
    if let Some(self_signed) = i.panel_tls_self_signed {
        let mut c = check(
            "panel-tls",
            !self_signed,
            if self_signed {
                "The panel uses a self-signed certificate and has no domain yet"
            } else {
                "The panel is served over HTTPS with a domain certificate"
            },
        );
        if self_signed {
            c.link = Some("#/settings");
        }
        v.push(c);
    }
    v
}

// ------------------------------------------------------------------ audit helpers

/// `2026-11-17T07:55:46.123Z` as milliseconds since 1970. Anything else gives 0.
pub fn rfc3339_ms(ts: &str) -> i64 {
    let b = ts.as_bytes();
    let num = |a: usize, n: usize| -> Option<i64> { ts.get(a..a + n)?.parse().ok() };
    let parse = || -> Option<i64> {
        if b.len() < 19 || b[4] != b'-' || b[10] != b'T' {
            return None;
        }
        let (y, m, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
        let (hh, mm, ss) = (num(11, 2)?, num(14, 2)?, num(17, 2)?);
        // Days from civil (Howard Hinnant's algorithm).
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        let days = era * 146_097 + doe - 719_468;
        let frac = ts
            .get(19..)
            .and_then(|r| r.strip_prefix('.'))
            .map(|r| {
                r.chars()
                    .take_while(char::is_ascii_digit)
                    .take(3)
                    .collect::<String>()
            })
            .filter(|f| !f.is_empty())
            .map_or(0, |f| {
                f.parse::<i64>().unwrap_or(0) * 10_i64.pow(3 - f.len() as u32)
            });
        Some(((days * 24 + hh) * 60 + mm) * 60 * 1000 + ss * 1000 + frac)
    };
    parse().unwrap_or(0)
}

fn role_text(role: Role) -> &'static str {
    match role {
        Role::Admin => "Administrator",
        Role::Deployer => "Deploy and view logs, no terminal",
        Role::Viewer => "View only",
    }
}

// ------------------------------------------------------------------ endpoints

/// `GET /security`
pub(crate) async fn get(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let pc = patient(&state, 60);
    let apps = state.files.apps_dir.to_string_lossy().into_owned();
    let (access, fw, pkgs, scan) = tokio::join!(
        pc.access_list(&rid),
        pc.firewall_list(&rid),
        pc.pkg_list(&rid),
        pc.security_scan(&rid, &apps)
    );
    let users = state
        .store
        .list_users()
        .map_err(|_| HttpError::internal())?;
    let is_admin = who.user.role == Role::Admin;
    let scan: Option<SecurityScan> = scan.ok();
    let inputs = Inputs {
        without_2fa: users
            .iter()
            .filter(|u| !u.totp_enabled && !state.auth.is_pending(u))
            .map(|u| u.name.clone())
            .collect(),
        listen: Some(state.listen),
        ssh: access.ok().map(|a| a.ssh),
        sshd_dropins: scan.as_ref().is_some_and(|s| s.sshd_dropins),
        firewall_on: fw.ok().map(|f| !f.rules.is_empty()),
        security_updates: pkgs
            .as_ref()
            .ok()
            .filter(|p| !p.manager.is_empty())
            .map(|p| p.updates.iter().filter(|u| u.security).count()),
        auto_updates: pkgs.ok().and_then(|p| p.auto_security),
        world_readable: scan.as_ref().map(|s| s.world_readable.clone()),
        master_key_private: scan.as_ref().map(|s| s.master_key_private),
        panel_tls_self_signed: state.tls.then(|| {
            state
                .store
                .get_setting("opsbox.panel.domain")
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(|s| !s.is_empty()))
                != Some(true)
        }),
    };
    let checks = build_checks(&inputs);
    let passing = checks.iter().filter(|c| c.ok).count();
    let audit_rows: Vec<Value> = if is_admin {
        state
            .store
            .recent_audit(12)
            .map_err(|_| HttpError::internal())?
            .into_iter()
            .map(|e| json!({ "t": rfc3339_ms(&e.ts), "user": e.actor, "action": e.action, "warn": e.result != "ok" }))
            .collect()
    } else {
        Vec::new()
    };
    Ok(Json(json!({
        "checks": checks.iter().map(|c| json!({ "id": c.id, "t": c.text, "ok": c.ok, "action": c.action, "link": c.link })).collect::<Vec<_>>(),
        "total": checks.len(), "passing": passing,
        "users": users.iter().map(|u| user_json(&state, u, &who.user)).collect::<Vec<_>>(),
        "audit": audit_rows,
        "allowIps": allowlist::render(&state.allow.get()),
    })))
}

fn user_json(state: &AppState, u: &User, me: &User) -> Value {
    let pending = state.auth.is_pending(u);
    json!({
        "name": u.name, "role": if pending { format!("{} · invitation not used yet", role_text(u.role)) } else { role_text(u.role).to_owned() },
        "roleKey": u.role, "tfa": u.totp_enabled, "pending": pending, "you": u.id == me.id,
    })
}

#[derive(Deserialize)]
pub(crate) struct AllowBody {
    value: String,
    #[serde(default)]
    confirm: bool,
}

/// `PUT /security/allow-ips`
pub(crate) async fn set_allow_ips(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    crate::json::ApiJson(body): crate::json::ApiJson<AllowBody>,
) -> Result<Json<Value>, HttpError> {
    crate::session::deny_token(&who)?;
    let list =
        allowlist::parse_list(&body.value).map_err(|m| HttpError::new(ErrorCode::Validation, m))?;
    if list.len() > 50 {
        return Err(HttpError::new(ErrorCode::Validation, "At most 50 entries."));
    }
    // Do not let a typo lock the person out: their own address must be on the list.
    if let Ok(mine) = client.ip.parse::<IpAddr>()
        && !mine.is_loopback()
        && !list.is_empty()
        && !list.iter().any(|c| c.contains(mine))
        && !body.confirm
    {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            format!(
                "This list does not include your current address ({mine}). After saving, this browser could no longer use the panel."
            ),
        ));
    }
    let strings: Vec<String> = list.iter().map(ToString::to_string).collect();
    state
        .store
        .set_setting(ALLOW_KEY, &json!(strings))
        .map_err(|_| HttpError::internal())?;
    state.allow.set(list);
    audit(
        &state,
        &who,
        &client.ip,
        "security.allow_ips",
        None,
        json!({ "entries": strings }),
    );
    Ok(Json(json!({ "ok": true, "allowIps": strings.join(", ") })))
}

/// `POST /security/checks/{id}/fix`
pub(crate) async fn fix(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Response, HttpError> {
    if id == "autoupdates" {
        return crate::system::set_auto_updates(
            State(state),
            Admin(who),
            client,
            Extension(RequestId(rid)),
            crate::json::ApiJson(crate::system::OnBody { on: true }),
        )
        .await
        .map(IntoResponse::into_response);
    }
    let pc = patient(&state, 700);
    match id.as_str() {
        "secrets" | "ssh-password" | "ssh-root" => {
            let apps = state.files.apps_dir.to_string_lossy().into_owned();
            pc.security_fix(&rid, &who.user.name, &id, &apps).await?;
        }
        _ => {
            return Err(HttpError::new(
                ErrorCode::Validation,
                "This check has no automatic fix.",
            ));
        }
    }
    audit(
        &state,
        &who,
        &client.ip,
        "security.fix",
        Some(&id),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(c: &[Check]) -> Vec<&str> {
        c.iter().map(|c| c.id).collect()
    }

    #[test]
    fn a_healthy_server_passes_everything_it_can_be_asked() {
        let c = build_checks(&Inputs {
            listen: Some("127.0.0.1:8443".parse().unwrap()),
            ssh: Some(SshStatus {
                installed: true,
                active: true,
                port: 22,
                password_login: Some(false),
                root_login: Some("no".into()),
            }),
            firewall_on: Some(true),
            security_updates: Some(0),
            auto_updates: Some(true),
            world_readable: Some(vec![]),
            master_key_private: Some(true),
            ..Inputs::default()
        });
        assert!(
            c.iter()
                .all(|c| c.ok && c.action.is_none() && c.link.is_none()),
            "{c:?}"
        );
        assert_eq!(
            ids(&c),
            [
                "2fa",
                "lockout",
                "panel",
                "firewall",
                "ssh-password",
                "ssh-root",
                "updates",
                "autoupdates",
                "secrets",
                "masterkey"
            ]
        );
    }

    #[test]
    fn problems_come_with_the_right_remedy() {
        let c = build_checks(&Inputs {
            without_2fa: vec!["dana".into(), "erin".into()],
            listen: Some("0.0.0.0:8443".parse().unwrap()),
            ssh: Some(SshStatus {
                installed: true,
                active: true,
                port: 22,
                password_login: Some(true),
                root_login: Some("yes".into()),
            }),
            sshd_dropins: true,
            firewall_on: Some(false),
            security_updates: Some(3),
            auto_updates: Some(false),
            world_readable: Some(vec!["/var/lib/opsbox/apps/web/.env".into()]),
            master_key_private: Some(false),
            panel_tls_self_signed: Some(true),
        });
        let by = |id: &str| c.iter().find(|c| c.id == id).unwrap();
        assert!(!by("2fa").ok && by("2fa").text.contains("dana, erin"));
        assert!(!by("panel").ok && by("panel").text.contains("0.0.0.0:8443"));
        assert_eq!(
            (by("firewall").ok, by("firewall").link),
            (false, Some("#/system"))
        );
        assert_eq!(
            (by("ssh-password").ok, by("ssh-password").action),
            (false, Some("Turn off"))
        );
        assert_eq!(
            (by("ssh-root").ok, by("ssh-root").action),
            (false, Some("Turn off"))
        );
        assert_eq!(
            (by("updates").ok, by("updates").link),
            (false, Some("#/system"))
        );
        assert!(by("updates").text.starts_with("3 security updates are"));
        assert_eq!(
            (by("autoupdates").ok, by("autoupdates").action),
            (false, Some("Turn on"))
        );
        assert_eq!(
            (by("secrets").ok, by("secrets").action),
            (false, Some("Fix permissions"))
        );
        assert!(by("secrets").text.contains(".env"));
        assert!(
            !by("masterkey").ok && by("masterkey").action.is_none(),
            "no automatic fix for the master key"
        );
    }

    #[test]
    fn unreadable_sources_leave_their_checks_out_and_ssh_without_dropins_has_no_button() {
        let c = build_checks(&Inputs::default());
        assert_eq!(ids(&c), ["2fa", "lockout"], "nothing is guessed");
        let c = build_checks(&Inputs {
            ssh: Some(SshStatus {
                installed: true,
                active: true,
                port: 22,
                password_login: Some(true),
                root_login: Some("prohibit-password".into()),
            }),
            sshd_dropins: false,
            ..Inputs::default()
        });
        let pw = c.iter().find(|c| c.id == "ssh-password").unwrap();
        assert!(pw.action.is_none() && pw.text.contains("edit sshd_config yourself"));
        assert!(
            c.iter().find(|c| c.id == "ssh-root").unwrap().ok,
            "key-only root is acceptable"
        );
        // A stopped SSH server accepts nothing.
        let c = build_checks(&Inputs {
            ssh: Some(SshStatus {
                installed: true,
                active: false,
                port: 22,
                password_login: Some(true),
                root_login: Some("yes".into()),
            }),
            ..Inputs::default()
        });
        assert!(c.iter().find(|c| c.id == "ssh-password").unwrap().ok);
        assert!(c.iter().all(|c| c.id != "ssh-root"));
    }

    #[test]
    fn timestamps_become_milliseconds() {
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), 0);
        assert_eq!(rfc3339_ms("2026-11-17T07:55:46.123Z"), 1_794_902_146_123);
        assert_eq!(rfc3339_ms("2024-02-29T12:00:00Z"), 1_709_208_000_000);
        assert_eq!(rfc3339_ms("garbage"), 0);
        assert_eq!(rfc3339_ms(""), 0);
    }
}
