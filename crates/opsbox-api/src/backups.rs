//! Backup orchestration, cron scheduler, retention/checksum triggers, and API handlers
//! (docs/BACKEND_SPEC.md 7.12).
//!
//! `serve` schedules jobs, validates inputs, manages metadata in SQLite, and surfaces
//! Attention alerts; every `restic`, container dump, `/etc/opsbox`, and `/var/backups/opsbox`
//! operation runs inside `opsbox-priv`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use opsbox_auth::Principal;
use opsbox_backup::BackupError;
use opsbox_backup::cron::{
    CronSchedule, format_utc_short, human_label, is_run_missed, validate_cron,
};
use opsbox_backup::destinations::{
    DestinationFieldInput, ValidatedDestinationConfig, validate_destination_input,
};
use opsbox_backup::dumps::default_stdin_filename;
use opsbox_backup::restic::date_suffix_utc;
use opsbox_backup::validation::{
    validate_job_name, validate_path_list, validate_read_data_subset, validate_retention,
    validate_volume_list,
};
use opsbox_deploy::templates::{generate_instance_env, get_template};
use opsbox_metrics::{Attention, Severity};
use opsbox_priv::PrivClient;
use opsbox_proto::{
    BackupCheckRequest, BackupCheckState, BackupDbKind, BackupDestinationTestRequest,
    BackupDestinationTestTarget, BackupForgetRequest, BackupGenerateSftpKeyRequest,
    BackupLocalInfo, BackupProviderId, BackupRepoTarget, BackupRestoreMode, BackupRestoreRequest,
    BackupRestoreTarget, BackupRestoreTestRequest, BackupRestoreTestTarget, BackupRunSpec,
    BackupRunTarget, BackupS3Credentials, BackupSnapshotsRequest, BackupTargetKind, DeployEnvSpec,
    DeployStartResult, ErrorCode, ResticSnapshotInfo, Role, Secret, TemplateStartRequest,
    backup_provider_presets, valid_backup_id, valid_service_name,
};
use opsbox_store::{
    BackupDestinationRecord, BackupJobRecord, BackupSnapshotRecord, DeployTrigger,
    LOCAL_DESTINATION_ID, NewBackupJob, NewService, OPSBOX_SELF_JOB_ID, ServiceKind, SourceKind,
    Store, SyncBackupSnapshot, UpdateBackupJob,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::deploy::{next_graph_pos, seal_and_store_vars, var_seal_context};
use crate::json::ApiJson;
use crate::session::{Admin, Client, Viewer};
use crate::system::audit;
use crate::tasks::{Outcome, Tasks};
use crate::{AppState, HttpError, RequestId};

const SCHEDULER_POLL_SECS: u64 = 30;
const MISSED_RUN_GRACE_SECS: i64 = 3600;
const DEFAULT_SNAPSHOT_LIMIT: usize = 200;
const RESTORE_TEST_INTERVAL_SECS: i64 = 30 * 24 * 3600;
pub(crate) const SETTING_DEFAULT_DEST: &str = "backups.default_destination_id";
const SETTING_NEXT_RESTORE_TEST_AT: &str = "backups.next_restore_test_at";
const SETTING_LAST_RESTORE_TEST_AT: &str = "backups.last_restore_test_at";
const SETTING_LAST_RESTORE_TEST_OK: &str = "backups.last_restore_test_ok";
const SETTING_LAST_RESTORE_TEST_SUMMARY: &str = "backups.last_restore_test_summary";
const SETTING_LAST_RESTORE_TEST_ERROR: &str = "backups.last_restore_test_error";

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
                let (due, restore_test_due) = tokio::task::spawn_blocking(move || {
                    initialize_missing_next_runs(&s, ts);
                    let jobs = s.list_due_backup_jobs(ts).unwrap_or_default();
                    let next_rt = s
                        .get_setting(SETTING_NEXT_RESTORE_TEST_AT)
                        .ok()
                        .flatten()
                        .and_then(|v| v.as_i64());
                    let rt_due = match next_rt {
                        None => {
                            let has_snaps =
                                !s.list_backup_snapshots(1).unwrap_or_default().is_empty();
                            if has_snaps {
                                let _ = s.set_setting(
                                    SETTING_NEXT_RESTORE_TEST_AT,
                                    &json!(ts + RESTORE_TEST_INTERVAL_SECS),
                                );
                            }
                            false
                        }
                        Some(due_at) => due_at <= ts,
                    };
                    (jobs, rt_due)
                })
                .await
                .unwrap_or_default();

                if !due.is_empty() {
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
                                    log.push(format!(
                                        "{}: failed: {}",
                                        job.name, e.0.error.message
                                    ));
                                    warn!(job = %job.id, error = %e.0.error.message, "scheduled backup failed");
                                }
                            }
                        }
                        sync_all_snapshots(&st, &pc, "sched-sync", None, None, None).await;
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

                if restore_test_due {
                    let pc = priv_client.clone();
                    let r = runner.clone();
                    let st = store.clone();
                    let _ = st.set_setting(
                        SETTING_NEXT_RESTORE_TEST_AT,
                        &json!(ts + RESTORE_TEST_INTERVAL_SECS),
                    );
                    let _ = tasks.start("backups.restore_test", async move {
                        let _guard = r.run_lock.lock().await;
                        execute_scheduled_restore_test(&st, &pc, "sched-restore-test", None).await
                    });
                }
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
        if let Some(fit) = crate::capacity::wait_backup(actor.is_none(), || async {
            priv_client.capacity(rid).await.map_err(HttpError::from)
        })
        .await?
        {
            store
                .set_setting(
                    "capacity.backup_alert",
                    &json!({"job":job.name,"at":now_unix(),"fit":fit}),
                )
                .map_err(|_| HttpError::internal())?;
        }
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

/// Build a [`BackupRepoTarget`] from a `backup_destinations` row using sealed credentials
/// (never unsealing any secrets in `serve`).
pub fn repo_target_from_record(
    dest: &BackupDestinationRecord,
) -> Result<BackupRepoTarget, HttpError> {
    if dest.id == LOCAL_DESTINATION_ID || dest.provider == "local" {
        return Ok(BackupRepoTarget::Local);
    }
    let secrets_enc = dest.secrets_enc.clone().ok_or_else(|| {
        HttpError::new(
            ErrorCode::Internal,
            format!("Destination '{}' is missing sealed credentials.", dest.name),
        )
    })?;
    let seal_context = format!("backup:dest:{}", dest.id);
    if dest.provider == "sftp" {
        let host = dest
            .config
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let port = dest
            .config
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(22);
        let user = dest
            .config
            .get("user")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let repo_path = dest
            .config
            .get("repoPath")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        return Ok(BackupRepoTarget::Sftp {
            destination_id: dest.id.clone(),
            host,
            port,
            user,
            repo_path,
            seal_context,
            private_key_enc: secrets_enc,
        });
    }

    let endpoint = dest
        .config
        .get("endpoint")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let bucket = dest
        .config
        .get("bucket")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let repo_path = dest
        .config
        .get("repoPath")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let region = dest
        .config
        .get("region")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(BackupRepoTarget::S3 {
        destination_id: dest.id.clone(),
        endpoint,
        bucket,
        repo_path,
        region,
        credentials: BackupS3Credentials::Sealed {
            seal_context,
            value_enc: secrets_enc,
        },
    })
}

/// Resolve the default destination ID for newly created jobs (offsite verified default if set, else `'local'`).
pub fn resolve_default_destination_id(store: &Store) -> String {
    if let Ok(Some(val)) = store.get_setting(SETTING_DEFAULT_DEST)
        && let Some(id) = val.as_str()
        && let Ok(Some(dest)) = store.get_backup_destination(id)
        && (dest.id == LOCAL_DESTINATION_ID || dest.verified_at.is_some())
    {
        return dest.id;
    }
    if let Ok(dests) = store.list_backup_destinations()
        && let Some(offsite) = dests
            .into_iter()
            .find(|d| d.id != LOCAL_DESTINATION_ID && d.verified_at.is_some())
    {
        return offsite.id;
    }
    LOCAL_DESTINATION_ID.to_owned()
}

pub fn build_run_spec(
    store: &Store,
    job: &BackupJobRecord,
    run_prune: bool,
    run_check: bool,
) -> Result<BackupRunSpec, HttpError> {
    let dest = store
        .get_backup_destination(&job.destination_id)
        .map_err(store_err)?
        .ok_or_else(|| {
            HttpError::new(
                ErrorCode::NotFound,
                format!(
                    "Backup destination '{}' does not exist.",
                    job.destination_id
                ),
            )
        })?;
    let repo = repo_target_from_record(&dest)?;
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
        repo,
        job_id: job.id.clone(),
        job_name: job.name.clone(),
        target,
        retain_daily: u32::try_from(job.retain_daily.max(0)).unwrap_or(14),
        retain_weekly: u32::try_from(job.retain_weekly.max(0)).unwrap_or(8),
        run_prune,
        run_check,
    })
}

