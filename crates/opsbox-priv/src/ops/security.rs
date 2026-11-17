//! What the Security page needs from the machine, and its automatic fixes
//! (docs/BACKEND_SPEC.md 7.14).
//!
//! A fix only goes ahead when it cannot lock the administrator out and when afterwards the
//! setting is verifiably in effect; otherwise it is undone and the reason is reported.

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::Duration;

use nix::unistd::geteuid;
use opsbox_proto::{PrivError, PrivErrorCode, PrivReply, SecurityScan};

use super::access;
use super::exec::{find, run};

const QUICK: Duration = Duration::from_secs(15);
const SSHD_MAIN: &str = "/etc/ssh/sshd_config";
const DROPIN_DIR: &str = "/etc/ssh/sshd_config.d";
/// Sorts before the other drop-ins (`50-cloud-init.conf` ...): sshd keeps the first value it
/// reads for a setting, so this file wins.
const DROPIN: &str = "/etc/ssh/sshd_config.d/00-opsbox.conf";
const MAX_REPORTED: usize = 50;
const MAX_VISITED: usize = 20_000;
const MAX_DEPTH: usize = 5;

fn err(code: PrivErrorCode, message: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}

// ------------------------------------------------------------------ pure helpers

/// A file name that usually holds a secret.
pub fn looks_secret(name: &str) -> bool {
    name.starts_with(".env")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name == "id_rsa"
        || name == "id_ed25519"
}

/// Does the main `sshd_config` read the drop-in folder?
pub fn includes_dropins(main_config: &str) -> bool {
    main_config.lines().any(|l| {
        let l = l.trim().to_ascii_lowercase();
        l.starts_with("include") && l.contains("sshd_config.d")
    })
}

/// `existing` with `key value` set (replacing an earlier line for the same key, keeping
/// everything else).
pub fn merge_dropin(existing: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    if !existing.contains("opsbox") {
        out.push_str("# Managed by opsbox. Settings here take precedence over later files.\n");
    }
    for line in existing.lines() {
        let first = line.split_whitespace().next().unwrap_or("");
        if first.eq_ignore_ascii_case(key) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!("{key} {value}\n"));
    out
}

/// Which setting a check id fixes, and to what.
pub fn ssh_setting(id: &str) -> Option<(&'static str, &'static str)> {
    match id {
        "ssh-password" => Some(("PasswordAuthentication", "no")),
        "ssh-root" => Some(("PermitRootLogin", "no")),
        _ => None,
    }
}

// ------------------------------------------------------------------ scan

fn secrets_readable_by_anyone(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut visited = 0usize;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            visited += 1;
            if visited > MAX_VISITED || found.len() >= MAX_REPORTED {
                return found;
            }
            let Ok(meta) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            let ft = meta.file_type();
            if ft.is_dir() && depth < MAX_DEPTH {
                stack.push((e.path(), depth + 1));
            } else if ft.is_file()
                && meta.permissions().mode() & 0o004 != 0
                && e.file_name().to_str().is_some_and(looks_secret)
            {
                found.push(e.path().to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    found
}

pub(crate) fn scan(apps_dir: &str, master_key: &Path) -> PrivReply {
    let me = geteuid().as_raw();
    let master_key_private = fs::metadata(master_key)
        // Private means: nobody but the owner (who is root in production) can read it.
        .is_ok_and(|m| m.permissions().mode() & 0o077 == 0 && m.uid() == me);
    let sshd_dropins = fs::read_to_string(SSHD_MAIN).is_ok_and(|t| includes_dropins(&t));
    PrivReply::Security(SecurityScan {
        world_readable: secrets_readable_by_anyone(Path::new(apps_dir)),
        master_key_private,
        sshd_dropins,
    })
}

// ------------------------------------------------------------------ fixes

pub(crate) fn fix(id: &str, apps_dir: &str) -> Result<(), PrivError> {
    match id {
        "secrets" => fix_secrets(apps_dir),
        other if ssh_setting(other).is_some() => fix_ssh(other),
        _ => Err(err(
            PrivErrorCode::Invalid,
            "This check has no automatic fix.",
        )),
    }
}

/// Take the "everyone" permission bits off secret files inside the apps folder.
fn fix_secrets(apps_dir: &str) -> Result<(), PrivError> {
    for p in secrets_readable_by_anyone(Path::new(apps_dir)) {
        // Re-check at the moment of the change: it must still be a plain file.
        let Ok(meta) = fs::symlink_metadata(&p) else {
            continue;
        };
        if meta.file_type().is_file() {
            let mode = meta.permissions().mode() & 0o7770;
            fs::set_permissions(&p, fs::Permissions::from_mode(mode)).map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Could not change {p}: {e}"),
                )
            })?;
        }
    }
    Ok(())
}

