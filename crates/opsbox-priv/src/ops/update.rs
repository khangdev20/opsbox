//! Privileged self-update and rollback operations (spec 7.15).

use std::path::Path;
use std::process::Command;

use opsbox_proto::{PrivError, PrivErrorCode};
use opsbox_update::{SlotLayout, UpdateOutcome, UpdateState};
use rustix::fs::{AtFlags, FileType, Mode, OFlags, fstat, open, openat, unlinkat};

use super::files;

const DEFAULT_DB_PATH: &str = "/var/lib/opsbox/opsbox.db";

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn verify_staged_binary(
    staging: &str,
    xfer_dir: &Path,
) -> Result<(std::os::fd::OwnedFd, String), PrivError> {
    let lexical = files::normalize_lexical(staging)?;
    let xfer_fd = super::managed_dirs::open_dir(xfer_dir, true)?;
    let xfer_st = fstat(&xfer_fd).map_err(|_| {
        err(
            PrivErrorCode::Internal,
            "The transfer folder does not exist.",
        )
    })?;

    let name = lexical
        .file_name()
        .and_then(|n| n.to_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            err(
                PrivErrorCode::Refused,
                "That is not a staged update binary.",
            )
        })?;
    let parent = lexical.parent().unwrap_or(Path::new("/"));
    let parent_fd = open(
        parent,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| {
        err(
            PrivErrorCode::Refused,
            "That is not in the transfer folder.",
        )
    })?;
    let parent_st = fstat(&parent_fd).map_err(|_| {
        err(
            PrivErrorCode::Refused,
            "That is not in the transfer folder.",
        )
    })?;

    if (parent_st.st_dev, parent_st.st_ino) != (xfer_st.st_dev, xfer_st.st_ino) {
        return Err(err(
            PrivErrorCode::Refused,
            "Update binary must be staged inside the transfer folder.",
        ));
    }
    let fd = openat(
        &xfer_fd,
        &name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|e| {
        err(
            PrivErrorCode::NotFound,
            format!("Cannot open staged binary: {e}"),
        )
    })?;
    let st = fstat(&fd).map_err(|e| err(PrivErrorCode::Internal, e.to_string()))?;
    if !FileType::from_raw_mode(st.st_mode as _).is_file() {
        return Err(err(
            PrivErrorCode::Refused,
            "Staged update is not a regular file.",
        ));
    }
    Ok((xfer_fd, name))
}

