//! Installation migration also run by the new helper after a binary-only update.
use std::io;
use std::path::Path;
use std::process::Command;

const INSTALLER: &str = include_str!("../../../install.sh");
const PROXY_UNIT: &str = include_str!("../../../packaging/systemd/opsbox-proxy.service");

/// Create the separate account and migrate only proxy state. Root startup calls
/// this before listening, so even updates initiated by an older helper migrate.
pub fn prepare_proxy_identity(state_dir: &Path) -> io::Result<()> {
    if !nix::unistd::geteuid().is_root() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "proxy migration requires root",
        ));
    }
    let script = INSTALLER
        .split("# BEGIN proxy-identity\n")
        .nth(1)
        .and_then(|s| s.split("# END proxy-identity").next())
        .expect("installer has the shared proxy migration");
    let script = format!(
        r#"set -eu
run() {{ "$@"; }}
die() {{ echo "$*" >&2; exit 1; }}
{script}"#
    );
    let output = Command::new("/bin/sh")
        .args(["-c", &script])
        .env("DRY_RUN", "0")
        .env("PROXY_USER", "opsbox-proxy")
        .env("STATE_DIR", "/var/lib/opsbox")
        .env("PROXY_STATE_DIR", state_dir)
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    // Old update helpers switch binaries, but do not install unit files. Refresh
    // the proxy unit before their queued restart. Do not create units in dev runs.
    let unit_path = Path::new("/etc/systemd/system/opsbox-proxy.service");
    if unit_path.exists() && std::fs::read_to_string(unit_path)? != PROXY_UNIT {
        // The containing directory is root-owned; avoid following a unit symlink.
        let temp = unit_path.with_extension(format!("service.opsbox-new-{}", std::process::id()));
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&temp)?;
        file.write_all(PROXY_UNIT.as_bytes())?;
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o644))?;
        file.sync_all()?;
        std::fs::rename(temp, unit_path)?;
        if !Command::new("systemctl")
            .arg("daemon-reload")
            .status()?
            .success()
        {
            return Err(io::Error::other("systemctl daemon-reload failed"));
        }
        // Type=simple lets an older updater's restart command return before
        // this bootstrap finishes. Queue our own restart after installing the
        // new unit, so it cannot leave legacy writers stopped or on the old UID.
        if !Command::new("systemctl")
            .args([
                "restart",
                "--no-block",
                "opsbox-proxy.service",
                "opsbox.service",
            ])
            .status()?
            .success()
        {
            return Err(io::Error::other(
                "cannot restart services after proxy migration",
            ));
        }
    }
    Ok(())
}
