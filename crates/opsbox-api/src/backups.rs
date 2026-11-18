//! Backup orchestration, cron scheduler, retention/checksum triggers, and API handlers
//! (docs/BACKEND_SPEC.md 7.12).
//!
//! `serve` schedules jobs, validates inputs, manages metadata in SQLite, and surfaces
//! Attention alerts; every `restic`, container dump, `/etc/opsbox`, and `/var/backups/opsbox`
//! operation runs inside `opsbox-priv`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use opsbox_auth::Principal;
use opsbox_backup::BackupError;
use opsbox_backup::cron::{
    CronSchedule, format_utc_short, human_label, is_run_missed, validate_cron,
};
use opsbox_backup::validation::{
    validate_job_name, validate_path_list, validate_read_data_subset, validate_retention,
    validate_volume_list,
};
use opsbox_deploy::templates::get_template;
use opsbox_metrics::Attention;
use opsbox_priv::{ClientError, PrivClient};
use opsbox_proto::{
    BackupCheckRequest, BackupCheckState, BackupDbKind, BackupForgetRequest, BackupLocalInfo,
    BackupRunSpec, BackupRunTarget, BackupSnapshotsRequest, BackupTargetKind, DeployEnvSpec,
    ErrorCode, PrivErrorCode, ResticSnapshotInfo, Role, valid_backup_id,
};
use opsbox_store::{
    BackupDestinationRecord, BackupJobRecord, BackupSnapshotRecord, LOCAL_DESTINATION_ID,
    NewBackupJob, OPSBOX_SELF_JOB_ID, ServiceKind, Store, SyncBackupSnapshot, UpdateBackupJob,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::deploy::var_seal_context;
use crate::json::ApiJson;
use crate::session::{Admin, Client, Viewer};
use crate::system::audit;
use crate::tasks::{Outcome, Tasks};
use crate::{AppState, HttpError, RequestId};

const SCHEDULER_POLL_SECS: u64 = 30;
const MISSED_RUN_GRACE_SECS: i64 = 3600;
const DEFAULT_SNAPSHOT_LIMIT: usize = 200;

/// Background backup serializer + cron loop attached to [`AppState`].
#[derive(Clone)]
pub struct BackupRunner {
    backup_dir: Arc<std::sync::Mutex<PathBuf>>,
    run_lock: Arc<tokio::sync::Mutex<()>>,
    scheduler_started: Arc<AtomicBool>,
}

impl Default for BackupRunner {
    fn default() -> Self {
        Self {
            backup_dir: Arc::new(std::sync::Mutex::new(PathBuf::from("/var/backups/opsbox"))),
            run_lock: Arc::new(tokio::sync::Mutex::new(())),
            scheduler_started: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl BackupRunner {
    pub fn backup_dir(&self) -> PathBuf {
        self.backup_dir
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_backup_dir(&self, dir: impl Into<PathBuf>) {
        *self
            .backup_dir
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = dir.into();
    }

    /// Start the once-per-30s cron scheduler loop (no-op in tests when
    /// `OPSBOX_DISABLE_BG_BACKUPS` is set or no Tokio runtime is active).
    pub fn spawn_scheduler(&self, store: Arc<Store>, priv_client: PrivClient, tasks: Tasks) {
        if self.scheduler_started.swap(true, Ordering::AcqRel) {
            return;
        }
        if std::env::var_os("OPSBOX_DISABLE_BG_BACKUPS").is_some()
            || tokio::runtime::Handle::try_current().is_err()
        {
            return;
        }
        let runner = self.clone();
        tokio::spawn(async move {
            let init_store = store.clone();
            let dir_str = runner.backup_dir().display().to_string();
            let _ = tokio::task::spawn_blocking(move || {
                let ts = now_unix();
                let _ = init_store.ensure_local_destination_path(&dir_str, ts);
                initialize_missing_next_runs(&init_store, ts);
            })
            .await;

            loop {
                tokio::time::sleep(Duration::from_secs(SCHEDULER_POLL_SECS)).await;
                let s = store.clone();
                let ts = now_unix();
                let due = tokio::task::spawn_blocking(move || {
                    initialize_missing_next_runs(&s, ts);
                    s.list_due_backup_jobs(ts).unwrap_or_default()
                })
                .await
                .unwrap_or_default();

                if due.is_empty() {
                    continue;
                }

                let pc = priv_client.clone();
                let r = runner.clone();
                let st = store.clone();
                let _ = tasks.start("backups.scheduled", async move {
                    let mut log = Vec::new();
                    let mut ran = 0u32;
                    let mut failed = 0u32;
                    for job in due {
                        let rid = format!("sched-backup-{}", job.id);
                        match r
                            .execute_job(
                                &st,
                                &pc,
                                &rid,
                                None,
                                &job,
                                RunOptionsBody {
                                    run_prune: true,
                                    run_check: job.id == OPSBOX_SELF_JOB_ID,
                                },
                            )
                            .await
                        {
                            Ok(msg) => {
                                ran += 1;
                                log.push(msg);
                            }
                            Err(e) => {
                                failed += 1;
                                log.push(format!("{}: failed: {}", job.name, e.0.error.message));
                                warn!(job = %job.id, error = %e.0.error.message, "scheduled backup failed");
                            }
                        }
                    }
                    sync_all_snapshots(&st, &pc, "sched-sync", None, None).await;
                    if failed > 0 && ran == 0 {
                        return Err(HttpError::new(
                            ErrorCode::Internal,
                            format!("{failed} scheduled backup job(s) failed."),
                        ));
                    }
                    Ok(Outcome {
                        log,
                        result: json!({ "ran": ran, "failed": failed }),
                    })
                });
            }
        });
    }

    async fn execute_job(
        &self,
        store: &Arc<Store>,
        priv_client: &PrivClient,
        rid: &str,
        actor: Option<&str>,
        job: &BackupJobRecord,
        opts: RunOptionsBody,
    ) -> Result<String, HttpError> {
        let _guard = self.run_lock.lock().await;
        let start_ts = now_unix();
        let st = store.clone();
        let jid = job.id.clone();
        let _ =
            tokio::task::spawn_blocking(move || st.mark_backup_job_running(&jid, start_ts)).await;

        let st_build = store.clone();
        let job_clone = job.clone();
        let spec_res = tokio::task::spawn_blocking(move || {
            build_run_spec(&st_build, &job_clone, opts.run_prune, opts.run_check)
        })
        .await
        .map_err(|_| HttpError::internal())?;

        let spec = match spec_res {
            Ok(s) => s,
            Err(e) => {
                let finish_ts = now_unix();
                let next_run = next_run_after_safe(&job.cron, finish_ts);
                let st_err = store.clone();
                let jid = job.id.clone();
                let msg = e.0.error.message.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    st_err.record_backup_job_result(
                        &jid,
                        false,
                        finish_ts,
                        None,
                        None,
                        Some(0),
                        Some(&msg),
                        next_run,
                    )
                })
                .await;
                return Err(e);
            }
        };

        match priv_client.backup_run(rid, actor, spec).await {
            Ok(res) => {
                let finish_ts = res.taken_at.max(now_unix());
                let next_run = next_run_after_safe(&job.cron, finish_ts);
                let st_ok = store.clone();
                let jid = job.id.clone();
                let snap_id = res.snapshot_id.clone();
                let size_i64 = i64::try_from(res.size_bytes).unwrap_or(i64::MAX);
                let dur_i64 = i64::try_from(res.duration_ms).unwrap_or(i64::MAX);
                let check_state = res.check_state;
                let check_err = res.check_error.clone();
                let status_str = if check_state == BackupCheckState::ChecksumFailed {
                    "failed"
                } else {
                    "ok"
                };
                let err_msg = if status_str == "failed" {
                    check_err.clone()
                } else {
                    None
                };
                let _ = tokio::task::spawn_blocking(move || {
                    st_ok.record_backup_job_result(
                        &jid,
                        check_state != BackupCheckState::ChecksumFailed,
                        finish_ts,
                        Some(&snap_id),
                        Some(size_i64),
                        Some(dur_i64),
                        err_msg.as_deref(),
                        next_run,
                    )
                })
                .await;
                let mut line = format!(
                    "{}: snapshot {} ({})",
                    job.name,
                    res.short_id,
                    human_bytes(res.size_bytes)
                );
                if res.pruned_count > 0 {
                    line.push_str(&format!(", pruned {}", res.pruned_count));
                }
                if let Some(ref cerr) = check_err {
                    line.push_str(&format!("; check failed: {cerr}"));
                }
                Ok(line)
            }
            Err(e) => {
                let http_err = HttpError::from(e);
                let finish_ts = now_unix();
                let next_run = next_run_after_safe(&job.cron, finish_ts);
                let st_err = store.clone();
                let jid = job.id.clone();
                let msg = http_err.0.error.message.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    st_err.record_backup_job_result(
                        &jid,
                        false,
                        finish_ts,
                        None,
                        None,
                        Some((finish_ts - start_ts).max(0) * 1000),
                        Some(&msg),
                        next_run,
                    )
                })
                .await;
                Err(http_err)
            }
        }
    }
}

pub fn now_unix() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    )
    .unwrap_or(0)
}

