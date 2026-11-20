//! Privileged self-update and rollback operations (spec 7.15).

use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
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

fn open_staged_archive(
    staging: &str,
    xfer_dir: &Path,
) -> Result<(std::os::fd::OwnedFd, String, std::fs::File), PrivError> {
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
                "That is not a staged update archive.",
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
            "Update archive must be staged inside the transfer folder.",
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
            format!("Cannot open staged archive: {e}"),
        )
    })?;
    let st = fstat(&fd).map_err(|e| err(PrivErrorCode::Internal, e.to_string()))?;
    if !FileType::from_raw_mode(st.st_mode as _).is_file() {
        return Err(err(
            PrivErrorCode::Refused,
            "Staged update is not a regular file.",
        ));
    }
    Ok((xfer_fd, name, std::fs::File::from(fd)))
}

fn prepare_update(
    staging: &str,
    xfer: &Path,
    version: &str,
    sha256: &str,
    minisign: &str,
) -> Result<(std::os::fd::OwnedFd, String, tempfile::TempPath), PrivError> {
    let (xfer_fd, name, mut file) = open_staged_archive(staging, xfer)?;
    // Pin the opened inode and authenticate exactly these bytes. Never reopen xfer by path.
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| err(PrivErrorCode::Internal, e.to_string()))?;
    opsbox_update::verify_sha256(&bytes, sha256)
        .map_err(|e| err(PrivErrorCode::Refused, e.to_string()))?;
    opsbox_update::verify_minisign(&bytes, minisign, opsbox_update::MINISIGN_PUBKEY)
        .map_err(|e| err(PrivErrorCode::Refused, e.to_string()))?;
    let binary = opsbox_update::extract_release_binary(
        &bytes,
        version,
        opsbox_update::current_target().map_err(|e| err(PrivErrorCode::Refused, e.to_string()))?,
    )
    .map_err(|e| err(PrivErrorCode::Refused, e.to_string()))?;
    let staged = private_binary(&binary)?;
    let selftest = Command::new(&staged)
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
    Ok((xfer_fd, name, staged))
}

fn private_binary(bytes: &[u8]) -> Result<tempfile::TempPath, PrivError> {
    let io_err = |e: std::io::Error| err(PrivErrorCode::Internal, e.to_string());
    // tempfile uses exclusive creation. The helper's uid is root in production;
    // 0700 excludes the panel even when it knows this unpredictable temp path.
    let mut file = tempfile::Builder::new()
        .prefix("opsbox-verified-")
        .tempfile()
        .map_err(io_err)?;
    file.as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o700))
        .map_err(io_err)?;
    file.write_all(bytes).map_err(io_err)?;
    file.as_file().sync_all().map_err(io_err)?;
    // Close the writable fd before exec (Linux ETXTBSY), retain automatic cleanup.
    Ok(file.into_temp_path())
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
    verification: &opsbox_proto::UpdateVerification,
    apps_dir: &Path,
    dirs: &files::Dirs,
) -> Result<(), PrivError> {
    super::managed_dirs::prepare(apps_dir, &dirs.xfer, &dirs.trash)?;
    let (xfer_fd, staged_name, staged) = prepare_update(
        staging,
        &dirs.xfer,
        version,
        &verification.sha256,
        &verification.minisign,
    )?;
    let staged_path: &Path = staged.as_ref();

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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn verified_copy_survives_staging_replacement_and_keeps_rollback() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempfile::tempdir().unwrap();
        let staged = tmp.path().join("staged");
        let trusted = b"#!/bin/sh\n[ \"$1\" = selftest ]\n";
        std::fs::write(&staged, trusted).unwrap();
        let copy = private_binary(&std::fs::read(&staged).unwrap()).unwrap();
        std::fs::write(&staged, b"replaced by caller").unwrap();
        let meta = std::fs::metadata(&copy).unwrap();
        assert_eq!(meta.uid(), nix::unistd::geteuid().as_raw());
        assert_eq!(meta.mode() & 0o7777, 0o700);
        assert!(
            Command::new(&copy)
                .arg("selftest")
                .status()
                .unwrap()
                .success()
        );
        let layout = SlotLayout {
            slots_dir: tmp.path().join("slots"),
            symlink: tmp.path().join("opsbox"),
            state_file: tmp.path().join("state.json"),
        };
        std::fs::write(&layout.symlink, b"old binary").unwrap();
        layout
            .install_and_switch(&copy, "0.1.0", 1, "0.2.0", 1)
            .unwrap();
        assert_eq!(std::fs::read(&layout.symlink).unwrap(), trusted);
        layout.rollback_slot(None, 1).unwrap();
        assert_eq!(std::fs::read(&layout.symlink).unwrap(), b"old binary");
        let path = copy.to_path_buf();
        drop(copy);
        assert!(!path.exists());
    }

    #[test]
    fn unsigned_update_never_runs_selftest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let staged = root.join("unsigned.bin");
        let marker = root.join("selftest-ran");
        let script = format!("#!/bin/sh\nprintf ran > '{}'\nexit 0\n", marker.display());
        std::fs::write(&staged, &script).unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
        let checksum = Sha256::digest(script.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let result = prepare_update(staged.to_str().unwrap(), &root, "0.1.0", &checksum, "");
        assert!(!marker.exists(), "unsigned update executed selftest");
        let error = result.unwrap_err();
        assert_eq!(error.code, PrivErrorCode::Refused);
        assert!(error.message.contains("signature verification failed"));
    }

    #[test]
    fn modified_update_never_runs_selftest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let staged = root.join("modified.bin");
        let marker = root.join("selftest-ran");
        std::fs::write(
            &staged,
            format!("#!/bin/sh\nprintf ran > '{}'\n", marker.display()),
        )
        .unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
        let checksum = Sha256::digest(b"original signed bytes")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let result = prepare_update(
            staged.to_str().unwrap(),
            &root,
            "0.1.0",
            &checksum,
            "invalid signature",
        );
        assert!(!marker.exists(), "modified update executed selftest");
        let error = result.unwrap_err();
        assert_eq!(error.code, PrivErrorCode::Refused);
        assert!(error.message.contains("checksum verification failed"));
    }
}
