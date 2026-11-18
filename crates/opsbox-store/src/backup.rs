//! Backup destinations, backup jobs, and snapshot metadata (migration 0010).

use opsbox_proto::{BackupCheckState, BackupTargetKind};
use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};
use serde_json::Value;

use crate::{Store, StoreError};

pub const LOCAL_DESTINATION_ID: &str = "local";
pub const OPSBOX_SELF_JOB_ID: &str = "opsbox-self";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupDestinationRecord {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub config: Value,
    pub secrets_enc: Option<String>,
    pub verified_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupJobRecord {
    pub id: String,
    pub name: String,
    pub kind: BackupTargetKind,
    pub what: Value,
    pub service_id: Option<String>,
    pub template_id: Option<String>,
    pub method: String,
    pub cron: String,
    pub retain_daily: i64,
    pub retain_weekly: i64,
    pub destination_id: String,
    pub enabled: bool,
    pub last_run_at: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_snapshot_id: Option<String>,
    pub last_size_bytes: Option<i64>,
    pub last_duration_ms: Option<i64>,
    pub next_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct NewBackupJob<'a> {
    pub name: &'a str,
    pub kind: BackupTargetKind,
    pub what: &'a Value,
    pub service_id: Option<&'a str>,
    pub template_id: Option<&'a str>,
    pub method: &'a str,
    pub cron: &'a str,
    pub retain_daily: i64,
    pub retain_weekly: i64,
    pub destination_id: &'a str,
    pub enabled: bool,
    pub next_run_at: Option<i64>,
    pub now: i64,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateBackupJob<'a> {
    pub name: Option<&'a str>,
    pub what: Option<&'a Value>,
    pub cron: Option<&'a str>,
    pub retain_daily: Option<i64>,
    pub retain_weekly: Option<i64>,
    pub enabled: Option<bool>,
    pub next_run_at: Option<Option<i64>>,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupSnapshotRecord {
    pub id: String,
    pub job_id: Option<String>,
    pub destination_id: String,
    pub restic_id: String,
    pub short_id: String,
    pub what: String,
    pub kind: BackupTargetKind,
    pub taken_at: i64,
    pub size_bytes: i64,
    pub check_state: BackupCheckState,
    pub checked_at: Option<i64>,
    pub tags: Vec<String>,
    pub paths: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct SyncBackupSnapshot<'a> {
    pub job_id: Option<&'a str>,
    pub restic_id: &'a str,
    pub short_id: &'a str,
    pub what: &'a str,
    pub kind: BackupTargetKind,
    pub taken_at: i64,
    pub size_bytes: i64,
    pub tags: &'a [String],
    pub paths: &'a [String],
}

const DEST_COLS: &str =
    "id, provider, name, config_json, secrets_enc, verified_at, created_at, updated_at";

const JOB_COLS: &str = "\
    id, name, kind, what_json, service_id, template_id, method, cron, \
    retain_daily, retain_weekly, destination_id, enabled, \
    last_run_at, last_status, last_error, last_snapshot_id, last_size_bytes, \
    last_duration_ms, next_run_at, created_at, updated_at";

const SNAP_COLS: &str = "\
    id, job_id, destination_id, restic_id, short_id, what, kind, taken_at, \
    size_bytes, check_state, checked_at, tags_json, paths_json, created_at, updated_at";

fn invalid_col(col: usize, what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        col,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid {what}"),
        )),
    )
}