fn map_val_err(e: BackupError) -> HttpError {
    HttpError::new(ErrorCode::Validation, e.to_string())
}

fn store_err(_: opsbox_store::StoreError) -> HttpError {
    HttpError::internal()
}

pub fn next_run_after_safe(cron: &str, now: i64) -> Option<i64> {
    CronSchedule::parse(cron)
        .ok()
        .and_then(|s| s.next_run_after(now))
}

fn initialize_missing_next_runs(store: &Store, now: i64) {
    let Ok(jobs) = store.list_backup_jobs() else {
        return;
    };
    for job in jobs {
        if job.enabled
            && job.next_run_at.is_none()
            && let Some(next) = next_run_after_safe(&job.cron, now)
        {
            let _ = store.set_backup_job_next_run(&job.id, Some(next), now);
        }
    }
}

fn store_env_specs(store: &Store, service_id: &str) -> Vec<DeployEnvSpec> {
    store
        .list_service_vars(service_id)
        .unwrap_or_default()
        .into_iter()
        .map(|v| DeployEnvSpec {
            seal_context: var_seal_context(service_id, &v.key),
            key: v.key,
            value_enc: v.value_enc,
            is_secret: v.is_secret,
        })
        .collect()
}

fn infer_db_kind(job: &BackupJobRecord) -> Result<BackupDbKind, HttpError> {
    let tmpl = job
        .template_id
        .as_deref()
        .or_else(|| job.what.get("templateId").and_then(Value::as_str));
    if let Some(t) = tmpl {
        match t {
            "postgres" => return Ok(BackupDbKind::Postgres),
            "mariadb" => return Ok(BackupDbKind::Mariadb),
            "mysql" => return Ok(BackupDbKind::Mysql),
            "redis" => return Ok(BackupDbKind::Redis),
            "mongo" => return Ok(BackupDbKind::Mongo),
            _ => {}
        }
    }
    match job.method.as_str() {
        "pg_dump" => Ok(BackupDbKind::Postgres),
        "mysqldump" => Ok(BackupDbKind::Mysql),
        "mariadb-dump" => Ok(BackupDbKind::Mariadb),
        "redis-cli --rdb" | "redis-cli" | "rdb" => Ok(BackupDbKind::Redis),
        "mongodump" => Ok(BackupDbKind::Mongo),
        other => Err(HttpError::new(
            ErrorCode::Validation,
            format!("Unsupported database backup method '{other}'."),
        )),
    }
}