/// Somebody other than root can still get in: an unlocked sudo account with a key.
fn still_reachable() -> Result<(), PrivError> {
    let admins = access::collect_users()?;
    let logins = access::login_users()?;
    let ok = admins
        .iter()
        .filter(|u| u.sudo && !u.locked && u.uid != 0)
        .any(|u| {
            logins
                .iter()
                .find(|l| l.name == u.name)
                .and_then(|l| access::read_key_file(l).ok())
                .is_some_and(|t| !access::parse_keys(&t).is_empty())
        });
    if ok {
        Ok(())
    } else {
        Err(err(
            PrivErrorCode::Refused,
            "No other account that can use sudo has an SSH key yet. Add a key on the Access page first, so turning this off cannot lock you out.",
        ))
    }
}

fn fix_ssh(id: &str) -> Result<(), PrivError> {
    let (key, value) =
        ssh_setting(id).ok_or_else(|| err(PrivErrorCode::Invalid, "Unknown check."))?;
    if !geteuid().is_root() {
        return Err(err(
            PrivErrorCode::Unsupported,
            "Changing SSH settings needs root.",
        ));
    }
    let sshd = find(&["/usr/sbin/sshd", "/usr/bin/sshd"])
        .ok_or_else(|| err(PrivErrorCode::Unsupported, "No SSH server is installed."))?;
    if !fs::read_to_string(SSHD_MAIN).is_ok_and(|t| includes_dropins(&t)) {
        return Err(err(
            PrivErrorCode::Unsupported,
            format!(
                "This SSH server does not read {DROPIN_DIR}, so opsbox will not edit its main file. Set {key} {value} in {SSHD_MAIN} yourself."
            ),
        ));
    }
    still_reachable()?;

    let previous = fs::read_to_string(DROPIN).ok();
    let merged = merge_dropin(previous.as_deref().unwrap_or(""), key, value);
    write_dropin(&merged)?;
    let undo = |why: String| -> PrivError {
        match &previous {
            Some(text) => {
                let _ = write_dropin(text);
            }
            None => {
                let _ = fs::remove_file(DROPIN);
            }
        }
        err(PrivErrorCode::Internal, why)
    };

    let test = run(&sshd, &["-t"], QUICK).ok_or_else(|| undo("sshd did not run.".into()))?;
    if !test.ok() {
        return Err(undo(format!(
            "The SSH server rejected the change, so it was undone: {}",
            test.combined().trim()
        )));
    }
    // Is it really in effect, or does another file take precedence?
    let effective = run(&sshd, &["-T"], QUICK)
        .filter(super::exec::Output::ok)
        .map(|o| o.stdout);
    let wanted = format!("{} {value}", key.to_ascii_lowercase());
    if effective.is_some_and(|t| !t.lines().any(|l| l.trim() == wanted)) {
        return Err(undo(format!(
            "Another SSH setting overrides this one, so nothing changed. Look for {key} in {DROPIN_DIR} and {SSHD_MAIN}."
        )));
    }
    // Existing connections stay open on a reload.
    let reloaded = ["ssh", "sshd"]
        .iter()
        .any(|unit| run("/usr/bin/systemctl", &["reload", unit], QUICK).is_some_and(|o| o.ok()));
    if !reloaded {
        return Err(err(
            PrivErrorCode::Internal,
            "The setting was saved but the SSH service could not be reloaded. Reload it yourself (systemctl reload ssh).",
        ));
    }
    Ok(())
}