fn dest_from_row(r: &Row<'_>) -> rusqlite::Result<BackupDestinationRecord> {
    let cfg_raw: String = r.get(3)?;
    let config: Value =
        serde_json::from_str(&cfg_raw).map_err(|_| invalid_col(3, "config_json"))?;
    Ok(BackupDestinationRecord {
        id: r.get(0)?,
        provider: r.get(1)?,
        name: r.get(2)?,
        config,
        secrets_enc: r.get(4)?,
        verified_at: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

fn job_from_row(r: &Row<'_>) -> rusqlite::Result<BackupJobRecord> {
    let kind_s: String = r.get(2)?;
    let what_raw: String = r.get(3)?;
    let enabled_i: i64 = r.get(11)?;
    let kind = BackupTargetKind::parse(&kind_s).ok_or_else(|| invalid_col(2, "kind"))?;
    let what: Value = serde_json::from_str(&what_raw).map_err(|_| invalid_col(3, "what_json"))?;
    Ok(BackupJobRecord {
        id: r.get(0)?,
        name: r.get(1)?,
        kind,
        what,
        service_id: r.get(4)?,
        template_id: r.get(5)?,
        method: r.get(6)?,
        cron: r.get(7)?,
        retain_daily: r.get(8)?,
        retain_weekly: r.get(9)?,
        destination_id: r.get(10)?,
        enabled: enabled_i != 0,
        last_run_at: r.get(12)?,
        last_status: r.get(13)?,
        last_error: r.get(14)?,
        last_snapshot_id: r.get(15)?,
        last_size_bytes: r.get(16)?,
        last_duration_ms: r.get(17)?,
        next_run_at: r.get(18)?,
        created_at: r.get(19)?,
        updated_at: r.get(20)?,
    })
}

fn snap_from_row(r: &Row<'_>) -> rusqlite::Result<BackupSnapshotRecord> {
    let kind_s: String = r.get(6)?;
    let check_s: String = r.get(9)?;
    let tags_raw: String = r.get(11)?;
    let paths_raw: String = r.get(12)?;
    let kind = BackupTargetKind::parse(&kind_s).ok_or_else(|| invalid_col(6, "kind"))?;
    let check_state =
        BackupCheckState::parse(&check_s).ok_or_else(|| invalid_col(9, "check_state"))?;
    let tags: Vec<String> =
        serde_json::from_str(&tags_raw).map_err(|_| invalid_col(11, "tags_json"))?;
    let paths: Vec<String> =
        serde_json::from_str(&paths_raw).map_err(|_| invalid_col(12, "paths_json"))?;
    Ok(BackupSnapshotRecord {
        id: r.get(0)?,
        job_id: r.get(1)?,
        destination_id: r.get(2)?,
        restic_id: r.get(3)?,
        short_id: r.get(4)?,
        what: r.get(5)?,
        kind,
        taken_at: r.get(7)?,
        size_bytes: r.get(8)?,
        check_state,
        checked_at: r.get(10)?,
        tags,
        paths,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
    })
}

impl Store {
    pub fn ensure_local_destination_path(&self, path: &str, now: i64) -> Result<(), StoreError> {
        let cfg = serde_json::json!({ "path": path }).to_string();
        self.lock().execute(
            "INSERT INTO backup_destinations (id, provider, name, config_json, secrets_enc, verified_at, created_at, updated_at)
             VALUES (?1, 'local', 'Local disk', ?2, NULL, NULL, ?3, ?3)
             ON CONFLICT(id) DO UPDATE SET config_json = excluded.config_json, updated_at = ?3",
            params![LOCAL_DESTINATION_ID, cfg, now],
        )?;
        Ok(())
    }

    pub fn list_backup_destinations(&self) -> Result<Vec<BackupDestinationRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {DEST_COLS} FROM backup_destinations ORDER BY CASE WHEN id = 'local' THEN 1 ELSE 0 END, name"
        ))?;
        Ok(stmt
            .query_map([], dest_from_row)?
            .collect::<Result<_, _>>()?)
    }

    pub fn get_backup_destination(
        &self,
        id: &str,
    ) -> Result<Option<BackupDestinationRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {DEST_COLS} FROM backup_destinations WHERE id = ?1 LIMIT 1"),
                params![id],
                dest_from_row,
            )
            .optional()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_backup_destination(
        &self,
        id: &str,
        provider: &str,
        name: &str,
        config: &Value,
        secrets_enc: Option<&str>,
        verified_at: Option<i64>,
        now: i64,
    ) -> Result<BackupDestinationRecord, StoreError> {
        let cfg_raw = serde_json::to_string(config)?;
        self.lock().execute(
            "INSERT INTO backup_destinations (
                id, provider, name, config_json, secrets_enc, verified_at, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![id, provider, name, cfg_raw, secrets_enc, verified_at, now],
        )?;
        Ok(self
            .get_backup_destination(id)?
            .expect("inserted backup destination must exist"))
    }

    pub fn update_backup_destination_verification(
        &self,
        id: &str,
        verified_at: Option<i64>,
        now: i64,
    ) -> Result<bool, StoreError> {
        let n = self.lock().execute(
            "UPDATE backup_destinations SET verified_at = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, verified_at, now],
        )?;
        Ok(n > 0)
    }

    pub fn delete_backup_destination(&self, id: &str, now: i64) -> Result<bool, StoreError> {
        if id == LOCAL_DESTINATION_ID {
            return Ok(false);
        }
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE backup_jobs SET destination_id = ?2, updated_at = ?3 WHERE destination_id = ?1",
            params![id, LOCAL_DESTINATION_ID, now],
        )?;
        tx.execute(
            "DELETE FROM backup_snapshots WHERE destination_id = ?1",
            params![id],
        )?;
        let n = tx.execute("DELETE FROM backup_destinations WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(n > 0)
    }

    pub fn reassign_backup_jobs_destination(
        &self,
        from_destination_id: &str,
        to_destination_id: &str,
        now: i64,
    ) -> Result<usize, StoreError> {
        let n = self.lock().execute(
            "UPDATE backup_jobs SET destination_id = ?2, updated_at = ?3 WHERE destination_id = ?1",
            params![from_destination_id, to_destination_id, now],
        )?;
        Ok(n)
    }

    pub fn set_backup_job_destination(
        &self,
        job_id: &str,
        destination_id: &str,
        now: i64,
    ) -> Result<bool, StoreError> {
        let n = self.lock().execute(
            "UPDATE backup_jobs SET destination_id = ?2, updated_at = ?3 WHERE id = ?1",
            params![job_id, destination_id, now],
        )?;
        Ok(n > 0)
    }

    /// Create or update the default backup schedule for a template service (used by Prompt 11).
    pub fn upsert_backup_job(
        &self,
        service_id: &str,
        template_id: Option<&str>,
        method: &str,
        cron: &str,
        enabled: bool,
        now: i64,
    ) -> Result<BackupJobRecord, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing_id: Option<String> = tx
            .query_row(
                "SELECT id FROM backup_jobs WHERE service_id = ?1 LIMIT 1",
                params![service_id],
                |r| r.get(0),
            )
            .optional()?;
        let svc_name: String = tx
            .query_row(
                "SELECT name FROM services WHERE id = ?1 LIMIT 1",
                params![service_id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| service_id.to_owned());
        let (kind, what) = if method == "volume" {
            (
                BackupTargetKind::Volume,
                serde_json::json!({
                    "serviceId": service_id,
                    "serviceName": svc_name,
                    "volumes": [format!("opsbox-{svc_name}-data")],
                    "pause": false,
                }),
            )
        } else {
            (
                BackupTargetKind::Database,
                serde_json::json!({
                    "serviceId": service_id,
                    "serviceName": svc_name,
                    "templateId": template_id,
                    "method": method,
                }),
            )
        };
        let what_json = what.to_string();
        let id = existing_id.unwrap_or_else(|| ulid::Ulid::generate().to_string());
        tx.execute(
            "INSERT INTO backup_jobs (
                id, name, kind, what_json, service_id, template_id, method, cron,
                retain_daily, retain_weekly, destination_id, enabled, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 14, 8, 'local', ?9, ?10, ?10)
            ON CONFLICT(service_id) DO UPDATE SET
                name = excluded.name,
                kind = excluded.kind,
                what_json = excluded.what_json,
                template_id = excluded.template_id,
                method = excluded.method,
                cron = excluded.cron,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at",
            params![
                id,
                svc_name,
                kind.as_str(),
                what_json,
                service_id,
                template_id,
                method,
                cron,
                i64::from(enabled),
                now
            ],
        )?;
        let rec = tx.query_row(
            &format!("SELECT {JOB_COLS} FROM backup_jobs WHERE service_id = ?1"),
            params![service_id],
            job_from_row,
        )?;
        tx.commit()?;
        Ok(rec)
    }

    pub fn get_backup_job_for_service(
        &self,
        service_id: &str,
    ) -> Result<Option<BackupJobRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {JOB_COLS} FROM backup_jobs WHERE service_id = ?1 LIMIT 1"),
                params![service_id],
                job_from_row,
            )
            .optional()?)
    }

    pub fn list_backup_jobs(&self) -> Result<Vec<BackupJobRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {JOB_COLS} FROM backup_jobs ORDER BY CASE WHEN id = 'opsbox-self' THEN 1 ELSE 0 END, created_at ASC, name ASC"
        ))?;
        Ok(stmt
            .query_map([], job_from_row)?
            .collect::<Result<_, _>>()?)
    }

    pub fn get_backup_job(&self, id: &str) -> Result<Option<BackupJobRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {JOB_COLS} FROM backup_jobs WHERE id = ?1 LIMIT 1"),
                params![id],
                job_from_row,
            )
            .optional()?)
    }

    pub fn create_backup_job(
        &self,
        input: NewBackupJob<'_>,
    ) -> Result<BackupJobRecord, StoreError> {
        let id = ulid::Ulid::generate().to_string();
        let what_json = serde_json::to_string(input.what)?;
        let conn = self.lock();
        conn.execute(
            "INSERT INTO backup_jobs (
                id, name, kind, what_json, service_id, template_id, method, cron,
                retain_daily, retain_weekly, destination_id, enabled, next_run_at, created_at, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
            params![
                id,
                input.name,
                input.kind.as_str(),
                what_json,
                input.service_id,
                input.template_id,
                input.method,
                input.cron,
                input.retain_daily,
                input.retain_weekly,
                input.destination_id,
                i64::from(input.enabled),
                input.next_run_at,
                input.now,
            ],
        )?;
        Ok(conn.query_row(
            &format!("SELECT {JOB_COLS} FROM backup_jobs WHERE id = ?1"),
            params![id],
            job_from_row,
        )?)
    }

    pub fn update_backup_job(
        &self,
        id: &str,
        patch: UpdateBackupJob<'_>,
    ) -> Result<Option<BackupJobRecord>, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(cur) = tx
            .query_row(
                &format!("SELECT {JOB_COLS} FROM backup_jobs WHERE id = ?1"),
                params![id],
                job_from_row,
            )
            .optional()?
        else {
            return Ok(None);
        };
        let name = patch.name.unwrap_or(&cur.name);
        let what_json = match patch.what {
            Some(v) => serde_json::to_string(v)?,
            None => serde_json::to_string(&cur.what)?,
        };
        let cron = patch.cron.unwrap_or(&cur.cron);
        let retain_daily = patch.retain_daily.unwrap_or(cur.retain_daily);
        let retain_weekly = patch.retain_weekly.unwrap_or(cur.retain_weekly);
        let enabled = patch.enabled.unwrap_or(cur.enabled);
        let next_run_at = patch.next_run_at.unwrap_or(cur.next_run_at);
        tx.execute(
            "UPDATE backup_jobs
             SET name = ?2, what_json = ?3, cron = ?4, retain_daily = ?5,
                 retain_weekly = ?6, enabled = ?7, next_run_at = ?8, updated_at = ?9
             WHERE id = ?1",
            params![
                id,
                name,
                what_json,
                cron,
                retain_daily,
                retain_weekly,
                i64::from(enabled),
                next_run_at,
                patch.now,
            ],
        )?;
        let updated = tx.query_row(
            &format!("SELECT {JOB_COLS} FROM backup_jobs WHERE id = ?1"),
            params![id],
            job_from_row,
        )?;
        tx.commit()?;
        Ok(Some(updated))
    }

    pub fn delete_backup_job(&self, id: &str) -> Result<bool, StoreError> {
        let changed = self
            .lock()
            .execute("DELETE FROM backup_jobs WHERE id = ?1", params![id])?;
        Ok(changed > 0)
    }

    pub fn set_backup_job_next_run(
        &self,
        id: &str,
        next_run_at: Option<i64>,
        now: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE backup_jobs SET next_run_at = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, next_run_at, now],
        )?;
        Ok(())
    }

    pub fn mark_backup_job_running(&self, id: &str, now: i64) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE backup_jobs SET last_status = 'running', last_error = NULL, updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_backup_job_result(
        &self,
        id: &str,
        ok: bool,
        run_at: i64,
        snapshot_id: Option<&str>,
        size_bytes: Option<i64>,
        duration_ms: Option<i64>,
        error: Option<&str>,
        next_run_at: Option<i64>,
    ) -> Result<(), StoreError> {
        let status = if ok { "ok" } else { "failed" };
        self.lock().execute(
            "UPDATE backup_jobs
             SET last_run_at = ?2,
                 last_status = ?3,
                 last_error = ?4,
                 last_snapshot_id = COALESCE(?5, last_snapshot_id),
                 last_size_bytes = COALESCE(?6, last_size_bytes),
                 last_duration_ms = COALESCE(?7, last_duration_ms),
                 next_run_at = ?8,
                 updated_at = ?2
             WHERE id = ?1",
            params![
                id,
                run_at,
                status,
                error,
                snapshot_id,
                size_bytes,
                duration_ms,
                next_run_at
            ],
        )?;
        Ok(())
    }

    pub fn list_due_backup_jobs(&self, now: i64) -> Result<Vec<BackupJobRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {JOB_COLS} FROM backup_jobs
             WHERE enabled = 1 AND method != 'none' AND next_run_at IS NOT NULL AND next_run_at <= ?1
             ORDER BY next_run_at ASC, id ASC"
        ))?;
        Ok(stmt
            .query_map(params![now], job_from_row)?
            .collect::<Result<_, _>>()?)
    }

    /// Reconcile `backup_snapshots` for `destination_id` against the authoritative list from restic.
    /// Deletes rows no longer present in restic (for example after `forget --prune`) and preserves
    /// prior check states on existing snapshots unless `override_check` is specified.
    pub fn sync_backup_snapshots(
        &self,
        destination_id: &str,
        snapshots: &[SyncBackupSnapshot<'_>],
        override_check: Option<(BackupCheckState, i64)>,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let valid_job_ids: std::collections::HashSet<String> = {
            let mut st = tx.prepare("SELECT id FROM backup_jobs")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };

        let existing_ids: Vec<(String, String)> = {
            let mut st =
                tx.prepare("SELECT id, restic_id FROM backup_snapshots WHERE destination_id = ?1")?;
            let rows = st.query_map(params![destination_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<Result<_, _>>()?
        };
        let keep_restic: std::collections::HashSet<&str> =
            snapshots.iter().map(|s| s.restic_id).collect();
        for (row_id, rid) in existing_ids {
            if !keep_restic.contains(rid.as_str()) {
                tx.execute(
                    "DELETE FROM backup_snapshots WHERE id = ?1",
                    params![row_id],
                )?;
            }
        }

        for snap in snapshots {
            let jid = snap.job_id.filter(|id| valid_job_ids.contains(*id));
            let tags_json = serde_json::to_string(snap.tags)?;
            let paths_json = serde_json::to_string(snap.paths)?;
            let id = ulid::Ulid::generate().to_string();
            let (chk_state, chk_at) = override_check.unwrap_or((BackupCheckState::Unverified, 0));
            let chk_at_opt = override_check.map(|(_, ts)| ts);
            let _ = chk_at;
            tx.execute(
                "INSERT INTO backup_snapshots (
                    id, job_id, destination_id, restic_id, short_id, what, kind, taken_at,
                    size_bytes, check_state, checked_at, tags_json, paths_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)
                ON CONFLICT(destination_id, restic_id) DO UPDATE SET
                    job_id = COALESCE(excluded.job_id, backup_snapshots.job_id),
                    short_id = excluded.short_id,
                    what = excluded.what,
                    kind = excluded.kind,
                    taken_at = excluded.taken_at,
                    size_bytes = CASE
                        WHEN excluded.size_bytes > 0 THEN excluded.size_bytes
                        ELSE backup_snapshots.size_bytes
                    END,
                    tags_json = excluded.tags_json,
                    paths_json = excluded.paths_json,
                    updated_at = excluded.updated_at",
                params![
                    id,
                    jid,
                    destination_id,
                    snap.restic_id,
                    snap.short_id,
                    snap.what,
                    snap.kind.as_str(),
                    snap.taken_at,
                    snap.size_bytes,
                    chk_state.as_str(),
                    chk_at_opt,
                    tags_json,
                    paths_json,
                    now,
                ],
            )?;
        }

        if let Some((state, ts)) = override_check {
            // Preserve 'restore_tested' when checksum check succeeds unless checksum failed.
            if state == BackupCheckState::ChecksumFailed {
                tx.execute(
                    "UPDATE backup_snapshots SET check_state = ?2, checked_at = ?3, updated_at = ?3 WHERE destination_id = ?1",
                    params![destination_id, state.as_str(), ts],
                )?;
            } else {
                tx.execute(
                    "UPDATE backup_snapshots
                     SET check_state = CASE WHEN check_state = 'restore_tested' THEN 'restore_tested' ELSE ?2 END,
                         checked_at = ?3,
                         updated_at = ?3
                     WHERE destination_id = ?1",
                    params![destination_id, state.as_str(), ts],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn list_backup_snapshots(
        &self,
        limit: usize,
    ) -> Result<Vec<BackupSnapshotRecord>, StoreError> {
        let lim = i64::try_from(limit.clamp(1, 500)).unwrap_or(500);
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SNAP_COLS} FROM backup_snapshots ORDER BY taken_at DESC, created_at DESC LIMIT ?1"
        ))?;
        Ok(stmt
            .query_map(params![lim], snap_from_row)?
            .collect::<Result<_, _>>()?)
    }

    /// Failed snapshots are an alert source, independent of the paged history UI.
    pub fn list_failed_backup_snapshots(&self) -> Result<Vec<BackupSnapshotRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {SNAP_COLS} FROM backup_snapshots WHERE check_state = 'checksum_failed' ORDER BY taken_at DESC"
        ))?;
        Ok(stmt
            .query_map([], snap_from_row)?
            .collect::<Result<_, _>>()?)
    }

    pub fn get_backup_snapshot(
        &self,
        id: &str,
    ) -> Result<Option<BackupSnapshotRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {SNAP_COLS} FROM backup_snapshots WHERE id = ?1 LIMIT 1"),
                params![id],
                snap_from_row,
            )
            .optional()?)
    }

    pub fn get_backup_snapshot_by_any_id(
        &self,
        id_or_restic: &str,
    ) -> Result<Option<BackupSnapshotRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!(
                    "SELECT {SNAP_COLS} FROM backup_snapshots
                     WHERE id = ?1 OR restic_id = ?1 OR short_id = ?1
                     ORDER BY CASE WHEN id = ?1 THEN 0 WHEN restic_id = ?1 THEN 1 ELSE 2 END, taken_at DESC
                     LIMIT 1"
                ),
                params![id_or_restic],
                snap_from_row,
            )
            .optional()?)
    }

    pub fn set_backup_snapshot_check_state(
        &self,
        id: &str,
        state: BackupCheckState,
        checked_at: i64,
    ) -> Result<bool, StoreError> {
        let n = self.lock().execute(
            "UPDATE backup_snapshots SET check_state = ?2, checked_at = ?3, updated_at = ?3 WHERE id = ?1",
            params![id, state.as_str(), checked_at],
        )?;
        Ok(n > 0)
    }

    pub fn upsert_single_backup_snapshot(
        &self,
        destination_id: &str,
        snap: &SyncBackupSnapshot<'_>,
        check_state: Option<(BackupCheckState, i64)>,
        now: i64,
    ) -> Result<BackupSnapshotRecord, StoreError> {
        let id = ulid::Ulid::generate().to_string();
        let tags_json = serde_json::to_string(snap.tags)?;
        let paths_json = serde_json::to_string(snap.paths)?;
        let (state, chk_at_opt) = match check_state {
            Some((s, ts)) => (s, Some(ts)),
            None => (BackupCheckState::Unverified, None),
        };
        {
            let conn = self.lock();
            let valid_job = snap.job_id.filter(|jid| {
                conn.query_row(
                    "SELECT 1 FROM backup_jobs WHERE id = ?1",
                    params![jid],
                    |_| Ok(()),
                )
                .optional()
                .ok()
                .flatten()
                .is_some()
            });
            conn.execute(
                "INSERT INTO backup_snapshots (
                    id, job_id, destination_id, restic_id, short_id, what, kind, taken_at,
                    size_bytes, check_state, checked_at, tags_json, paths_json, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)
                ON CONFLICT(destination_id, restic_id) DO UPDATE SET
                    job_id = COALESCE(excluded.job_id, backup_snapshots.job_id),
                    short_id = excluded.short_id,
                    what = excluded.what,
                    kind = excluded.kind,
                    taken_at = excluded.taken_at,
                    size_bytes = CASE
                        WHEN excluded.size_bytes > 0 THEN excluded.size_bytes
                        ELSE backup_snapshots.size_bytes
                    END,
                    tags_json = excluded.tags_json,
                    paths_json = excluded.paths_json,
                    updated_at = excluded.updated_at",
                params![
                    id,
                    valid_job,
                    destination_id,
                    snap.restic_id,
                    snap.short_id,
                    snap.what,
                    snap.kind.as_str(),
                    snap.taken_at,
                    snap.size_bytes,
                    state.as_str(),
                    chk_at_opt,
                    tags_json,
                    paths_json,
                    now,
                ],
            )?;
        }
        Ok(self
            .lock()
            .query_row(
                &format!(
                    "SELECT {SNAP_COLS} FROM backup_snapshots WHERE destination_id = ?1 AND restic_id = ?2 LIMIT 1"
                ),
                params![destination_id, snap.restic_id],
                snap_from_row,
            )?)
    }
}