pub fn build_run_spec(
    store: &Store,
    job: &BackupJobRecord,
    run_prune: bool,
    run_check: bool,
) -> Result<BackupRunSpec, HttpError> {
    let target = match job.kind {
        BackupTargetKind::Opsbox => BackupRunTarget::Opsbox,
        BackupTargetKind::Path => {
            let raw_paths: Vec<String> = job
                .what
                .get("paths")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let paths = validate_path_list(&raw_paths).map_err(map_val_err)?;
            BackupRunTarget::Path { paths }
        }
        BackupTargetKind::Volume => {
            let mut raw_vols: Vec<String> = job
                .what
                .get("volumes")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            if raw_vols.is_empty()
                && let Some(sname) = job.what.get("serviceName").and_then(Value::as_str)
            {
                raw_vols.push(format!("opsbox-{sname}-data"));
            }
            let volumes = validate_volume_list(&raw_vols).map_err(map_val_err)?;
            let pause_containers = job
                .what
                .get("pause")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            BackupRunTarget::Volume {
                volumes,
                pause_containers,
            }
        }
        BackupTargetKind::Database => {
            let sid = job
                .service_id
                .as_deref()
                .or_else(|| job.what.get("serviceId").and_then(Value::as_str))
                .ok_or_else(|| {
                    HttpError::new(
                        ErrorCode::Validation,
                        "Database backup job is not linked to a service.",
                    )
                })?;
            let svc = store.get_service(sid).map_err(store_err)?.ok_or_else(|| {
                HttpError::new(
                    ErrorCode::NotFound,
                    "Linked database service no longer exists.",
                )
            })?;
            let container_id = svc
                .active_container
                .clone()
                .or_else(|| {
                    store
                        .list_deploys(&svc.id, 1)
                        .ok()
                        .and_then(|v| v.into_iter().next())
                        .and_then(|d| d.container_id)
                })
                .ok_or_else(|| {
                    HttpError::new(
                        ErrorCode::Conflict,
                        format!(
                            "Database service '{}' has no running container to back up.",
                            svc.name
                        ),
                    )
                })?;
            let template = infer_db_kind(job)?;
            let vars = store_env_specs(store, &svc.id);
            BackupRunTarget::Database {
                template,
                service_id: svc.id,
                service_name: svc.name,
                container_id,
                vars,
            }
        }
    };

    Ok(BackupRunSpec {
        destination_id: job.destination_id.clone(),
        job_id: job.id.clone(),
        job_name: job.name.clone(),
        target,
        retain_daily: u32::try_from(job.retain_daily.max(0)).unwrap_or(14),
        retain_weekly: u32::try_from(job.retain_weekly.max(0)).unwrap_or(8),
        run_prune,
        run_check,
    })
}

async fn sync_all_snapshots(
    store: &Arc<Store>,
    priv_client: &PrivClient,
    rid: &str,
    actor: Option<&str>,
    override_check: Option<(BackupCheckState, i64)>,
) {
    let Ok(snaps) = priv_client
        .backup_snapshots(
            rid,
            actor,
            BackupSnapshotsRequest {
                destination_id: LOCAL_DESTINATION_ID.to_owned(),
            },
        )
        .await
    else {
        return;
    };
    let st = store.clone();
    let _ = tokio::task::spawn_blocking(move || {
        sync_restic_snapshots_to_store(&st, &snaps, override_check)
    })
    .await;
}

pub fn sync_restic_snapshots_to_store(
    store: &Store,
    snaps: &[ResticSnapshotInfo],
    override_check: Option<(BackupCheckState, i64)>,
) -> Result<(), opsbox_store::StoreError> {
    let now = now_unix();
    let jobs = store.list_backup_jobs().unwrap_or_default();
    let mut rows = Vec::with_capacity(snaps.len());
    for s in snaps {
        let matched_job = s
            .job_id
            .as_deref()
            .and_then(|jid| jobs.iter().find(|j| j.id == jid));
        let kind = s
            .kind
            .or_else(|| matched_job.map(|j| j.kind))
            .unwrap_or(BackupTargetKind::Path);
        let what = s
            .what
            .clone()
            .or_else(|| matched_job.map(job_what_label))
            .unwrap_or_else(|| {
                if s.paths.is_empty() {
                    s.short_id.clone()
                } else {
                    s.paths.join(", ")
                }
            });
        rows.push((s, kind, what));
    }
    let sync_refs: Vec<SyncBackupSnapshot<'_>> = rows
        .iter()
        .map(|(s, kind, what)| SyncBackupSnapshot {
            job_id: s.job_id.as_deref(),
            restic_id: &s.restic_id,
            short_id: &s.short_id,
            what,
            kind: *kind,
            taken_at: s.taken_at,
            size_bytes: i64::try_from(s.size_bytes).unwrap_or(i64::MAX),
            tags: &s.tags,
            paths: &s.paths,
        })
        .collect();
    store.sync_backup_snapshots(LOCAL_DESTINATION_ID, &sync_refs, override_check, now)
}

fn collect_check_paths(jobs: &[BackupJobRecord]) -> Vec<String> {
    let mut out = Vec::new();
    for j in jobs {
        if j.kind == BackupTargetKind::Path
            && let Some(arr) = j.what.get("paths").and_then(Value::as_array)
        {
            for v in arr {
                if let Some(s) = v.as_str()
                    && !out.iter().any(|x| x == s)
                {
                    out.push(s.to_owned());
                }
            }
        }
    }
    out
}

fn job_what_label(job: &BackupJobRecord) -> String {
    match job.kind {
        BackupTargetKind::Opsbox => "opsbox (SQLite + /etc/opsbox)".to_owned(),
        BackupTargetKind::Database => job
            .what
            .get("serviceName")
            .and_then(Value::as_str)
            .unwrap_or(&job.name)
            .to_owned(),
        BackupTargetKind::Path => job
            .what
            .get("paths")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| job.name.clone()),
        BackupTargetKind::Volume => job
            .what
            .get("volumes")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| job.name.clone()),
    }
}

