//! Panel listener configuration (`opsbox web local-only on|off` from the UI).

use std::fs;
use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use nix::unistd::geteuid;
use opsbox_proto::{PrivError, PrivErrorCode};

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

/// Update or add `[api]\nlisten = "..."` in `raw` TOML text, preserving any custom port
/// while switching the bind address between loopback (`local_only = true`) and public (`false`).
pub fn set_listen_in_toml(raw: &str, local_only: bool) -> String {
    let existing_listen = extract_listen(raw);
    let target_listen = target_addr(existing_listen.as_deref(), local_only);
    replace_listen_in_toml(raw, &target_listen)
}

fn target_addr(existing: Option<&str>, local_only: bool) -> String {
    if let Some(raw) = existing
        && let Ok(addr) = raw.parse::<SocketAddr>()
    {
        let port = addr.port();
        return match (addr.is_ipv6(), local_only) {
            (true, true) => format!("[::1]:{port}"),
            (true, false) => format!("[::]:{port}"),
            (false, true) => format!("127.0.0.1:{port}"),
            (false, false) => format!("0.0.0.0:{port}"),
        };
    }
    if local_only {
        "127.0.0.1:8443".to_owned()
    } else {
        "0.0.0.0:8443".to_owned()
    }
}

fn extract_listen(raw: &str) -> Option<String> {
    let mut in_api = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_api = trimmed == "[api]";
            continue;
        }
        if in_api
            && !trimmed.starts_with('#')
            && let Some((k, v)) = trimmed.split_once('=')
            && k.trim() == "listen"
        {
            let val = v.split('#').next().unwrap_or(v).trim();
            let unquoted = val.trim_matches('"').trim_matches('\'');
            if !unquoted.is_empty() {
                return Some(unquoted.to_owned());
            }
        }
    }
    None
}

/// Replace `[api] listen = "..."` in `raw` with `listen`, inserting `[api]` if absent.
pub fn replace_listen_in_toml(raw: &str, listen: &str) -> String {
    let mut out = Vec::new();
    let mut in_api = false;
    let mut saw_api = false;
    let mut replaced = false;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            if in_api && !replaced {
                out.push(format!("listen = \"{listen}\""));
                replaced = true;
            }
            in_api = trimmed == "[api]";
            if in_api {
                saw_api = true;
            }
            out.push(line.to_owned());
            continue;
        }
        if in_api
            && !trimmed.starts_with('#')
            && trimmed.starts_with("listen")
            && let Some((k, _)) = trimmed.split_once('=')
            && k.trim() == "listen"
        {
            out.push(format!("listen = \"{listen}\""));
            replaced = true;
            continue;
        }
        out.push(line.to_owned());
    }

    if in_api && !replaced {
        out.push(format!("listen = \"{listen}\""));
        replaced = true;
    }
    if !saw_api && !replaced {
        if !out.is_empty() && !out.last().is_some_and(String::is_empty) {
            out.push(String::new());
        }
        out.push("[api]".to_owned());
        out.push(format!("listen = \"{listen}\""));
    }

    let mut result = out.join("\n");
    result.push('\n');
    result
}

pub(crate) fn set_local_only(config_path: &Path, local_only: bool) -> Result<(), PrivError> {
    if !geteuid().is_root() || !Path::new("/run/systemd/system").is_dir() {
        return Err(err(
            PrivErrorCode::Unsupported,
            "Changing the panel listener requires root on a systemd host.",
        ));
    }
    let existing = fs::read_to_string(config_path).unwrap_or_default();
    let updated = set_listen_in_toml(&existing, local_only);
    if let Some(parent) = config_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| err(PrivErrorCode::Internal, e.to_string()))?;
    }
    let tmp = config_path.with_extension("toml.tmp");
    fs::write(&tmp, updated)
        .and_then(|()| fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)))
        .and_then(|()| fs::rename(&tmp, config_path))
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("Cannot update {}: {e}", config_path.display()),
            )
        })?;

    // Schedule via transient unit so `priv` can reply and `serve` can return 200 to the browser first.
    let _ = Command::new("systemctl")
        .args(["reset-failed", "opsbox-web-restart.service"])
        .output();
    let out = Command::new("systemd-run")
        .args([
            "--unit=opsbox-web-restart",
            "--no-block",
            "--description=Restart opsbox after listener change",
            "/bin/sh",
            "-c",
            "sleep 0.5; systemctl restart opsbox.service; systemctl try-restart opsbox-watchdog.service",
        ])
        .output()
        .map_err(|e| err(PrivErrorCode::Internal, format!("systemd-run failed: {e}")))?;
    if !out.status.success() {
        return Err(err(
            PrivErrorCode::Internal,
            format!(
                "Configuration saved, but could not schedule opsbox restart: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_listen_and_preserves_custom_port() {
        assert_eq!(
            set_listen_in_toml("", true),
            "[api]\nlisten = \"127.0.0.1:8443\"\n"
        );
        assert_eq!(
            set_listen_in_toml("[api]\nlisten = \"0.0.0.0:9443\"\n\n[store]\n", true),
            "[api]\nlisten = \"127.0.0.1:9443\"\n\n[store]\n"
        );
        assert_eq!(
            set_listen_in_toml("[api]\nlisten = \"127.0.0.1:9443\"\n", false),
            "[api]\nlisten = \"0.0.0.0:9443\"\n"
        );
        assert_eq!(
            set_listen_in_toml("[api]\nlisten = \"[::]:8443\"\n", true),
            "[api]\nlisten = \"[::1]:8443\"\n"
        );
    }

    #[test]
    fn refuses_non_root_or_non_systemd() {
        if !geteuid().is_root() || !Path::new("/run/systemd/system").is_dir() {
            let dir = tempfile::tempdir().unwrap();
            let err = set_local_only(&dir.path().join("config.toml"), true).unwrap_err();
            assert_eq!(err.code, PrivErrorCode::Unsupported);
        }
    }
}