fn schedule_unit_restart() -> Result<(), PrivError> {
    // Run via a transient unit so `priv` can reply over the socket before systemd stops it.
    let _ = Command::new("systemctl")
        .args(["reset-failed", "opsbox-restart.service"])
        .output();
    let out = Command::new("systemd-run")
        .args([
            "--unit=opsbox-restart",
            "--no-block",
            "--description=Restart opsbox after update",
            "/bin/sh",
            "-c",
            "sleep 0.5; systemctl restart opsbox-priv.service; if systemctl cat opsbox-proxy.service >/dev/null 2>&1; then systemctl restart opsbox-proxy.service; fi; systemctl restart opsbox.service",
        ])
        .output()
        .map_err(|e| err(PrivErrorCode::Internal, format!("systemd-run failed: {e}")))?;
    if !out.status.success() {
        return Err(err(
            PrivErrorCode::Internal,
            format!(
                "Cannot schedule service restart: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ));
    }
    Ok(())
}

fn watchdog_is_active() -> bool {
    Path::new("/run/systemd/system").is_dir()
        && Command::new("systemctl")
            .args(["is-active", "--quiet", "opsbox-watchdog.service"])
            .status()
            .is_ok_and(|s| s.success())
}

fn clear_transient_rollback_check() {
    let _ = Command::new("systemctl")
        .args([
            "stop",
            "opsbox-rollback-check.timer",
            "opsbox-rollback-check.service",
        ])
        .output();
    let _ = Command::new("systemctl")
        .args(["reset-failed", "opsbox-rollback-check.service"])
        .output();
}

fn schedule_rollback_check(layout: &SlotLayout, from: Option<&str>, to: &str) {
    clear_transient_rollback_check();
    let checker_bin = from
        .map(|v| layout.slot_path(v))
        .filter(|p| p.exists())
        .unwrap_or_else(|| layout.symlink.clone());
    let mut args = vec![
        "--unit=opsbox-rollback-check".to_owned(),
        "--on-active=120s".to_owned(),
        "--description=opsbox update health check and automatic rollback".to_owned(),
        checker_bin.to_string_lossy().into_owned(),
        "update-verify".to_owned(),
        "--to".to_owned(),
        to.to_owned(),
    ];
    if let Some(prev) = from {
        args.push("--from".to_owned());
        args.push(prev.to_owned());
    }
    if let Err(e) = Command::new("systemd-run").args(&args).output() {
        tracing::warn!(error = %e, "failed to schedule opsbox-rollback-check timer");
    }
}

pub(super) fn install(
    staging: &str,
    version: &str,
    target_schema: u32,
    apps_dir: &Path,
    dirs: &files::Dirs,
) -> Result<(), PrivError> {
    super::managed_dirs::prepare(apps_dir, &dirs.xfer, &dirs.trash)?;
    let (xfer_fd, staged_name) = verify_staged_binary(staging, &dirs.xfer)?;
    let staged_path = Path::new(staging);

    let selftest = Command::new(staged_path)
        .arg("selftest")
        .output()
        .map_err(|e| err(PrivErrorCode::Internal, format!("Cannot run selftest: {e}")))?;
    if !selftest.status.success() {
        return Err(err(
            PrivErrorCode::Invalid,
            format!(
                "New binary failed self-test: {}",
                String::from_utf8_lossy(&selftest.stderr).trim()
            ),
        ));
    }

    let layout = SlotLayout::default();
    let current_ver = crate::VERSION;
    let prev = layout
        .install_and_switch(
            staged_path,
            current_ver,
            opsbox_store::LATEST_VERSION,
            version,
            target_schema,
        )
        .map_err(|e| err(PrivErrorCode::Internal, e.to_string()))?;

    let _ = unlinkat(&xfer_fd, &staged_name, AtFlags::empty());

    let clean_to = version.trim_start_matches('v').to_owned();
    let from_ver = prev.clone().unwrap_or_else(|| current_ver.to_owned());
    let watchdog_active = watchdog_is_active();
    let mut state: UpdateState = layout.read_state();
    state.last_outcome = Some(UpdateOutcome {
        status: "pending".to_owned(),
        from_version: from_ver,
        to_version: clean_to.clone(),
        message: if watchdog_active {
            format!("Switched to {clean_to}; watchdog is monitoring health for automatic rollback.")
        } else {
            format!("Switched to {clean_to}; health check scheduled in 2 minutes.")
        },
        at: now_secs(),
        audited: false,
    });
    let _ = layout.write_state(&state);

    if watchdog_active {
        // Watchdog owns post-update health checks and rollback once running.
        clear_transient_rollback_check();
        tracing::info!(
            to = %clean_to,
            from = ?prev,
            "opsbox-watchdog.service is active; skipping transient opsbox-rollback-check unit"
        );
    } else {
        schedule_rollback_check(&layout, prev.as_deref(), &clean_to);
    }
    schedule_unit_restart()?;
    Ok(())
}

pub(super) fn rollback(to_version: Option<&str>) -> Result<(), PrivError> {
    let layout = SlotLayout::default();
    let db_path = Path::new(DEFAULT_DB_PATH);
    let current_db_schema = if db_path.exists() {
        opsbox_store::db_schema_version(db_path)
            .map_err(|e| err(PrivErrorCode::Internal, e.to_string()))?
    } else {
        0
    };
    let prev_state = layout.read_state();
    let from_ver = prev_state
        .current_version
        .clone()
        .unwrap_or_else(|| crate::VERSION.to_owned());

    let rolled = layout
        .rollback_slot(to_version, current_db_schema)
        .map_err(|e| err(PrivErrorCode::Refused, e.to_string()))?;

    let mut state = layout.read_state();
    state.last_outcome = Some(UpdateOutcome {
        status: "rolled_back".to_owned(),
        from_version: from_ver.clone(),
        to_version: rolled.clone(),
        message: format!("Rolled back from {from_ver} to {rolled}."),
        at: now_secs(),
        audited: false,
    });
    let _ = layout.write_state(&state);

    schedule_unit_restart()?;
    Ok(())
}
