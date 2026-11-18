//! `opsbox backup list` and `opsbox backup now` CLI commands.

use anyhow::{Context, Result, bail};
use opsbox_api::backups::{
    build_run_spec, next_run_after_safe, now_unix, sync_restic_snapshots_to_store,
};
use opsbox_backup::cron::format_utc_short;
use opsbox_priv::PrivClient;
use opsbox_proto::{BackupCheckState, BackupSnapshotsRequest};
use opsbox_store::{LOCAL_DESTINATION_ID, Store};

use crate::config::Config;

pub fn list(config: &Config) -> Result<()> {
    let store = Store::open(&config.db_path).with_context(|| {
        format!(
            "cannot open database at {} (is opsbox installed?)",
            config.db_path.display()
        )
    })?;
    let dir_str = config.backup_dir.display().to_string();
    let now = now_unix();
    let _ = store.ensure_local_destination_path(&dir_str, now);
    let jobs = store.list_backup_jobs()?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start async runtime")?;
    let client = PrivClient::new(&config.priv_socket);

    let local_info = rt
        .block_on(client.backup_local_info("cli-backup-list", Some("root"), Vec::new()))
        .ok();
    if let Ok(snaps) = rt.block_on(client.backup_snapshots(
        "cli-backup-list",
        Some("root"),
        BackupSnapshotsRequest {
            destination_id: LOCAL_DESTINATION_ID.to_owned(),
        },
    )) {
        let _ = sync_restic_snapshots_to_store(&store, &snaps, None);
    }

    let snapshots = store.list_backup_snapshots(50)?;

    println!("Repository: {}", config.backup_dir.display());
    if let Some(ref info) = local_info {
        let ver = info.restic_version.as_deref().unwrap_or("not installed");
        let state = if info.repo_initialized {
            "initialized"
        } else {
            "uninitialized"
        };
        println!("Restic:     {ver} ({state})");
        if info.same_disk {
            println!(
                "Warning:    local backups live on the same disk as your data ({})",
                info.same_disk_paths.join(", ")
            );
        }
    } else {
        println!("Restic:     privileged helper unavailable (showing stored SQLite metadata)");
    }

    println!("\nSchedules:");
    if jobs.is_empty() {
        println!("  (none)");
    } else {
        for j in &jobs {
            let status = j.last_status.as_deref().unwrap_or("never");
            let state = if j.enabled { "enabled" } else { "disabled" };
            println!(
                "  {:<26} {:<9} {:<16} {:<12} {} ({})",
                j.id,
                j.kind.as_str(),
                j.name,
                j.cron,
                status,
                state
            );
        }
    }

    println!("\nSnapshots:");
    if snapshots.is_empty() {
        println!("  (none)");
    } else {
        for s in &snapshots {
            println!(
                "  {}  {}  {:<9} {:<24} {:>10} B  {}",
                s.short_id,
                format_utc_short(s.taken_at),
                s.kind.as_str(),
                s.what,
                s.size_bytes,
                s.check_state.display_label()
            );
        }
    }
    Ok(())
}

pub fn now(config: &Config, job_id: Option<&str>, prune: bool, check: bool) -> Result<bool> {
    let store = Store::open(&config.db_path).with_context(|| {
        format!(
            "cannot open database at {} (is opsbox installed?)",
            config.db_path.display()
        )
    })?;
    let dir_str = config.backup_dir.display().to_string();
    let ts = now_unix();
    let _ = store.ensure_local_destination_path(&dir_str, ts);

    let jobs = if let Some(jid) = job_id {
        let Some(job) = store.get_backup_job(jid)? else {
            bail!("backup schedule '{jid}' does not exist");
        };
        vec![job]
    } else {
        store
            .list_backup_jobs()?
            .into_iter()
            .filter(|j| j.enabled)
            .collect()
    };

    if jobs.is_empty() {
        println!("No enabled backup schedules found.");
        return Ok(true);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("cannot start async runtime")?;
    let client = PrivClient::new(&config.priv_socket);
    let mut all_ok = true;

    for job in &jobs {
        let start = now_unix();
        let _ = store.mark_backup_job_running(&job.id, start);
        let spec = match build_run_spec(&store, job, prune, check) {
            Ok(s) => s,
            Err(e) => {
                all_ok = false;
                let msg = e.0.error.message;
                let finish = now_unix();
                let next_run = next_run_after_safe(&job.cron, finish);
                let _ = store.record_backup_job_result(
                    &job.id,
                    false,
                    finish,
                    None,
                    None,
                    Some(0),
                    Some(&msg),
                    next_run,
                );
                eprintln!("{}: failed: {msg}", job.name);
                continue;
            }
        };

        let rid = format!("cli-backup-{}", job.id);
        match rt.block_on(client.backup_run(&rid, Some("root"), spec)) {
            Ok(res) => {
                let finish = res.taken_at.max(now_unix());
                let next_run = next_run_after_safe(&job.cron, finish);
                let ok = res.check_state != BackupCheckState::ChecksumFailed;
                if !ok {
                    all_ok = false;
                }
                let _ = store.record_backup_job_result(
                    &job.id,
                    ok,
                    finish,
                    Some(&res.snapshot_id),
                    Some(i64::try_from(res.size_bytes).unwrap_or(i64::MAX)),
                    Some(i64::try_from(res.duration_ms).unwrap_or(i64::MAX)),
                    res.check_error.as_deref(),
                    next_run,
                );
                println!(
                    "{}: snapshot {} ({} B, {} ms)",
                    job.name, res.short_id, res.size_bytes, res.duration_ms
                );
            }
            Err(e) => {
                all_ok = false;
                let finish = now_unix();
                let next_run = next_run_after_safe(&job.cron, finish);
                let msg = e.to_string();
                let _ = store.record_backup_job_result(
                    &job.id,
                    false,
                    finish,
                    None,
                    None,
                    Some((finish - start).max(0) * 1000),
                    Some(&msg),
                    next_run,
                );
                eprintln!("{}: failed: {msg}", job.name);
            }
        }
    }

    if let Ok(snaps) = rt.block_on(client.backup_snapshots(
        "cli-backup-sync",
        Some("root"),
        BackupSnapshotsRequest {
            destination_id: LOCAL_DESTINATION_ID.to_owned(),
        },
    )) {
        let _ = sync_restic_snapshots_to_store(&store, &snaps, None);
    }

    Ok(all_ok)
}