pub(crate) async fn sync_all_snapshots(
    store: &Arc<Store>,
    priv_client: &PrivClient,
    rid: &str,
    actor: Option<&str>,
    only_destination_id: Option<&str>,
    override_check: Option<(BackupCheckState, i64)>,
) {
    let st_dests = store.clone();
    let dests = tokio::task::spawn_blocking(move || st_dests.list_backup_destinations())
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();

    for dest in dests {
        if let Some(only) = only_destination_id
            && dest.id != only
        {
            continue;
        }
        if dest.id != LOCAL_DESTINATION_ID && dest.verified_at.is_none() {
            continue;
        }
        let Ok(repo) = repo_target_from_record(&dest) else {
            continue;
        };
        let Ok(snaps) = priv_client
            .backup_snapshots(
                rid,
                actor,
                BackupSnapshotsRequest {
                    destination_id: dest.id.clone(),
                    repo,
                },
            )
            .await
        else {
            continue;
        };
        let st = store.clone();
        let did = dest.id.clone();
        let _ = tokio::task::spawn_blocking(move || {
            sync_restic_snapshots_to_store(&st, &did, &snaps, override_check)
        })
        .await;
    }
}

pub fn sync_restic_snapshots_to_store(
    store: &Store,
    destination_id: &str,
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
    store.sync_backup_snapshots(destination_id, &sync_refs, override_check, now)
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

fn snapshot_json(snap: &BackupSnapshotRecord, dest_names: &HashMap<String, String>) -> Value {
    let where_label = dest_names
        .get(&snap.destination_id)
        .cloned()
        .unwrap_or_else(|| snap.destination_id.clone());
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
        "where": where_label,
        "check": snap.check_state.as_str(),
        "checkLabel": snap.check_state.display_label(),
        "checkedAt": snap.checked_at,
        "tags": snap.tags,
        "paths": snap.paths,
    })
}