fn format_ago(ts: i64, now: i64) -> String {
    let diff = (now - ts).max(0);
    if diff < 60 {
        return "just now".to_owned();
    }
    let mins = diff / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 48 {
        return format!("{hours}h ago");
    }
    format!("{}d ago", hours / 24)
}

fn human_bytes(bytes: u64) -> String {
    const MIB: f64 = (1u64 << 20) as f64;
    const GIB: f64 = (1u64 << 30) as f64;
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    if bytes < 1024 * 1024 {
        return format!("{} KB", bytes / 1024);
    }
    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    if bytes < 1024 * 1024 * 1024 {
        return format!("{:.1} MB", b / MIB);
    }
    format!("{:.2} GB", b / GIB)
}

fn job_json(job: &BackupJobRecord, now: i64) -> Value {
    let last = match (job.last_status.as_deref(), job.last_run_at) {
        (Some("running"), _) => "running".to_owned(),
        (Some("failed"), Some(t)) => format!("failed · {}", format_ago(t, now)),
        (Some("failed"), None) => "failed".to_owned(),
        (Some("ok"), Some(t)) => format!("ok · {}", format_ago(t, now)),
        _ if !job.enabled => "disabled".to_owned(),
        _ => "never".to_owned(),
    };
    let kind_label = match job.kind {
        BackupTargetKind::Database => "database",
        BackupTargetKind::Volume => "volume",
        BackupTargetKind::Path => "files",
        BackupTargetKind::Opsbox => "self",
    };
    json!({
        "id": job.id,
        "name": job.name,
        "kind": job.kind.as_str(),
        "kindLabel": kind_label,
        "what": job_what_label(job),
        "whatDetail": job.what,
        "serviceId": job.service_id,
        "templateId": job.template_id,
        "method": job.method,
        "cron": job.cron,
        "cronHuman": human_label(&job.cron),
        "schedule": human_label(&job.cron),
        "retainDaily": job.retain_daily,
        "retainWeekly": job.retain_weekly,
        "retention": format!("{} daily · {} weekly", job.retain_daily, job.retain_weekly),
        "destinationId": job.destination_id,
        "destination": job.destination_id,
        "enabled": job.enabled,
        "last": last,
        "lastRunAt": job.last_run_at,
        "lastStatus": job.last_status,
        "lastError": job.last_error,
        "lastSnapshotId": job.last_snapshot_id,
        "lastSizeBytes": job.last_size_bytes,
        "lastDurationMs": job.last_duration_ms,
        "nextRunAt": job.next_run_at,
    })
}

fn snapshot_json(snap: &BackupSnapshotRecord) -> Value {
    json!({
        "id": snap.id,
        "resticId": snap.restic_id,
        "shortId": snap.short_id,
        "jobId": snap.job_id,
        "destinationId": snap.destination_id,
        "when": format_utc_short(snap.taken_at),
        "takenAt": snap.taken_at,
        "what": snap.what,
        "kind": snap.kind.as_str(),
        "sizeBytes": snap.size_bytes,
        "where": snap.destination_id,
        "check": snap.check_state.as_str(),
        "checkLabel": snap.check_state.display_label(),
        "checkedAt": snap.checked_at,
        "tags": snap.tags,
        "paths": snap.paths,
    })
}