/// Replace the drop-in atomically so sshd never reads half a file.
fn write_dropin(text: &str) -> Result<(), PrivError> {
    let tmp = format!("{DROPIN}.tmp");
    fs::write(&tmp, text)
        .and_then(|()| fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)))
        .and_then(|()| fs::rename(&tmp, DROPIN))
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Could not write {DROPIN}: {e}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_file_names() {
        for yes in [
            ".env",
            ".env.production",
            "server.pem",
            "tls.key",
            "id_rsa",
            "id_ed25519",
        ] {
            assert!(looks_secret(yes), "{yes}");
        }
        for no in [
            "environment.md",
            "keyboard.txt",
            "id_rsa.pub",
            "readme",
            "monkey",
        ] {
            assert!(!looks_secret(no), "{no}");
        }
    }

    #[test]
    fn only_world_readable_secrets_inside_the_apps_folder_are_found() {
        let d = tempfile::tempdir().unwrap();
        let w = |p: &str, mode: u32| {
            let f = d.path().join(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(&f, "x").unwrap();
            fs::set_permissions(&f, fs::Permissions::from_mode(mode)).unwrap();
        };
        w("web/.env", 0o644);
        w("web/.env.local", 0o600);
        w("web/deep/er/tls.key", 0o664);
        w("web/notes.txt", 0o644);
        w("api/.env", 0o640);
        std::os::unix::fs::symlink("/etc/hosts", d.path().join("web/.env.link")).unwrap();
        let found = secrets_readable_by_anyone(d.path());
        let rel: Vec<_> = found
            .iter()
            .map(|p| p.strip_prefix(d.path().to_str().unwrap()).unwrap())
            .collect();
        assert_eq!(rel, ["/web/.env", "/web/deep/er/tls.key"]);
        // The fix removes only the "everyone" bits and keeps the rest.
        fix_secrets(d.path().to_str().unwrap()).unwrap();
        let mode = |p: &str| fs::metadata(d.path().join(p)).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            (
                mode("web/.env"),
                mode("web/deep/er/tls.key"),
                mode("api/.env")
            ),
            (0o640, 0o660, 0o640)
        );
        assert!(secrets_readable_by_anyone(d.path()).is_empty());
        assert!(secrets_readable_by_anyone(&d.path().join("missing")).is_empty());
    }

    #[test]
    fn the_main_config_is_checked_for_the_include() {
        assert!(includes_dropins(
            "Port 22\nInclude /etc/ssh/sshd_config.d/*.conf\n"
        ));
        assert!(includes_dropins("  include /etc/ssh/sshd_config.d/*.conf"));
        assert!(!includes_dropins(
            "# Include /etc/ssh/sshd_config.d/*.conf is commented\nPort 22"
        ));
        assert!(!includes_dropins("Port 22\nPasswordAuthentication yes\n"));
    }

    #[test]
    fn the_dropin_is_merged_not_replaced() {
        let first = merge_dropin("", "PasswordAuthentication", "no");
        assert_eq!(
            first,
            "# Managed by opsbox. Settings here take precedence over later files.\nPasswordAuthentication no\n"
        );
        let both = merge_dropin(&first, "PermitRootLogin", "no");
        assert!(
            both.contains("PasswordAuthentication no\n") && both.ends_with("PermitRootLogin no\n")
        );
        assert_eq!(
            both.matches("# Managed by opsbox").count(),
            1,
            "the header is not repeated"
        );
        // Setting a key again replaces its line, in any letter case.
        let again = merge_dropin(
            "passwordauthentication yes\nPort 22\n",
            "PasswordAuthentication",
            "no",
        );
        assert!(again.contains("Port 22\n") && again.ends_with("PasswordAuthentication no\n"));
        assert_eq!(
            again
                .to_lowercase()
                .matches("passwordauthentication")
                .count(),
            1
        );
    }

    #[test]
    fn unknown_fixes_and_ids_are_refused() {
        assert_eq!(
            fix("firewall", "/nonexistent").unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            fix("; rm", "/nonexistent").unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(ssh_setting("ssh-root"), Some(("PermitRootLogin", "no")));
        assert_eq!(ssh_setting("secrets"), None);
    }
}