fn dest_json(
    d: &BackupDestinationRecord,
    default_dest_id: &str,
    local_info: Option<&BackupLocalInfo>,
) -> Value {
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
        d.config
            .get("displayLoc")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| d.name.clone())
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
        "publicKey": d.config.get("publicKey").and_then(Value::as_str),
        "fingerprint": d.config.get("fingerprint").and_then(Value::as_str),
        "isDefault": d.id == default_dest_id,
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

        let subject = if job.id == OPSBOX_SELF_JOB_ID {
            "opsbox"
        } else {
            job.id.as_str()
        };
        let item_id = format!("backup:{subject}");

        if job.last_status.as_deref() == Some("failed") {
            let reason = job.last_error.as_deref().unwrap_or("Backup failed");
            out.push(Attention {
                id: item_id,
                kind: "backup".into(),
                severity: Severity::Critical,
                since: job.last_run_at.unwrap_or(job.updated_at).max(1),
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
            let missed_since = CronSchedule::parse(&job.cron)
                .ok()
                .and_then(|s| s.prev_run_at_or_before(now.saturating_sub(MISSED_RUN_GRACE_SECS)))
                .unwrap_or(job.updated_at)
                .max(1);
            out.push(Attention {
                id: item_id,
                kind: "backup".into(),
                severity: Severity::Warning,
                since: missed_since,
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
        && let Ok(snaps) = state.store.list_failed_backup_snapshots()
    {
        let failed: Vec<_> = snaps
            .into_iter()
            .filter(|s| s.check_state == BackupCheckState::ChecksumFailed)
            .collect();
        if !failed.is_empty() {
            let since = failed
                .iter()
                .map(|s| s.checked_at.unwrap_or(s.updated_at))
                .max()
                .unwrap_or(now)
                .max(1);
            out.push(Attention {
                id: "backup:checksum".into(),
                kind: "backup".into(),
                severity: Severity::Critical,
                since,
                text: format!(
                    "Backup checksum verification failed on {} snapshot(s)",
                    failed.len()
                ),
                link: ("Inspect backups →".to_owned(), "#/backups".to_owned()),
            });
        }
        if let Ok(Some(err_v)) = state.store.get_setting(SETTING_LAST_RESTORE_TEST_ERROR)
            && let Some(err_s) = err_v.as_str()
            && !err_s.trim().is_empty()
        {
            let since = state
                .store
                .get_setting(SETTING_LAST_RESTORE_TEST_AT)
                .ok()
                .flatten()
                .and_then(|v| v.as_i64())
                .unwrap_or(now)
                .max(1);
            out.push(Attention {
                id: "backup:restore-test".into(),
                kind: "backup".into(),
                severity: Severity::Critical,
                since,
                text: format!("Monthly backup restore test failed: {err_s}"),
                link: ("Inspect backups →".to_owned(), "#/backups".to_owned()),
            });
        }
    }
    out
}

fn dest_name_map(dests: &[BackupDestinationRecord]) -> HashMap<String, String> {
    dests
        .iter()
        .map(|d| (d.id.clone(), d.name.clone()))
        .collect()
}

/// `GET /api/v1/backups`
pub(crate) async fn overview(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let store = state.store.clone();
    let dir_str = state.backups.backup_dir().display().to_string();
    let (jobs, dests, default_dest_id, rt_meta) = tokio::task::spawn_blocking(move || {
        let now = now_unix();
        let _ = store.ensure_local_destination_path(&dir_str, now);
        initialize_missing_next_runs(&store, now);
        let default_id = resolve_default_destination_id(&store);
        let last_at = store
            .get_setting(SETTING_LAST_RESTORE_TEST_AT)
            .ok()
            .flatten()
            .and_then(|v| v.as_i64());
        let last_ok = store
            .get_setting(SETTING_LAST_RESTORE_TEST_OK)
            .ok()
            .flatten()
            .and_then(|v| v.as_bool());
        let last_summary = store
            .get_setting(SETTING_LAST_RESTORE_TEST_SUMMARY)
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_owned));
        let last_err = store
            .get_setting(SETTING_LAST_RESTORE_TEST_ERROR)
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|s| !s.is_empty());
        let next_at = store
            .get_setting(SETTING_NEXT_RESTORE_TEST_AT)
            .ok()
            .flatten()
            .and_then(|v| v.as_i64());
        Ok::<_, opsbox_store::StoreError>((
            store.list_backup_jobs()?,
            store.list_backup_destinations()?,
            default_id,
            json!({
                "lastRunAt": last_at,
                "lastOk": last_ok,
                "lastSummary": last_summary,
                "lastError": last_err,
                "nextRunAt": next_at,
            }),
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

    let has_remote = dests
        .iter()
        .any(|d| d.id != LOCAL_DESTINATION_ID && d.verified_at.is_some());
    if has_remote
        || local_info
            .as_ref()
            .is_some_and(|i| i.restic_installed && i.repo_initialized)
    {
        sync_all_snapshots(
            &state.store,
            &state.priv_client,
            &rid,
            Some(&who.user.name),
            None,
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
    let dnames = dest_name_map(&dests);
    let jobs_json: Vec<Value> = jobs.iter().map(|j| job_json(j, now)).collect();
    let snaps_json: Vec<Value> = snapshots
        .iter()
        .map(|s| snapshot_json(s, &dnames))
        .collect();
    let dests_json: Vec<Value> = dests
        .iter()
        .map(|d| dest_json(d, &default_dest_id, local_info.as_ref()))
        .collect();

    Ok(Json(json!({
        "jobs": jobs_json,
        "snapshots": snaps_json,
        "destinations": dests_json,
        "defaultDestinationId": default_dest_id,
        "localDisk": local_info.as_ref().map(local_disk_json),
        "sameDisk": same_disk,
        "remoteDestinationsReady": true,
        "restoreReady": true,
        "restoreTest": rt_meta,
    })))
}

/// `GET /api/v1/backups/snapshots`
pub(crate) async fn list_snapshots(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    sync_all_snapshots(
        &state.store,
        &state.priv_client,
        &rid,
        Some(&who.user.name),
        None,
        None,
    )
    .await;

    let st = state.store.clone();
    let (rows, dests) = tokio::task::spawn_blocking(move || {
        Ok::<_, opsbox_store::StoreError>((
            st.list_backup_snapshots(DEFAULT_SNAPSHOT_LIMIT)?,
            st.list_backup_destinations()?,
        ))
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(store_err)?;
    let dnames = dest_name_map(&dests);
    let out: Vec<Value> = rows.iter().map(|s| snapshot_json(s, &dnames)).collect();
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
    let requested_dest = body
        .destination_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if let Some(ref did) = requested_dest
        && !valid_backup_id(did)
    {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Invalid backup destination ID.",
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
        let destination_id = match requested_dest {
            Some(did) => {
                let dest = store
                    .get_backup_destination(&did)
                    .map_err(store_err)?
                    .ok_or_else(|| {
                        HttpError::new(
                            ErrorCode::NotFound,
                            format!("Backup destination '{did}' does not exist."),
                        )
                    })?;
                if dest.id != LOCAL_DESTINATION_ID && dest.verified_at.is_none() {
                    return Err(HttpError::new(
                        ErrorCode::Conflict,
                        format!(
                            "Verify destination '{}' before assigning jobs to it.",
                            dest.name
                        ),
                    ));
                }
                dest.id
            }
            None => resolve_default_destination_id(&store),
        };
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
    destination_id: Option<String>,
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

        let new_dest_id = if let Some(ref raw_did) = body.destination_id {
            let did = raw_did.trim();
            if !valid_backup_id(did) {
                return Err(HttpError::new(
                    ErrorCode::Validation,
                    "Invalid backup destination ID.",
                ));
            }
            let dest = store
                .get_backup_destination(did)
                .map_err(store_err)?
                .ok_or_else(|| {
                    HttpError::new(
                        ErrorCode::NotFound,
                        format!("Backup destination '{did}' does not exist."),
                    )
                })?;
            if dest.id != LOCAL_DESTINATION_ID && dest.verified_at.is_none() {
                return Err(HttpError::new(
                    ErrorCode::Conflict,
                    format!(
                        "Verify destination '{}' before assigning jobs to it.",
                        dest.name
                    ),
                ));
            }
            Some(dest.id)
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
            .ok_or_else(HttpError::not_found)?;
        if let Some(ref did) = new_dest_id {
            store
                .set_backup_job_destination(&jid, did, now)
                .map_err(store_err)?;
        }
        store
            .get_backup_job(&jid)
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
        sync_all_snapshots(
            &s.store,
            &s.priv_client,
            &rid,
            Some(&actor_name),
            Some(&job.destination_id),
            None,
        )
        .await;
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
        sync_all_snapshots(
            &s.store,
            &s.priv_client,
            &rid,
            Some(&actor_name),
            None,
            None,
        )
        .await;
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
    destination_id: Option<String>,
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
    if let Some(ref did) = body.destination_id
        && !valid_backup_id(did)
    {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Invalid destination ID.",
        ));
    }
    let store = state.store.clone();
    let job_id_clone = body.job_id.clone();
    let dest_id_clone = body.destination_id.clone();
    type PruneItem = (String, BackupRepoTarget, Option<String>, u32, u32);
    let jobs_to_prune: Vec<PruneItem> =
        tokio::task::spawn_blocking(move || -> Result<Vec<PruneItem>, HttpError> {
            let dests: HashMap<String, BackupDestinationRecord> = store
                .list_backup_destinations()
                .map_err(store_err)?
                .into_iter()
                .map(|d| (d.id.clone(), d))
                .collect();
            let repo_for = |did: &str| -> Result<BackupRepoTarget, HttpError> {
                let d = dests.get(did).ok_or_else(|| {
                    HttpError::new(
                        ErrorCode::NotFound,
                        format!("Destination '{did}' does not exist."),
                    )
                })?;
                repo_target_from_record(d)
            };

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
                let repo = repo_for(&job.destination_id)?;
                return Ok(vec![(job.destination_id, repo, Some(jid), rd, rw)]);
            }
            if let (Some(rd), Some(rw)) = (body.retain_daily, body.retain_weekly) {
                validate_retention(rd, rw).map_err(map_val_err)?;
                let did = dest_id_clone.unwrap_or_else(|| resolve_default_destination_id(&store));
                let repo = repo_for(&did)?;
                return Ok(vec![(did, repo, None, rd, rw)]);
            }
            let all = store.list_backup_jobs().map_err(store_err)?;
            let mut out = Vec::with_capacity(all.len());
            for j in all {
                if let Some(ref only_did) = dest_id_clone
                    && &j.destination_id != only_did
                {
                    continue;
                }
                let rd = u32::try_from(j.retain_daily.max(0)).unwrap_or(14);
                let rw = u32::try_from(j.retain_weekly.max(0)).unwrap_or(8);
                if (rd > 0 || rw > 0)
                    && let Ok(repo) = repo_for(&j.destination_id)
                {
                    out.push((j.destination_id, repo, Some(j.id), rd, rw));
                }
            }
            if out.is_empty() {
                let did = dest_id_clone.unwrap_or_else(|| LOCAL_DESTINATION_ID.to_owned());
                let repo = repo_for(&did)?;
                out.push((did, repo, None, 14, 8));
            }
            Ok(out)
        })
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
        for (did, repo, jid, rd, rw) in jobs_to_prune {
            let res = s
                .priv_client
                .backup_forget(
                    &rid,
                    Some(&actor_name),
                    BackupForgetRequest {
                        destination_id: did,
                        repo,
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
        sync_all_snapshots(
            &s.store,
            &s.priv_client,
            &rid,
            Some(&actor_name),
            None,
            None,
        )
        .await;
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
    destination_id: Option<String>,
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
    let body = body.map(|ApiJson(b)| b).unwrap_or_default();
    if let Some(ref did) = body.destination_id
        && !valid_backup_id(did)
    {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Invalid destination ID.",
        ));
    }
    let subset = body
        .read_data_subset
        .map(|s| validate_read_data_subset(&s))
        .transpose()
        .map_err(map_val_err)?;

    let store = state.store.clone();
    let explicit_did = body.destination_id.clone();
    let targets: Vec<(String, BackupRepoTarget)> = tokio::task::spawn_blocking(move || {
        let dests = store.list_backup_destinations().map_err(store_err)?;
        if let Some(ref want) = explicit_did {
            let d = dests
                .into_iter()
                .find(|x| &x.id == want)
                .ok_or_else(HttpError::not_found)?;
            return Ok::<_, HttpError>(vec![(d.id.clone(), repo_target_from_record(&d)?)]);
        }
        let snaps = store
            .list_backup_snapshots(DEFAULT_SNAPSHOT_LIMIT)
            .unwrap_or_default();
        let active_dests: std::collections::HashSet<&str> =
            snaps.iter().map(|s| s.destination_id.as_str()).collect();
        let mut out = Vec::new();
        for d in &dests {
            if (active_dests.contains(d.id.as_str())
                || d.id == LOCAL_DESTINATION_ID && active_dests.is_empty())
                && (d.id == LOCAL_DESTINATION_ID || d.verified_at.is_some())
                && let Ok(repo) = repo_target_from_record(d)
            {
                out.push((d.id.clone(), repo));
            }
        }
        if out.is_empty() {
            let fallback_id = resolve_default_destination_id(&store);
            if let Some(d) = dests.into_iter().find(|x| x.id == fallback_id) {
                out.push((d.id.clone(), repo_target_from_record(&d)?));
            }
        }
        Ok(out)
    })
    .await
    .map_err(|_| HttpError::internal())??;

    let s = state.clone();
    let actor_name = who.user.name.clone();
    let ip = client.ip.clone();
    let principal = who.clone();

    let task = state.tasks.start("backups.check", async move {
        let _guard = s.backups.run_lock.lock().await;
        let mut all_ok = true;
        let mut total_errors = 0u64;
        let mut summaries = Vec::new();
        for (did, repo) in targets {
            let res = s
                .priv_client
                .backup_check(
                    &rid,
                    Some(&actor_name),
                    BackupCheckRequest {
                        destination_id: did.clone(),
                        repo,
                        read_data_subset: subset.clone(),
                    },
                )
                .await?;
            let ts = now_unix();
            let chk_state = if res.ok {
                BackupCheckState::ChecksumOk
            } else {
                all_ok = false;
                BackupCheckState::ChecksumFailed
            };
            total_errors = total_errors.saturating_add(res.num_errors);
            summaries.push(res.summary);
            sync_all_snapshots(
                &s.store,
                &s.priv_client,
                &rid,
                Some(&actor_name),
                Some(&did),
                Some((chk_state, ts)),
            )
            .await;
        }
        let summary = summaries.join(" | ");
        audit(
            &s,
            &principal,
            &ip,
            "backup.check",
            None,
            json!({ "ok": all_ok, "errors": total_errors }),
        );
        if !all_ok {
            return Err(HttpError::new(ErrorCode::Internal, summary));
        }
        Ok(Outcome {
            log: summaries,
            result: json!({ "ok": true, "numErrors": total_errors, "summary": summary }),
        })
    })?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}

/// `GET /api/v1/backups/providers` — canonical offsite provider presets + server hostname.
pub(crate) async fn list_providers(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Json<Value> {
    let hostname = state
        .priv_client
        .host_info(&rid, Some(&who.user.name))
        .await
        .map(|h| h.hostname)
        .unwrap_or_else(|_| "opsbox-server".to_owned());
    Json(json!({
        "providers": backup_provider_presets(),
        "hostname": hostname,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TestDestinationBody {
    provider: BackupProviderId,
    #[serde(default, alias = "fields")]
    values: DestinationFieldInput,
}

/// `POST /api/v1/backups/destinations/test` — validate and probe an unsaved destination.
pub(crate) async fn test_destination(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<TestDestinationBody>,
) -> Result<Json<Value>, HttpError> {
    let hostname = state
        .priv_client
        .host_info(&rid, Some(&who.user.name))
        .await
        .map(|h| h.hostname)
        .unwrap_or_else(|_| "opsbox-server".to_owned());

    let cfg = validate_destination_input(
        body.provider,
        None,
        &body.values,
        &hostname,
        !body.provider.is_sftp(),
    )
    .map_err(map_val_err)?;

    let target = match cfg {
        ValidatedDestinationConfig::Sftp {
            host,
            port,
            user,
            repo_path,
            ..
        } => BackupDestinationTestTarget::SftpReachability {
            host,
            port,
            user,
            repo_path,
        },
        ValidatedDestinationConfig::S3 {
            endpoint,
            region,
            bucket,
            repo_path,
            ..
        } => BackupDestinationTestTarget::Restic {
            repo: BackupRepoTarget::S3 {
                destination_id: "probe".into(),
                endpoint,
                bucket,
                repo_path,
                region,
                credentials: BackupS3Credentials::Plaintext {
                    access_key_id: Secret::new(body.values.key.trim().to_owned()),
                    secret_access_key: Secret::new(body.values.secret.clone()),
                },
            },
        },
    };

    let res = state
        .priv_client
        .backup_test_destination(
            &rid,
            Some(&who.user.name),
            BackupDestinationTestRequest {
                provider: body.provider,
                target,
            },
        )
        .await?;

    audit(
        &state,
        &who,
        &client.ip,
        "backup.destination.test",
        None,
        json!({ "provider": body.provider.as_str(), "ok": res.ok }),
    );
    Ok(Json(json!(res)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateDestinationBody {
    provider: BackupProviderId,
    name: Option<String>,
    #[serde(default, alias = "fields")]
    values: DestinationFieldInput,
    make_default: Option<bool>,
    use_for_existing: Option<bool>,
    verify: Option<bool>,
}

/// `POST /api/v1/backups/destinations` — seal credentials in `priv` and save destination.
pub(crate) async fn create_destination(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<CreateDestinationBody>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let hostname = state
        .priv_client
        .host_info(&rid, Some(&who.user.name))
        .await
        .map(|h| h.hostname)
        .unwrap_or_else(|_| "opsbox-server".to_owned());

    let cfg = validate_destination_input(
        body.provider,
        body.name.as_deref(),
        &body.values,
        &hostname,
        !body.provider.is_sftp(),
    )
    .map_err(map_val_err)?;

    let id = ulid::Ulid::generate().to_string();
    let now = now_unix();
    let seal_context = format!("backup:dest:{id}");

    match cfg {
        ValidatedDestinationConfig::Sftp {
            name,
            host,
            port,
            user,
            path,
            server,
            repo_path,
            display_loc,
        } => {
            let key = state
                .priv_client
                .backup_generate_sftp_key(
                    &rid,
                    Some(&who.user.name),
                    BackupGenerateSftpKeyRequest {
                        destination_id: id.clone(),
                    },
                )
                .await?;
            let config = json!({
                "host": host,
                "port": port,
                "user": user,
                "path": path,
                "server": server,
                "repoPath": repo_path,
                "displayLoc": display_loc,
                "publicKey": key.public_key,
                "fingerprint": key.fingerprint,
                "sealContext": key.seal_context,
            });
            let st = state.store.clone();
            let did = id.clone();
            let rec = tokio::task::spawn_blocking(move || {
                st.create_backup_destination(
                    &did,
                    body.provider.as_str(),
                    &name,
                    &config,
                    Some(&key.private_key_enc),
                    None,
                    now,
                )
            })
            .await
            .map_err(|_| HttpError::internal())?
            .map_err(store_err)?;

            let default_dest = resolve_default_destination_id(&state.store);
            audit(
                &state,
                &who,
                &client.ip,
                "backup.destination.create",
                Some(&rec.id),
                json!({ "provider": rec.provider, "name": rec.name, "verified": false }),
            );
            Ok((
                StatusCode::CREATED,
                Json(json!({
                    "destination": dest_json(&rec, &default_dest, None),
                    "publicKey": key.public_key,
                    "fingerprint": key.fingerprint,
                    "nextAction": "add_ssh_key_and_test",
                })),
            ))
        }
        ValidatedDestinationConfig::S3 {
            provider,
            name,
            endpoint,
            region,
            bucket,
            prefix,
            server,
            repo_path,
            display_loc,
        } => {
            let secret_payload = json!({
                "accessKeyId": body.values.key.trim(),
                "secretAccessKey": body.values.secret,
            })
            .to_string();
            let sealed_creds = state
                .priv_client
                .secret_seal(
                    &rid,
                    Some(&who.user.name),
                    &seal_context,
                    Secret::new(secret_payload),
                )
                .await?;

            if body.verify.unwrap_or(true) {
                let probe = state
                    .priv_client
                    .backup_test_destination(
                        &rid,
                        Some(&who.user.name),
                        BackupDestinationTestRequest {
                            provider,
                            target: BackupDestinationTestTarget::Restic {
                                repo: BackupRepoTarget::S3 {
                                    destination_id: id.clone(),
                                    endpoint: endpoint.clone(),
                                    bucket: bucket.clone(),
                                    repo_path: repo_path.clone(),
                                    region: region.clone(),
                                    credentials: BackupS3Credentials::Sealed {
                                        seal_context: seal_context.clone(),
                                        value_enc: sealed_creds.clone(),
                                    },
                                },
                            },
                        },
                    )
                    .await?;
                if !probe.ok {
                    let fail_msg = probe
                        .steps
                        .iter()
                        .find(|s| s.status == opsbox_proto::BackupTestStepStatus::Fail)
                        .map_or_else(
                            || "Destination test failed.".to_owned(),
                            |s| s.message.clone(),
                        );
                    return Err(HttpError::new(ErrorCode::Validation, fail_msg));
                }
            }

            let config = json!({
                "endpoint": endpoint,
                "region": region,
                "bucket": bucket,
                "prefix": prefix,
                "server": server,
                "repoPath": repo_path,
                "displayLoc": display_loc,
                "sealContext": seal_context,
            });
            let make_default = body.make_default.unwrap_or(true);
            let use_existing = body.use_for_existing.unwrap_or(true);
            let st = state.store.clone();
            let did = id.clone();
            let (rec, default_dest) =
                tokio::task::spawn_blocking(move || -> Result<_, opsbox_store::StoreError> {
                    let rec = st.create_backup_destination(
                        &did,
                        provider.as_str(),
                        &name,
                        &config,
                        Some(&sealed_creds),
                        Some(now),
                        now,
                    )?;
                    if use_existing {
                        let _ =
                            st.reassign_backup_jobs_destination(LOCAL_DESTINATION_ID, &did, now);
                    }
                    if make_default {
                        let _ = st.set_setting(SETTING_DEFAULT_DEST, &json!(did));
                    }
                    let cur_def = resolve_default_destination_id(&st);
                    Ok((rec, cur_def))
                })
                .await
                .map_err(|_| HttpError::internal())?
                .map_err(store_err)?;

            audit(
                &state,
                &who,
                &client.ip,
                "backup.destination.create",
                Some(&rec.id),
                json!({ "provider": rec.provider, "name": rec.name, "verified": true }),
            );
            Ok((
                StatusCode::CREATED,
                Json(json!({
                    "destination": dest_json(&rec, &default_dest, None),
                })),
            ))
        }
    }
}

/// `POST /api/v1/backups/destinations/{id}/test` — test a saved destination and mark verified.
pub(crate) async fn test_saved_destination(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    if !valid_backup_id(&id) {
        return Err(HttpError::not_found());
    }
    let st = state.store.clone();
    let did = id.clone();
    let dest = tokio::task::spawn_blocking(move || st.get_backup_destination(&did))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(store_err)?
        .ok_or_else(HttpError::not_found)?;

    let provider = if dest.provider == "local" {
        BackupProviderId::Custom
    } else {
        BackupProviderId::parse(&dest.provider).ok_or_else(|| {
            HttpError::new(
                ErrorCode::Validation,
                format!("Unsupported provider '{}'.", dest.provider),
            )
        })?
    };
    let repo = repo_target_from_record(&dest)?;
    let res = state
        .priv_client
        .backup_test_destination(
            &rid,
            Some(&who.user.name),
            BackupDestinationTestRequest {
                provider,
                target: BackupDestinationTestTarget::Restic { repo },
            },
        )
        .await?;

    let mut updated_dest = dest.clone();
    if res.ok {
        let now = now_unix();
        let was_unverified = dest.verified_at.is_none();
        let st_upd = state.store.clone();
        let did_upd = id.clone();
        updated_dest =
            tokio::task::spawn_blocking(move || -> Result<_, opsbox_store::StoreError> {
                st_upd.update_backup_destination_verification(&did_upd, Some(now), now)?;
                if was_unverified && did_upd != LOCAL_DESTINATION_ID {
                    let _ = st_upd.reassign_backup_jobs_destination(
                        LOCAL_DESTINATION_ID,
                        &did_upd,
                        now,
                    );
                    if resolve_default_destination_id(&st_upd) == LOCAL_DESTINATION_ID
                        || st_upd
                            .get_setting(SETTING_DEFAULT_DEST)
                            .ok()
                            .flatten()
                            .is_none()
                    {
                        let _ = st_upd.set_setting(SETTING_DEFAULT_DEST, &json!(did_upd));
                    }
                }
                Ok(st_upd.get_backup_destination(&did_upd)?.unwrap_or(dest))
            })
            .await
            .map_err(|_| HttpError::internal())?
            .map_err(store_err)?;

        sync_all_snapshots(
            &state.store,
            &state.priv_client,
            &rid,
            Some(&who.user.name),
            Some(&id),
            None,
        )
        .await;
    }

    let default_dest = resolve_default_destination_id(&state.store);
    audit(
        &state,
        &who,
        &client.ip,
        "backup.destination.test",
        Some(&id),
        json!({ "ok": res.ok, "verified": res.verified }),
    );
    Ok(Json(json!({
        "ok": res.ok,
        "verified": res.verified,
        "steps": res.steps,
        "destination": dest_json(&updated_dest, &default_dest, None),
    })))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetDefaultDestBody {
    #[serde(default)]
    reassign_existing: bool,
}

/// `POST /api/v1/backups/destinations/{id}/default` — select default destination.
pub(crate) async fn set_default_destination(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Path(id): Path<String>,
    body: Option<ApiJson<SetDefaultDestBody>>,
) -> Result<Json<Value>, HttpError> {
    if !valid_backup_id(&id) {
        return Err(HttpError::not_found());
    }
    let reassign = body.is_some_and(|ApiJson(b)| b.reassign_existing);
    let st = state.store.clone();
    let did = id.clone();
    let dest =
        tokio::task::spawn_blocking(move || -> Result<BackupDestinationRecord, HttpError> {
            let dest = st
                .get_backup_destination(&did)
                .map_err(store_err)?
                .ok_or_else(HttpError::not_found)?;
            if dest.id != LOCAL_DESTINATION_ID && dest.verified_at.is_none() {
                return Err(HttpError::new(
                    ErrorCode::Conflict,
                    "Verify this destination before setting it as default.",
                ));
            }
            let now = now_unix();
            st.set_setting(SETTING_DEFAULT_DEST, &json!(did))
                .map_err(store_err)?;
            if reassign {
                for j in st.list_backup_jobs().map_err(store_err)? {
                    let _ = st.set_backup_job_destination(&j.id, &did, now);
                }
            }
            Ok(dest)
        })
        .await
        .map_err(|_| HttpError::internal())??;

    audit(
        &state,
        &who,
        &client.ip,
        "backup.destination.default",
        Some(&id),
        json!({ "reassignExisting": reassign }),
    );
    Ok(Json(json!({ "destination": dest_json(&dest, &id, None) })))
}

/// `DELETE /api/v1/backups/destinations/{id}` — remove an offsite destination and return jobs to local.
pub(crate) async fn delete_destination(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    if !valid_backup_id(&id) {
        return Err(HttpError::not_found());
    }
    if id == LOCAL_DESTINATION_ID {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "The built-in local disk destination cannot be deleted.",
        ));
    }
    let st = state.store.clone();
    let did = id.clone();
    let removed = tokio::task::spawn_blocking(move || -> Result<bool, opsbox_store::StoreError> {
        let now = now_unix();
        let ok = st.delete_backup_destination(&did, now)?;
        if ok
            && st
                .get_setting(SETTING_DEFAULT_DEST)?
                .and_then(|v| v.as_str().map(str::to_owned))
                .as_deref()
                == Some(did.as_str())
        {
            let _ = st.set_setting(SETTING_DEFAULT_DEST, &json!(LOCAL_DESTINATION_ID));
        }
        Ok(ok)
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(store_err)?;

    if !removed {
        return Err(HttpError::not_found());
    }
    audit(
        &state,
        &who,
        &client.ip,
        "backup.destination.delete",
        Some(&id),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RecoveryKeyBody {
    password: String,
}

/// `POST /api/v1/backups/recovery-key` — export the restic repo recovery password behind
/// admin session check, `deny_token`, and password re-authentication (never returning HTTP 401
/// on wrong password so interactive browser sessions are preserved).
pub(crate) async fn recovery_key(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<RecoveryKeyBody>,
) -> Result<(HeaderMap, Json<Value>), HttpError> {
    if who.via_token {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "Recovery key download requires an interactive admin session.",
        ));
    }
    let ok = state
        .auth
        .verify_user_password(&who.user.id, &body.password)
        .await
        .map_err(|_| HttpError::internal())?;
    if !ok {
        audit(
            &state,
            &who,
            &client.ip,
            "backup.recovery_key.denied",
            None,
            json!({ "reason": "wrong_password" }),
        );
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "Incorrect account password.",
        ));
    }

    let key = state
        .priv_client
        .backup_recovery_key(&rid, Some(&who.user.name))
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "backup.recovery_key.export",
        None,
        json!({ "hostname": key.hostname }),
    );
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((
        headers,
        Json(json!({
            "password": key.recovery_key.expose(),
            "hostname": key.hostname,
            "instructions": "Keep this recovery password offline (for example in a password manager). restic needs it to decrypt backups on a fresh server.",
        })),
    ))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RestoreBody {
    #[serde(default = "default_copy_mode")]
    mode: String,
    confirm: Option<String>,
    copy_target: Option<String>,
}

fn default_copy_mode() -> String {
    "copy".to_owned()
}

fn infer_db_kind_from_snapshot(
    snap: &BackupSnapshotRecord,
    job: Option<&BackupJobRecord>,
) -> Option<BackupDbKind> {
    if let Some(j) = job
        && let Ok(k) = infer_db_kind(j)
    {
        return Some(k);
    }
    for p in &snap.paths {
        if p.ends_with(".pg_dump.fc") {
            return Some(BackupDbKind::Postgres);
        }
        if p.ends_with(".rdb") {
            return Some(BackupDbKind::Redis);
        }
        if p.ends_with(".mongodump.archive") {
            return Some(BackupDbKind::Mongo);
        }
        if p.ends_with(".sql") {
            if let Some(j) = job
                && (j.template_id.as_deref() == Some("mariadb")
                    || j.what.get("templateId").and_then(Value::as_str) == Some("mariadb"))
            {
                return Some(BackupDbKind::Mariadb);
            }
            return Some(BackupDbKind::Mysql);
        }
    }
    None
}

fn allocate_copy_service_name(
    store: &Store,
    custom: Option<&str>,
    base: &str,
    now: i64,
) -> Result<String, HttpError> {
    if let Some(raw) = custom.map(str::trim).filter(|s| !s.is_empty()) {
        if !valid_service_name(raw) {
            return Err(HttpError::new(
                ErrorCode::Validation,
                "Service name must use lowercase letters, digits, and dashes.",
            ));
        }
        if store.get_service(raw).map_err(store_err)?.is_some() {
            return Err(HttpError::new(
                ErrorCode::Conflict,
                format!("A service named '{raw}' already exists."),
            ));
        }
        return Ok(raw.to_owned());
    }
    let clean_base: String = base
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed_base = clean_base.trim_matches('-');
    let prefix = if trimmed_base.is_empty() {
        "db"
    } else {
        trimmed_base
    };
    let sfx = date_suffix_utc(now);
    let head: String = prefix.chars().take(16).collect();
    let candidate = format!("{}-r{}", head.trim_end_matches('-'), sfx);
    let mut name: String = candidate.chars().take(32).collect();
    name = name.trim_end_matches('-').to_owned();
    if valid_service_name(&name) && store.get_service(&name).map_err(store_err)?.is_none() {
        return Ok(name);
    }
    for idx in 1..=99u32 {
        let cand = format!("{}-{idx}", name.chars().take(28).collect::<String>());
        if valid_service_name(&cand) && store.get_service(&cand).map_err(store_err)?.is_none() {
            return Ok(cand);
        }
    }
    Err(HttpError::new(
        ErrorCode::Conflict,
        "Could not allocate a unique copy service name.",
    ))
}

/// `POST /api/v1/backups/snapshots/{id}/restore` — restore a snapshot (`copy` by default, or
/// `replace` in place after exact `"replace"` typed confirmation and a live safety snapshot first).
pub(crate) async fn restore_snapshot(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    body: Option<ApiJson<RestoreBody>>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let body = body.map(|ApiJson(b)| b).unwrap_or_default();
    let mode = match body.mode.trim().to_ascii_lowercase().as_str() {
        "" | "copy" => BackupRestoreMode::Copy,
        "replace" => BackupRestoreMode::Replace,
        other => {
            return Err(HttpError::new(
                ErrorCode::Validation,
                format!("Invalid restore mode '{other}'; expected 'copy' or 'replace'."),
            ));
        }
    };
    if mode == BackupRestoreMode::Replace && body.confirm.as_deref() != Some("replace") {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            "Type \"replace\" to confirm replacing live data in place.",
        ));
    }

    let st = state.store.clone();
    let snap_lookup = id.clone();
    let (snap, job, dest, orig_svc_opt, orig_vars, proj_env, existing_count) =
        tokio::task::spawn_blocking(move || -> Result<_, HttpError> {
            let snap = st
                .get_backup_snapshot_by_any_id(&snap_lookup)
                .map_err(store_err)?
                .ok_or_else(HttpError::not_found)?;
            let job = snap
                .job_id
                .as_deref()
                .and_then(|jid| st.get_backup_job(jid).ok().flatten());
            let dest = st
                .get_backup_destination(&snap.destination_id)
                .map_err(store_err)?
                .ok_or_else(|| {
                    HttpError::new(
                        ErrorCode::NotFound,
                        format!("Destination '{}' no longer exists.", snap.destination_id),
                    )
                })?;
            let orig_svc = job
                .as_ref()
                .and_then(|j| j.service_id.as_deref())
                .and_then(|sid| st.get_service(sid).ok().flatten())
                .or_else(|| st.get_service(&snap.what).ok().flatten());
            let orig_vars = orig_svc
                .as_ref()
                .map(|s| st.list_service_vars(&s.id).unwrap_or_default())
                .unwrap_or_default();
            let now = now_unix();
            let (proj, env_rec) = if let Some(ref s) = orig_svc {
                st.resolve_project(&s.project_id, now)
                    .map_err(store_err)?
                    .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Project not found."))?
            } else {
                st.resolve_project("default", now)
                    .map_err(store_err)?
                    .ok_or_else(|| {
                        HttpError::new(ErrorCode::NotFound, "Default project not found.")
                    })?
            };
            let env_id_for_cnt = orig_svc
                .as_ref()
                .map_or(env_rec.id.as_str(), |s| s.env_id.as_str());
            let cnt = st
                .list_services(&proj.id, env_id_for_cnt)
                .map(|v| v.len())
                .unwrap_or(0);
            Ok((snap, job, dest, orig_svc, orig_vars, (proj, env_rec), cnt))
        })
        .await
        .map_err(|_| HttpError::internal())??;

    if snap.kind == BackupTargetKind::Opsbox && mode == BackupRestoreMode::Replace {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "opsbox itself cannot be replaced in place while running; restore to a copy instead.",
        ));
    }

    let repo = repo_target_from_record(&dest)?;
    let now = now_unix();

    // Validate DB restore targets synchronously before spawning the background task.
    enum RestorePlan {
        PrivDirect(BackupRestoreRequest),
        DbCopy {
            db_kind: BackupDbKind,
            tmpl_id: String,
            version: String,
            image_ref: String,
            copy_name: String,
            proj_id: String,
            proj_name: String,
            env_id: String,
            internal_only: bool,
            public_port: bool,
            overrides: Vec<(String, String, bool)>,
            stdin_filename: String,
        },
    }

    let plan = match snap.kind {
        BackupTargetKind::Opsbox => RestorePlan::PrivDirect(BackupRestoreRequest {
            destination_id: snap.destination_id.clone(),
            repo: repo.clone(),
            restic_id: snap.restic_id.clone(),
            mode,
            confirm_replace: body.confirm.clone(),
            target: BackupRestoreTarget::Opsbox {
                copy_target: body.copy_target.clone(),
            },
            job_id: snap.job_id.clone(),
        }),
        BackupTargetKind::Path => {
            let mut snapshot_paths = snap.paths.clone();
            if snapshot_paths.is_empty()
                && let Some(ref j) = job
            {
                snapshot_paths = j
                    .what
                    .get("paths")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
            }
            if snapshot_paths.is_empty() {
                return Err(HttpError::new(
                    ErrorCode::Validation,
                    "Snapshot does not record which path to restore.",
                ));
            }
            RestorePlan::PrivDirect(BackupRestoreRequest {
                destination_id: snap.destination_id.clone(),
                repo: repo.clone(),
                restic_id: snap.restic_id.clone(),
                mode,
                confirm_replace: body.confirm.clone(),
                target: BackupRestoreTarget::Path {
                    snapshot_paths,
                    copy_target: body.copy_target.clone(),
                },
                job_id: snap.job_id.clone(),
            })
        }
        BackupTargetKind::Volume => {
            let source_volumes = job
                .as_ref()
                .and_then(|j| j.what.get("volumes"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| {
                    snap.what
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect()
                });
            if source_volumes.is_empty() {
                return Err(HttpError::new(
                    ErrorCode::Validation,
                    "Snapshot does not record which Docker volume to restore.",
                ));
            }
            RestorePlan::PrivDirect(BackupRestoreRequest {
                destination_id: snap.destination_id.clone(),
                repo: repo.clone(),
                restic_id: snap.restic_id.clone(),
                mode,
                confirm_replace: body.confirm.clone(),
                target: BackupRestoreTarget::Volume {
                    source_volumes,
                    snapshot_paths: snap.paths.clone(),
                    copy_volume: body.copy_target.clone(),
                    project: proj_env.0.name.clone(),
                    service_label: snap.what.clone(),
                },
                job_id: snap.job_id.clone(),
            })
        }
        BackupTargetKind::Database => {
            let db_kind = infer_db_kind_from_snapshot(&snap, job.as_ref()).ok_or_else(|| {
                HttpError::new(
                    ErrorCode::Validation,
                    "Could not determine database type for this snapshot.",
                )
            })?;
            let stdin_filename = snap
                .paths
                .first()
                .map(|p| p.trim_start_matches('/').to_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default_stdin_filename(db_kind, &snap.what));

            if mode == BackupRestoreMode::Replace {
                let svc = orig_svc_opt.ok_or_else(|| {
                    HttpError::new(
                        ErrorCode::NotFound,
                        format!(
                            "Live database service '{}' no longer exists — restore to a copy instead.",
                            snap.what
                        ),
                    )
                })?;
                let cid = svc.active_container.clone().ok_or_else(|| {
                    HttpError::new(
                        ErrorCode::Conflict,
                        format!(
                            "Database service '{}' must be running before replace-in-place restore.",
                            svc.name
                        ),
                    )
                })?;
                let vars = store_env_specs(&state.store, &svc.id);
                RestorePlan::PrivDirect(BackupRestoreRequest {
                    destination_id: snap.destination_id.clone(),
                    repo: repo.clone(),
                    restic_id: snap.restic_id.clone(),
                    mode,
                    confirm_replace: body.confirm.clone(),
                    target: BackupRestoreTarget::Database {
                        template: db_kind,
                        service_id: svc.id,
                        service_name: svc.name,
                        container_id: cid,
                        stdin_filename,
                        vars,
                    },
                    job_id: snap.job_id.clone(),
                })
            } else {
                let tmpl = get_template(db_kind.as_str()).ok_or_else(|| {
                    HttpError::new(
                        ErrorCode::Validation,
                        "Missing database template definition.",
                    )
                })?;
                let copy_name = allocate_copy_service_name(
                    &state.store,
                    body.copy_target.as_deref(),
                    &snap.what,
                    now,
                )?;
                let version = orig_svc_opt
                    .as_ref()
                    .map(|s| s.branch.clone())
                    .filter(|v| tmpl.versions.contains(v))
                    .or_else(|| tmpl.versions.first().cloned())
                    .unwrap_or_else(|| "latest".to_owned());
                let image_ref = format!("{}:{version}", tmpl.image);
                let mut overrides = Vec::new();
                if let Some(ref osvc) = orig_svc_opt {
                    for v in &orig_vars {
                        if !v.is_secret
                            && v.ref_expr.is_none()
                            && matches!(
                                v.key.as_str(),
                                "POSTGRES_USER"
                                    | "POSTGRES_DB"
                                    | "MYSQL_DATABASE"
                                    | "MYSQL_USER"
                                    | "MARIADB_DATABASE"
                                    | "MARIADB_USER"
                                    | "MONGO_INITDB_ROOT_USERNAME"
                            )
                            && let Ok(opened) = state
                                .priv_client
                                .secret_open(
                                    &rid,
                                    Some(&who.user.name),
                                    &var_seal_context(&osvc.id, &v.key),
                                    &v.value_enc,
                                )
                                .await
                        {
                            let val = opened.expose().trim().to_owned();
                            if !val.is_empty() {
                                overrides.push((v.key.clone(), val, false));
                            }
                        }
                    }
                }
                let (internal_only, public_port) = orig_svc_opt
                    .as_ref()
                    .map_or((true, false), |s| (s.internal_only, s.public_port));
                let target_env_id = orig_svc_opt
                    .as_ref()
                    .map_or(proj_env.1.id.clone(), |s| s.env_id.clone());
                RestorePlan::DbCopy {
                    db_kind,
                    tmpl_id: tmpl.id.clone(),
                    version,
                    image_ref,
                    copy_name,
                    proj_id: proj_env.0.id,
                    proj_name: proj_env.0.name,
                    env_id: target_env_id,
                    internal_only,
                    public_port,
                    overrides,
                    stdin_filename,
                }
            }
        }
    };

    let s = state.clone();
    let actor_name = who.user.name.clone();
    let owner_id = who.user.id.clone();
    let ip = client.ip.clone();
    let principal = who.clone();
    let snap_id = snap.id.clone();
    let snap_short = snap.short_id.clone();
    let snap_dest = snap.destination_id.clone();
    let snap_restic_id = snap.restic_id.clone();
    let snap_job_id = snap.job_id.clone();

    let task = state.tasks.start("backups.restore", async move {
        let _guard = s.backups.run_lock.lock().await;
        let res = match plan {
            RestorePlan::PrivDirect(req) => {
                s.priv_client
                    .backup_restore(&rid, Some(&actor_name), req)
                    .await?
            }
            RestorePlan::DbCopy {
                db_kind,
                tmpl_id,
                version,
                image_ref,
                copy_name,
                proj_id,
                proj_name,
                env_id,
                internal_only,
                public_port,
                overrides,
                stdin_filename,
            } => {
                let tmpl = get_template(&tmpl_id)
                    .ok_or_else(|| HttpError::new(ErrorCode::Internal, "Missing DB template."))?;
                let generated = generate_instance_env(tmpl, &copy_name, &overrides)
                    .map_err(|_| HttpError::internal())?;
                let (gx, gy) = next_graph_pos(existing_count);
                let svc = s
                    .store
                    .create_service_with_exposure(
                        NewService {
                            project_id: &proj_id,
                            env_id: &env_id,
                            name: &copy_name,
                            kind: ServiceKind::Db,
                            owner_id: Some(&owner_id),
                            source_kind: SourceKind::DockerImage,
                            repo: &image_ref,
                            branch: &version,
                            build_method: &tmpl.name,
                            dockerfile_path: None,
                            start_cmd: None,
                            port: tmpl.port,
                            health_path: "tcp",
                            domain: None,
                            image_ref: Some(&image_ref),
                            deploy_key_id: None,
                            auto_deploy: false,
                            memory_limit_mb: Some(256),
                            cpu_limit_millis: None,
                            graph_x: gx,
                            graph_y: gy,
                            note: Some("restored copy"),
                            now,
                        },
                        None,
                        Some(&tmpl.id),
                        internal_only,
                        public_port,
                    )
                    .map_err(store_err)?;

                let seal_pairs: Vec<(String, String, bool)> = generated
                    .vars
                    .iter()
                    .map(|v| (v.key.clone(), v.value.clone(), v.is_secret))
                    .collect();
                if let Err(e) =
                    seal_and_store_vars(&s, &rid, &actor_name, &svc, &seal_pairs, now).await
                {
                    let _ = s.store.delete_service(&svc.id);
                    return Err(e);
                }

                let _ = s.store.upsert_backup_job(
                    &svc.id,
                    Some(&tmpl.id),
                    &tmpl.backup.method,
                    &tmpl.backup.cron,
                    false,
                    now,
                );
                let default_dest = resolve_default_destination_id(&s.store);
                if default_dest != LOCAL_DESTINATION_ID
                    && let Ok(Some(j)) = s.store.get_backup_job_for_service(&svc.id)
                {
                    let _ = s
                        .store
                        .set_backup_job_destination(&j.id, &default_dest, now);
                }

                let dep = s
                    .store
                    .create_deploy(
                        &svc.id,
                        None,
                        &format!("Restore snapshot {snap_short} into copy {copy_name}"),
                        DeployTrigger::Manual,
                        &actor_name,
                        now,
                    )
                    .map_err(store_err)?;

                let vars_specs = store_env_specs(&s.store, &svc.id);
                let started: DeployStartResult = match s
                    .priv_client
                    .template_start(
                        &rid,
                        &actor_name,
                        TemplateStartRequest {
                            project: proj_name,
                            service: svc.name.clone(),
                            deploy_id: dep.id.clone(),
                            template_id: tmpl_id,
                            version,
                            image_ref: image_ref.clone(),
                            internal_only,
                            public_port,
                            confirm_public_port: public_port,
                            vars: vars_specs.clone(),
                            ref_sources: Vec::new(),
                            memory_limit_mb: Some(256),
                            cpu_limit_millis: None,
                            data_volume: None,
                        },
                    )
                    .await
                {
                    Ok(st) => st,
                    Err(e) => {
                        let _ = s.store.delete_service(&svc.id);
                        return Err(HttpError::from(e));
                    }
                };

                let _ = s.store.complete_deploy_switch_with_refs(
                    &svc.id,
                    &dep.id,
                    &image_ref,
                    &started.container_id,
                    started.host_port,
                    None,
                    None,
                    now_unix(),
                );

                match s
                    .priv_client
                    .backup_restore(
                        &rid,
                        Some(&actor_name),
                        BackupRestoreRequest {
                            destination_id: snap_dest.clone(),
                            repo,
                            restic_id: snap_restic_id,
                            mode: BackupRestoreMode::Copy,
                            confirm_replace: None,
                            target: BackupRestoreTarget::Database {
                                template: db_kind,
                                service_id: svc.id.clone(),
                                service_name: svc.name.clone(),
                                container_id: started.container_id.clone(),
                                stdin_filename,
                                vars: vars_specs,
                            },
                            job_id: snap_job_id,
                        },
                    )
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = s
                            .priv_client
                            .deploy_stop_container(&rid, &actor_name, &started.container_id)
                            .await;
                        let _ = s.store.delete_service(&svc.id);
                        return Err(HttpError::from(e));
                    }
                }
            }
        };

        if mode == BackupRestoreMode::Replace {
            sync_all_snapshots(
                &s.store,
                &s.priv_client,
                &rid,
                Some(&actor_name),
                Some(&snap_dest),
                None,
            )
            .await;
        }
        audit(
            &s,
            &principal,
            &ip,
            "backup.restore",
            Some(&snap_id),
            json!({
                "mode": if mode == BackupRestoreMode::Copy { "copy" } else { "replace" },
                "target": res.target_summary,
                "safetySnapshotId": res.safety_snapshot_id,
            }),
        );
        Ok(Outcome {
            log: res.log,
            result: json!({
                "ok": res.ok,
                "mode": if mode == BackupRestoreMode::Copy { "copy" } else { "replace" },
                "targetSummary": res.target_summary,
                "message": res.message,
                "safetySnapshotId": res.safety_snapshot_id,
                "safetySnapshotShortId": res.safety_snapshot_short_id,
            }),
        })
    })?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}

async fn open_non_secret_var(
    priv_client: &PrivClient,
    rid: &str,
    actor: Option<&str>,
    service_id: Option<&str>,
    vars: &[opsbox_store::ServiceVarRecord],
    key: &str,
) -> Option<String> {
    let sid = service_id?;
    let v = vars
        .iter()
        .find(|v| v.key == key && !v.is_secret && v.ref_expr.is_none())?;
    let sec = priv_client
        .secret_open(rid, actor, &var_seal_context(sid, key), &v.value_enc)
        .await
        .ok()?;
    let s = sec.expose().trim().to_owned();
    (!s.is_empty()).then_some(s)
}

async fn build_restore_test_target(
    store: &Arc<Store>,
    priv_client: &PrivClient,
    rid: &str,
    actor: Option<&str>,
    snap: &BackupSnapshotRecord,
) -> BackupRestoreTestTarget {
    let st = store.clone();
    let snap_c = snap.clone();
    let (db_kind_opt, svc_opt, vars) = tokio::task::spawn_blocking(move || {
        let job = snap_c
            .job_id
            .as_deref()
            .and_then(|jid| st.get_backup_job(jid).ok().flatten());
        let db_kind_opt = if snap_c.kind == BackupTargetKind::Database {
            infer_db_kind_from_snapshot(&snap_c, job.as_ref())
        } else {
            None
        };
        let svc = job
            .as_ref()
            .and_then(|j| j.service_id.as_deref())
            .and_then(|sid| st.get_service(sid).ok().flatten())
            .or_else(|| st.get_service(&snap_c.what).ok().flatten());
        let vars = svc
            .as_ref()
            .map(|s| st.list_service_vars(&s.id).unwrap_or_default())
            .unwrap_or_default();
        (db_kind_opt, svc, vars)
    })
    .await
    .unwrap_or((None, None, Vec::new()));

    if let Some(db_kind) = db_kind_opt {
        let sid = svc_opt.as_ref().map(|s| s.id.as_str());
        let (db_user, db_name) = match db_kind {
            BackupDbKind::Postgres => (
                open_non_secret_var(priv_client, rid, actor, sid, &vars, "POSTGRES_USER").await,
                open_non_secret_var(priv_client, rid, actor, sid, &vars, "POSTGRES_DB").await,
            ),
            BackupDbKind::Mysql => (
                open_non_secret_var(priv_client, rid, actor, sid, &vars, "MYSQL_USER").await,
                open_non_secret_var(priv_client, rid, actor, sid, &vars, "MYSQL_DATABASE").await,
            ),
            BackupDbKind::Mariadb => (
                open_non_secret_var(priv_client, rid, actor, sid, &vars, "MARIADB_USER").await,
                open_non_secret_var(priv_client, rid, actor, sid, &vars, "MARIADB_DATABASE").await,
            ),
            BackupDbKind::Redis => (None, None),
            BackupDbKind::Mongo => (
                open_non_secret_var(
                    priv_client,
                    rid,
                    actor,
                    sid,
                    &vars,
                    "MONGO_INITDB_ROOT_USERNAME",
                )
                .await,
                None,
            ),
        };
        let version = svc_opt.map(|s| s.branch).unwrap_or_default();
        let stdin_filename = snap
            .paths
            .first()
            .map(|p| p.trim_start_matches('/').to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_stdin_filename(db_kind, &snap.what));
        return BackupRestoreTestTarget::Database {
            template: db_kind,
            version,
            stdin_filename,
            db_user,
            db_name,
        };
    }
    BackupRestoreTestTarget::Files {
        is_opsbox_self: snap.kind == BackupTargetKind::Opsbox
            || snap.job_id.as_deref() == Some(OPSBOX_SELF_JOB_ID),
    }
}

async fn run_single_restore_test(
    store: &Arc<Store>,
    priv_client: &PrivClient,
    rid: &str,
    actor: Option<&str>,
    snap: &BackupSnapshotRecord,
    dest: &BackupDestinationRecord,
) -> Result<Outcome, HttpError> {
    let repo = repo_target_from_record(dest)?;
    let target = build_restore_test_target(store, priv_client, rid, actor, snap).await;

    let priv_res = priv_client
        .backup_restore_test(
            rid,
            actor,
            BackupRestoreTestRequest {
                destination_id: snap.destination_id.clone(),
                repo,
                restic_id: snap.restic_id.clone(),
                target,
            },
        )
        .await;

    let (ok, summary, logs) = match priv_res {
        Ok(res) => (res.ok, res.summary, res.log),
        Err(e) => {
            let msg = HttpError::from(e).0.error.message;
            (false, msg, Vec::new())
        }
    };

    let ts = now_unix();
    let new_state = if ok {
        BackupCheckState::RestoreTested
    } else {
        BackupCheckState::ChecksumFailed
    };
    let st_save = store.clone();
    let snap_id = snap.id.clone();
    let summary_c = summary.clone();
    let _ = tokio::task::spawn_blocking(move || {
        let _ = st_save.set_backup_snapshot_check_state(&snap_id, new_state, ts);
        let _ = st_save.set_setting(SETTING_LAST_RESTORE_TEST_AT, &json!(ts));
        let _ = st_save.set_setting(SETTING_LAST_RESTORE_TEST_OK, &json!(ok));
        let _ = st_save.set_setting(SETTING_LAST_RESTORE_TEST_SUMMARY, &json!(summary_c));
        if ok {
            let _ = st_save.set_setting(SETTING_LAST_RESTORE_TEST_ERROR, &json!(""));
        } else {
            let _ = st_save.set_setting(SETTING_LAST_RESTORE_TEST_ERROR, &json!(summary_c));
        }
    })
    .await;

    if !ok {
        return Err(HttpError::new(ErrorCode::Internal, summary));
    }
    Ok(Outcome {
        log: if logs.is_empty() {
            vec![summary.clone()]
        } else {
            logs
        },
        result: json!({
            "ok": true,
            "snapshotId": snap.id,
            "summary": summary,
            "checkState": "restore_tested",
        }),
    })
}

async fn execute_scheduled_restore_test(
    store: &Arc<Store>,
    priv_client: &PrivClient,
    rid: &str,
    actor: Option<&str>,
) -> Result<Outcome, HttpError> {
    let st = store.clone();
    let candidate = tokio::task::spawn_blocking(move || -> Result<_, HttpError> {
        let snaps = st
            .list_backup_snapshots(DEFAULT_SNAPSHOT_LIMIT)
            .map_err(store_err)?;
        let preferred = snaps
            .iter()
            .find(|s| s.kind == BackupTargetKind::Database)
            .or_else(|| snaps.first())
            .cloned()
            .ok_or_else(|| {
                HttpError::new(ErrorCode::NotFound, "No snapshots exist to verify yet.")
            })?;
        let dest = st
            .get_backup_destination(&preferred.destination_id)
            .map_err(store_err)?
            .ok_or_else(|| {
                HttpError::new(
                    ErrorCode::NotFound,
                    "Destination for snapshot no longer exists.",
                )
            })?;
        Ok((preferred, dest))
    })
    .await
    .map_err(|_| HttpError::internal())??;

    run_single_restore_test(store, priv_client, rid, actor, &candidate.0, &candidate.1).await
}

/// `POST /api/v1/backups/snapshots/{id}/test` — run a live restore test on a specific snapshot.
pub(crate) async fn test_snapshot_restore(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let st = state.store.clone();
    let lookup = id.clone();
    let (snap, dest) = tokio::task::spawn_blocking(move || -> Result<_, HttpError> {
        let snap = st
            .get_backup_snapshot_by_any_id(&lookup)
            .map_err(store_err)?
            .ok_or_else(HttpError::not_found)?;
        let dest = st
            .get_backup_destination(&snap.destination_id)
            .map_err(store_err)?
            .ok_or_else(HttpError::not_found)?;
        Ok((snap, dest))
    })
    .await
    .map_err(|_| HttpError::internal())??;

    let s = state.clone();
    let actor_name = who.user.name.clone();
    let ip = client.ip.clone();
    let principal = who.clone();

    let task = state.tasks.start("backups.restore_test", async move {
        let _guard = s.backups.run_lock.lock().await;
        let out = run_single_restore_test(
            &s.store,
            &s.priv_client,
            &rid,
            Some(&actor_name),
            &snap,
            &dest,
        )
        .await;
        audit(
            &s,
            &principal,
            &ip,
            "backup.restore_test",
            Some(&snap.id),
            json!({ "ok": out.is_ok() }),
        );
        out
    })?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}

/// `POST /api/v1/backups/restore-test` — trigger a monthly-style restore test on the latest snapshot.
pub(crate) async fn run_restore_test_all(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let s = state.clone();
    let actor_name = who.user.name.clone();
    let ip = client.ip.clone();
    let principal = who.clone();

    let task = state.tasks.start("backups.restore_test", async move {
        let _guard = s.backups.run_lock.lock().await;
        let out =
            execute_scheduled_restore_test(&s.store, &s.priv_client, &rid, Some(&actor_name)).await;
        audit(
            &s,
            &principal,
            &ip,
            "backup.restore_test",
            None,
            json!({ "ok": out.is_ok() }),
        );
        out
    })?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}