fn dest_json(d: &BackupDestinationRecord, local_info: Option<&BackupLocalInfo>) -> Value {
    let detail = if d.provider == "local" {
        local_info
            .map(|i| i.path.clone())
            .or_else(|| {
                d.config
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "/var/backups/opsbox".to_owned())
    } else {
        d.name.clone()
    };
    let status = if d.provider == "local" {
        match local_info {
            Some(i) if !i.restic_installed => "restic_missing",
            Some(i) if i.repo_initialized || i.exists => "ok",
            Some(_) => "pending",
            None => "unavailable",
        }
    } else if d.verified_at.is_some() {
        "ok"
    } else {
        "pending"
    };
    json!({
        "id": d.id,
        "provider": d.provider,
        "name": d.name,
        "detail": detail,
        "config": d.config,
        "status": status,
        "verifiedAt": d.verified_at,
    })
}

fn local_disk_json(info: &BackupLocalInfo) -> Value {
    json!({
        "path": info.path,
        "exists": info.exists,
        "repoInitialized": info.repo_initialized,
        "resticInstalled": info.restic_installed,
        "resticVersion": info.restic_version,
        "mountPoint": info.mount_point,
        "fsType": info.fs_type,
        "totalBytes": info.total_bytes,
        "availableBytes": info.available_bytes,
        "sameDisk": info.same_disk,
        "sameDiskPaths": info.same_disk_paths,
    })
}

/// Collect active backup-related items for `GET /api/v1/attention`.
pub(crate) fn collect_attention_items(state: &AppState, who: &Principal) -> Vec<Attention> {
    let mut out = Vec::new();
    let now = now_unix();
    let jobs = match state.store.list_backup_jobs() {
        Ok(j) => j,
        Err(_) => return out,
    };

    for job in &jobs {
        if let Some(ref sid) = job.service_id {
            if let Ok(Some(svc)) = state.store.get_service(sid) {
                if !who.allows_service(&svc.name) {
                    continue;
                }
                if who.user.role == Role::Deployer
                    && !who.via_token
                    && svc.owner_id.as_deref() != Some(who.user.id.as_str())
                {
                    continue;
                }
            } else if who.scoped_service.is_some() {
                continue;
            }
        } else if who.scoped_service.is_some() {
            continue;
        }

        if job.last_status.as_deref() == Some("failed") {
            let reason = job.last_error.as_deref().unwrap_or("Backup failed");
            out.push(Attention {
                kind: "backup",
                text: format!("Backup failed for {}: {reason}", job.name),
                link: ("View backups →".to_owned(), "#/backups".to_owned()),
            });
        } else if job.enabled
            && job.last_status.as_deref() != Some("running")
            && is_run_missed(
                &job.cron,
                job.last_run_at,
                job.created_at,
                now,
                MISSED_RUN_GRACE_SECS,
            )
        {
            out.push(Attention {
                kind: "backup",
                text: format!(
                    "Scheduled backup missed for {} ({})",
                    job.name,
                    human_label(&job.cron)
                ),
                link: ("View backups →".to_owned(), "#/backups".to_owned()),
            });
        }
    }

    if who.scoped_service.is_none()
        && let Ok(snaps) = state.store.list_backup_snapshots(50)
    {
        let failed_count = snaps
            .iter()
            .filter(|s| s.check_state == BackupCheckState::ChecksumFailed)
            .count();
        if failed_count > 0 {
            out.push(Attention {
                kind: "backup",
                text: format!("Backup checksum verification failed on {failed_count} snapshot(s)"),
                link: ("Inspect backups →".to_owned(), "#/backups".to_owned()),
            });
        }
    }
    out
}

/// `GET /api/v1/backups`
pub(crate) async fn overview(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let store = state.store.clone();
    let dir_str = state.backups.backup_dir().display().to_string();
    let (jobs, dests) = tokio::task::spawn_blocking(move || {
        let now = now_unix();
        let _ = store.ensure_local_destination_path(&dir_str, now);
        initialize_missing_next_runs(&store, now);
        Ok::<_, opsbox_store::StoreError>((
            store.list_backup_jobs()?,
            store.list_backup_destinations()?,
        ))
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(store_err)?;

    let check_paths = collect_check_paths(&jobs);
    let local_info = state
        .priv_client
        .backup_local_info(&rid, Some(&who.user.name), check_paths)
        .await
        .ok();

    if local_info
        .as_ref()
        .is_some_and(|i| i.restic_installed && i.repo_initialized)
    {
        sync_all_snapshots(
            &state.store,
            &state.priv_client,
            &rid,
            Some(&who.user.name),
            None,
        )
        .await;
    }

    let store_snaps = state.store.clone();
    let snapshots = tokio::task::spawn_blocking(move || {
        store_snaps.list_backup_snapshots(DEFAULT_SNAPSHOT_LIMIT)
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(store_err)?;

    let now = now_unix();
    let same_disk = local_info.as_ref().is_some_and(|i| i.same_disk);
    let jobs_json: Vec<Value> = jobs.iter().map(|j| job_json(j, now)).collect();
    let snaps_json: Vec<Value> = snapshots.iter().map(snapshot_json).collect();
    let dests_json: Vec<Value> = dests
        .iter()
        .map(|d| dest_json(d, local_info.as_ref()))
        .collect();

    Ok(Json(json!({
        "jobs": jobs_json,
        "snapshots": snaps_json,
        "destinations": dests_json,
        "localDisk": local_info.as_ref().map(local_disk_json),
        "sameDisk": same_disk,
        "remoteDestinationsReady": false,
        "restoreReady": false,
    })))
}

/// `GET /api/v1/backups/snapshots`
pub(crate) async fn list_snapshots(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    match state
        .priv_client
        .backup_snapshots(
            &rid,
            Some(&who.user.name),
            BackupSnapshotsRequest {
                destination_id: LOCAL_DESTINATION_ID.to_owned(),
            },
        )
        .await
    {
        Ok(snaps) => {
            let st = state.store.clone();
            let _ = tokio::task::spawn_blocking(move || {
                sync_restic_snapshots_to_store(&st, &snaps, None)
            })
            .await;
        }
        Err(ClientError::Remote(ref r)) if r.code == PrivErrorCode::Unsupported => {}
        Err(e) => {
            warn!(error = %e, "could not refresh restic snapshots; returning cached list");
        }
    }

    let st = state.store.clone();
    let rows =
        tokio::task::spawn_blocking(move || st.list_backup_snapshots(DEFAULT_SNAPSHOT_LIMIT))
            .await
            .map_err(|_| HttpError::internal())?
            .map_err(store_err)?;
    let out: Vec<Value> = rows.iter().map(snapshot_json).collect();
    Ok(Json(json!({ "snapshots": out })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateJobBody {
    name: String,
    kind: String,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    volumes: Vec<String>,
    #[serde(default)]
    pause_containers: bool,
    service_id: Option<String>,
    cron: String,
    retain_daily: Option<u32>,
    retain_weekly: Option<u32>,
    destination_id: Option<String>,
    enabled: Option<bool>,
}

/// `POST /api/v1/backups/jobs`
pub(crate) async fn create_job(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    ApiJson(body): ApiJson<CreateJobBody>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let name = validate_job_name(&body.name).map_err(map_val_err)?;
    let cron = validate_cron(&body.cron).map_err(map_val_err)?;
    let (retain_daily, retain_weekly) = validate_retention(
        body.retain_daily.unwrap_or(14),
        body.retain_weekly.unwrap_or(8),
    )
    .map_err(map_val_err)?;
    let destination_id = body
        .destination_id
        .as_deref()
        .unwrap_or(LOCAL_DESTINATION_ID)
        .trim()
        .to_owned();
    if !valid_backup_id(&destination_id) {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Invalid backup destination ID.",
        ));
    }
    if destination_id != LOCAL_DESTINATION_ID {
        return Err(HttpError::new(
            ErrorCode::Unsupported,
            "Off-server destinations are not enabled yet; choose local storage.",
        ));
    }
    let enabled = body.enabled.unwrap_or(true);

    let store = state.store.clone();
    let kind_raw = body.kind.trim().to_ascii_lowercase();
    let paths_raw = body.paths;
    let vols_raw = body.volumes;
    let pause = body.pause_containers;
    let service_ref = body.service_id;

    let created = tokio::task::spawn_blocking(move || -> Result<BackupJobRecord, HttpError> {
        let now = now_unix();
        let next_run = if enabled {
            next_run_after_safe(&cron, now)
        } else {
            None
        };
        match kind_raw.as_str() {
            "path" | "files" => {
                let paths = validate_path_list(&paths_raw).map_err(map_val_err)?;
                let what = json!({ "paths": paths });
                store
                    .create_backup_job(NewBackupJob {
                        name: &name,
                        kind: BackupTargetKind::Path,
                        what: &what,
                        service_id: None,
                        template_id: None,
                        method: "restic",
                        cron: &cron,
                        retain_daily: i64::from(retain_daily),
                        retain_weekly: i64::from(retain_weekly),
                        destination_id: &destination_id,
                        enabled,
                        next_run_at: next_run,
                        now,
                    })
                    .map_err(store_err)
            }
            "volume" | "volumes" => {
                let volumes = validate_volume_list(&vols_raw).map_err(map_val_err)?;
                let what = json!({ "volumes": volumes, "pause": pause });
                store
                    .create_backup_job(NewBackupJob {
                        name: &name,
                        kind: BackupTargetKind::Volume,
                        what: &what,
                        service_id: None,
                        template_id: None,
                        method: "volume",
                        cron: &cron,
                        retain_daily: i64::from(retain_daily),
                        retain_weekly: i64::from(retain_weekly),
                        destination_id: &destination_id,
                        enabled,
                        next_run_at: next_run,
                        now,
                    })
                    .map_err(store_err)
            }
            "database" => {
                let sid_in = service_ref
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        HttpError::new(ErrorCode::Validation, "Select a managed database service.")
                    })?;
                let svc = store
                    .get_service(sid_in)
                    .map_err(store_err)?
                    .ok_or_else(|| {
                        HttpError::new(ErrorCode::NotFound, "Service does not exist.")
                    })?;
                if svc.kind != ServiceKind::Db {
                    return Err(HttpError::new(
                        ErrorCode::Validation,
                        "Logical database dumps apply to template database services.",
                    ));
                }
                if store
                    .get_backup_job_for_service(&svc.id)
                    .map_err(store_err)?
                    .is_some()
                {
                    return Err(HttpError::new(
                        ErrorCode::Conflict,
                        format!("A backup schedule already exists for '{}'.", svc.name),
                    ));
                }
                let tmpl_id = svc.template_id.clone();
                let method = tmpl_id
                    .as_deref()
                    .and_then(get_template)
                    .map_or("pg_dump".to_owned(), |t| t.backup.method.clone());
                let what = json!({
                    "serviceId": svc.id,
                    "serviceName": svc.name,
                    "templateId": tmpl_id,
                    "method": method,
                });
                store
                    .create_backup_job(NewBackupJob {
                        name: &name,
                        kind: BackupTargetKind::Database,
                        what: &what,
                        service_id: Some(&svc.id),
                        template_id: tmpl_id.as_deref(),
                        method: &method,
                        cron: &cron,
                        retain_daily: i64::from(retain_daily),
                        retain_weekly: i64::from(retain_weekly),
                        destination_id: &destination_id,
                        enabled,
                        next_run_at: next_run,
                        now,
                    })
                    .map_err(store_err)
            }
            "opsbox" => Err(HttpError::new(
                ErrorCode::Validation,
                "The built-in opsbox self-backup schedule already exists; edit it instead.",
            )),
            _ => Err(HttpError::new(
                ErrorCode::Validation,
                "Backup kind must be 'path', 'volume', or 'database'.",
            )),
        }
    })
    .await
    .map_err(|_| HttpError::internal())??;

    audit(
        &state,
        &who,
        &client.ip,
        "backup.job.create",
        Some(&created.id),
        json!({ "name": created.name, "kind": created.kind.as_str(), "cron": created.cron }),
    );
    Ok((
        StatusCode::CREATED,
        Json(json!({ "job": job_json(&created, now_unix()) })),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateJobBody {
    name: Option<String>,
    cron: Option<String>,
    retain_daily: Option<u32>,
    retain_weekly: Option<u32>,
    enabled: Option<bool>,
    paths: Option<Vec<String>>,
    volumes: Option<Vec<String>>,
    pause_containers: Option<bool>,
}

/// `PATCH /api/v1/backups/jobs/{id}`
pub(crate) async fn update_job(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateJobBody>,
) -> Result<Json<Value>, HttpError> {
    if !valid_backup_id(&id) {
        return Err(HttpError::not_found());
    }
    let clean_name = body
        .name
        .as_deref()
        .map(validate_job_name)
        .transpose()
        .map_err(map_val_err)?;
    let clean_cron = body
        .cron
        .as_deref()
        .map(validate_cron)
        .transpose()
        .map_err(map_val_err)?;
    let store = state.store.clone();
    let jid = id.clone();

    let updated = tokio::task::spawn_blocking(move || -> Result<BackupJobRecord, HttpError> {
        let existing = store
            .get_backup_job(&jid)
            .map_err(store_err)?
            .ok_or_else(HttpError::not_found)?;
        let cur_daily = u32::try_from(existing.retain_daily.max(0)).unwrap_or(14);
        let cur_weekly = u32::try_from(existing.retain_weekly.max(0)).unwrap_or(8);
        let new_daily = body.retain_daily.unwrap_or(cur_daily);
        let new_weekly = body.retain_weekly.unwrap_or(cur_weekly);
        let (rd, rw) = validate_retention(new_daily, new_weekly).map_err(map_val_err)?;

        let new_what = match existing.kind {
            BackupTargetKind::Path => {
                if let Some(ref p) = body.paths {
                    let clean = validate_path_list(p).map_err(map_val_err)?;
                    Some(json!({ "paths": clean }))
                } else {
                    None
                }
            }
            BackupTargetKind::Volume => {
                if body.volumes.is_some() || body.pause_containers.is_some() {
                    let vols = match body.volumes {
                        Some(ref v) => validate_volume_list(v).map_err(map_val_err)?,
                        None => existing
                            .what
                            .get("volumes")
                            .and_then(Value::as_array)
                            .map(|a| {
                                a.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    };
                    let pause = body.pause_containers.unwrap_or_else(|| {
                        existing
                            .what
                            .get("pause")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    });
                    let mut obj = existing.what.clone();
                    obj["volumes"] = json!(vols);
                    obj["pause"] = json!(pause);
                    Some(obj)
                } else {
                    None
                }
            }
            BackupTargetKind::Database | BackupTargetKind::Opsbox => None,
        };

        let now = now_unix();
        let target_cron = clean_cron.as_deref().unwrap_or(&existing.cron);
        let target_enabled = body.enabled.unwrap_or(existing.enabled);
        let next_run = if clean_cron.is_some() || body.enabled.is_some() {
            if target_enabled {
                Some(next_run_after_safe(target_cron, now))
            } else {
                Some(None)
            }
        } else {
            None
        };

        store
            .update_backup_job(
                &jid,
                UpdateBackupJob {
                    name: clean_name.as_deref(),
                    what: new_what.as_ref(),
                    cron: clean_cron.as_deref(),
                    retain_daily: body.retain_daily.map(|_| i64::from(rd)),
                    retain_weekly: body.retain_weekly.map(|_| i64::from(rw)),
                    enabled: body.enabled,
                    next_run_at: next_run,
                    now,
                },
            )
            .map_err(store_err)?
            .ok_or_else(HttpError::not_found)
    })
    .await
    .map_err(|_| HttpError::internal())??;

    audit(
        &state,
        &who,
        &client.ip,
        "backup.job.update",
        Some(&updated.id),
        json!({ "name": updated.name, "enabled": updated.enabled, "cron": updated.cron }),
    );
    Ok(Json(json!({ "job": job_json(&updated, now_unix()) })))
}

/// `DELETE /api/v1/backups/jobs/{id}`
pub(crate) async fn delete_job(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    if !valid_backup_id(&id) {
        return Err(HttpError::not_found());
    }
    if id == OPSBOX_SELF_JOB_ID {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "The built-in opsbox self-backup schedule cannot be deleted; disable it instead.",
        ));
    }
    let store = state.store.clone();
    let jid = id.clone();
    let deleted = tokio::task::spawn_blocking(move || store.delete_backup_job(&jid))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(store_err)?;
    if !deleted {
        return Err(HttpError::not_found());
    }
    audit(
        &state,
        &who,
        &client.ip,
        "backup.job.delete",
        Some(&id),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunOptionsBody {
    #[serde(default)]
    run_prune: bool,
    #[serde(default)]
    run_check: bool,
}

/// `POST /api/v1/backups/jobs/{id}/run`
pub(crate) async fn run_job(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    body: Option<ApiJson<RunOptionsBody>>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    if !valid_backup_id(&id) {
        return Err(HttpError::not_found());
    }
    let store = state.store.clone();
    let jid = id.clone();
    let job = tokio::task::spawn_blocking(move || store.get_backup_job(&jid))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(store_err)?
        .ok_or_else(HttpError::not_found)?;

    let opts = body.map(|ApiJson(b)| b).unwrap_or_default();
    let s = state.clone();
    let actor_name = who.user.name.clone();
    let ip = client.ip.clone();
    let principal = who.clone();

    let task = state.tasks.start("backups.run", async move {
        let res = s
            .backups
            .execute_job(
                &s.store,
                &s.priv_client,
                &rid,
                Some(&actor_name),
                &job,
                opts,
            )
            .await;
        sync_all_snapshots(&s.store, &s.priv_client, &rid, Some(&actor_name), None).await;
        match res {
            Ok(line) => {
                audit(
                    &s,
                    &principal,
                    &ip,
                    "backup.run",
                    Some(&job.id),
                    json!({ "job": job.name }),
                );
                Ok(Outcome {
                    log: vec![line],
                    result: json!({ "jobId": job.id, "ok": true }),
                })
            }
            Err(e) => {
                audit(
                    &s,
                    &principal,
                    &ip,
                    "backup.run.failed",
                    Some(&job.id),
                    json!({ "job": job.name, "error": e.0.error.message }),
                );
                Err(e)
            }
        }
    })?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}

/// `POST /api/v1/backups/run` — run all enabled jobs right now.
pub(crate) async fn run_all(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<RunOptionsBody>>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let store = state.store.clone();
    let jobs: Vec<BackupJobRecord> = tokio::task::spawn_blocking(move || store.list_backup_jobs())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(store_err)?
        .into_iter()
        .filter(|j| j.enabled)
        .collect();
    if jobs.is_empty() {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "No enabled backup schedules exist to run.",
        ));
    }

    let opts = body.map(|ApiJson(b)| b).unwrap_or_default();
    let s = state.clone();
    let actor_name = who.user.name.clone();
    let ip = client.ip.clone();
    let principal = who.clone();

    let task = state.tasks.start("backups.run", async move {
        let mut log = Vec::new();
        let mut ok_count = 0u32;
        let mut errors = Vec::new();
        for job in &jobs {
            match s
                .backups
                .execute_job(&s.store, &s.priv_client, &rid, Some(&actor_name), job, opts)
                .await
            {
                Ok(line) => {
                    ok_count += 1;
                    log.push(line);
                }
                Err(e) => {
                    let msg = e.0.error.message;
                    log.push(format!("{}: failed: {msg}", job.name));
                    errors.push(format!("{}: {msg}", job.name));
                }
            }
        }
        sync_all_snapshots(&s.store, &s.priv_client, &rid, Some(&actor_name), None).await;
        audit(
            &s,
            &principal,
            &ip,
            "backup.run_all",
            None,
            json!({ "ok": ok_count, "failed": errors.len() }),
        );
        if !errors.is_empty() {
            return Err(HttpError::new(ErrorCode::Internal, errors.join("; ")));
        }
        Ok(Outcome {
            log,
            result: json!({ "ran": ok_count, "failed": 0 }),
        })
    })?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PruneBody {
    job_id: Option<String>,
    retain_daily: Option<u32>,
    retain_weekly: Option<u32>,
}

/// `POST /api/v1/backups/prune` — apply retention policy via `restic forget --prune`.
pub(crate) async fn prune(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<PruneBody>>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let body = body.map(|ApiJson(b)| b).unwrap_or_default();
    if let Some(ref jid) = body.job_id
        && !valid_backup_id(jid)
    {
        return Err(HttpError::new(ErrorCode::Validation, "Invalid job ID."));
    }
    let store = state.store.clone();
    let job_id_clone = body.job_id.clone();
    let jobs_to_prune: Vec<(Option<String>, u32, u32)> = tokio::task::spawn_blocking(
        move || -> Result<Vec<(Option<String>, u32, u32)>, HttpError> {
            if let Some(jid) = job_id_clone {
                let job = store
                    .get_backup_job(&jid)
                    .map_err(store_err)?
                    .ok_or_else(HttpError::not_found)?;
                let rd = body
                    .retain_daily
                    .unwrap_or_else(|| u32::try_from(job.retain_daily.max(0)).unwrap_or(14));
                let rw = body
                    .retain_weekly
                    .unwrap_or_else(|| u32::try_from(job.retain_weekly.max(0)).unwrap_or(8));
                validate_retention(rd, rw).map_err(map_val_err)?;
                return Ok(vec![(Some(jid), rd, rw)]);
            }
            if let (Some(rd), Some(rw)) = (body.retain_daily, body.retain_weekly) {
                validate_retention(rd, rw).map_err(map_val_err)?;
                return Ok(vec![(None, rd, rw)]);
            }
            let all = store.list_backup_jobs().map_err(store_err)?;
            let mut out = Vec::with_capacity(all.len());
            for j in all {
                let rd = u32::try_from(j.retain_daily.max(0)).unwrap_or(14);
                let rw = u32::try_from(j.retain_weekly.max(0)).unwrap_or(8);
                if rd > 0 || rw > 0 {
                    out.push((Some(j.id), rd, rw));
                }
            }
            if out.is_empty() {
                out.push((None, 14, 8));
            }
            Ok(out)
        },
    )
    .await
    .map_err(|_| HttpError::internal())??;

    let s = state.clone();
    let actor_name = who.user.name.clone();
    let ip = client.ip.clone();
    let principal = who.clone();

    let task = state.tasks.start("backups.prune", async move {
        let _guard = s.backups.run_lock.lock().await;
        let mut total_removed = 0u32;
        let mut total_kept = 0u32;
        let mut log = Vec::new();
        for (jid, rd, rw) in jobs_to_prune {
            let res = s
                .priv_client
                .backup_forget(
                    &rid,
                    Some(&actor_name),
                    BackupForgetRequest {
                        destination_id: LOCAL_DESTINATION_ID.to_owned(),
                        job_id: jid,
                        retain_daily: rd,
                        retain_weekly: rw,
                    },
                )
                .await?;
            total_removed = total_removed.saturating_add(res.removed_count);
            total_kept = total_kept.saturating_add(res.kept_count);
            log.extend(res.log);
        }
        sync_all_snapshots(&s.store, &s.priv_client, &rid, Some(&actor_name), None).await;
        audit(
            &s,
            &principal,
            &ip,
            "backup.prune",
            None,
            json!({ "removed": total_removed, "kept": total_kept }),
        );
        Ok(Outcome {
            log,
            result: json!({ "removed": total_removed, "kept": total_kept }),
        })
    })?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckBody {
    read_data_subset: Option<String>,
}

/// `POST /api/v1/backups/check` — verify restic metadata & data subset checksums.
pub(crate) async fn check(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<CheckBody>>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let subset = body
        .map(|ApiJson(b)| b.read_data_subset)
        .unwrap_or(None)
        .map(|s| validate_read_data_subset(&s))
        .transpose()
        .map_err(map_val_err)?;

    let s = state.clone();
    let actor_name = who.user.name.clone();
    let ip = client.ip.clone();
    let principal = who.clone();

    let task = state.tasks.start("backups.check", async move {
        let _guard = s.backups.run_lock.lock().await;
        let res = s
            .priv_client
            .backup_check(
                &rid,
                Some(&actor_name),
                BackupCheckRequest {
                    destination_id: LOCAL_DESTINATION_ID.to_owned(),
                    read_data_subset: subset,
                },
            )
            .await?;
        let ts = now_unix();
        let chk_state = if res.ok {
            BackupCheckState::ChecksumOk
        } else {
            BackupCheckState::ChecksumFailed
        };
        sync_all_snapshots(
            &s.store,
            &s.priv_client,
            &rid,
            Some(&actor_name),
            Some((chk_state, ts)),
        )
        .await;
        audit(
            &s,
            &principal,
            &ip,
            "backup.check",
            None,
            json!({ "ok": res.ok, "errors": res.num_errors }),
        );
        if !res.ok {
            return Err(HttpError::new(ErrorCode::Internal, res.summary));
        }
        Ok(Outcome {
            log: vec![res.summary.clone()],
            result: json!({ "ok": true, "numErrors": res.num_errors, "summary": res.summary }),
        })
    })?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}
