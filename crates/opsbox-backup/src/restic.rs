//! restic key derivation, argument builders, and `--json` output parsers.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use hmac::{Hmac, KeyInit, Mac};
use opsbox_proto::{BackupTargetKind, ResticSnapshotInfo};
use serde::Deserialize;
use sha2::Sha256;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use zeroize::Zeroizing;

use crate::BackupError;

const RESTIC_DERIVE_CONTEXT: &[u8] = b"opsbox-restic-repo-v1";

/// Derive the restic repository password deterministically from the 32-byte master key.
pub fn derive_repo_password(master_key: &[u8; 32]) -> Zeroizing<String> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(master_key)
        .expect("HMAC accepts any 32-byte key");
    mac.update(RESTIC_DERIVE_CONTEXT);
    let out = mac.finalize().into_bytes();
    Zeroizing::new(B64.encode(out))
}

/// Sanitize a label for inclusion in a restic `--tag` value.
pub fn sanitize_tag_value(v: &str) -> String {
    v.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/') {
                c
            } else {
                '_'
            }
        })
        .take(96)
        .collect()
}

/// Canonical restic tags applied to every snapshot created by opsbox.
pub fn job_tags(job_id: &str, kind: BackupTargetKind, what: &str) -> Vec<String> {
    vec![
        format!("opsbox-job:{}", sanitize_tag_value(job_id)),
        format!("opsbox-kind:{}", kind.as_str()),
        format!("opsbox-what:{}", sanitize_tag_value(what)),
    ]
}

/// Extract `(job_id, kind, what)` back from snapshot tags.
pub fn parse_job_tags(
    tags: &[String],
) -> (Option<String>, Option<BackupTargetKind>, Option<String>) {
    let mut job_id = None;
    let mut kind = None;
    let mut what = None;
    for t in tags {
        if let Some(v) = t.strip_prefix("opsbox-job:") {
            job_id = Some(v.to_owned());
        } else if let Some(v) = t.strip_prefix("opsbox-kind:") {
            kind = BackupTargetKind::parse(v);
        } else if let Some(v) = t.strip_prefix("opsbox-what:") {
            what = Some(v.to_owned());
        }
    }
    (job_id, kind, what)
}

fn base_args(repo: &Path, password_file: &Path) -> Vec<String> {
    vec![
        "-r".to_owned(),
        repo.to_string_lossy().into_owned(),
        "--password-file".to_owned(),
        password_file.to_string_lossy().into_owned(),
    ]
}

pub fn init_args(repo: &Path, password_file: &Path) -> Vec<String> {
    let mut a = base_args(repo, password_file);
    a.extend(["init".to_owned(), "--json".to_owned()]);
    a
}

pub fn backup_stdin_args(
    repo: &Path,
    password_file: &Path,
    stdin_filename: &str,
    tags: &[String],
) -> Vec<String> {
    let mut a = base_args(repo, password_file);
    a.extend([
        "backup".to_owned(),
        "--stdin".to_owned(),
        "--stdin-filename".to_owned(),
        stdin_filename.to_owned(),
        "--json".to_owned(),
    ]);
    for t in tags {
        a.push("--tag".to_owned());
        a.push(t.clone());
    }
    a
}

pub fn backup_paths_args(
    repo: &Path,
    password_file: &Path,
    paths: &[String],
    excludes: &[String],
    tags: &[String],
) -> Vec<String> {
    let mut a = base_args(repo, password_file);
    a.extend(["backup".to_owned(), "--json".to_owned()]);
    for ex in excludes {
        a.push("--exclude".to_owned());
        a.push(ex.clone());
    }
    for t in tags {
        a.push("--tag".to_owned());
        a.push(t.clone());
    }
    a.extend(paths.iter().cloned());
    a
}

pub fn snapshots_args(repo: &Path, password_file: &Path) -> Vec<String> {
    let mut a = base_args(repo, password_file);
    a.extend(["snapshots".to_owned(), "--json".to_owned()]);
    a
}

pub fn forget_args(
    repo: &Path,
    password_file: &Path,
    job_id: Option<&str>,
    retain_daily: u32,
    retain_weekly: u32,
) -> Vec<String> {
    let mut a = base_args(repo, password_file);
    a.extend([
        "forget".to_owned(),
        "--group-by".to_owned(),
        "tags".to_owned(),
        "--prune".to_owned(),
        "--json".to_owned(),
    ]);
    if let Some(jid) = job_id {
        a.push("--tag".to_owned());
        a.push(format!("opsbox-job:{}", sanitize_tag_value(jid)));
    }
    if retain_daily > 0 {
        a.push("--keep-daily".to_owned());
        a.push(retain_daily.to_string());
    }
    if retain_weekly > 0 {
        a.push("--keep-weekly".to_owned());
        a.push(retain_weekly.to_string());
    }
    a
}

pub fn forget_single_snapshot_args(
    repo: &Path,
    password_file: &Path,
    snapshot_id: &str,
) -> Vec<String> {
    let mut a = base_args(repo, password_file);
    a.extend([
        "forget".to_owned(),
        "--prune".to_owned(),
        snapshot_id.to_owned(),
    ]);
    a
}

pub fn check_args(
    repo: &Path,
    password_file: &Path,
    read_data_subset: Option<&str>,
) -> Vec<String> {
    let mut a = base_args(repo, password_file);
    a.extend(["check".to_owned(), "--json".to_owned()]);
    if let Some(sub) = read_data_subset {
        a.push(format!("--read-data-subset={sub}"));
    }
    a
}

/// Extract version from `restic version` stdout (e.g. `"restic 0.19.1 compiled with..."`).
pub fn parse_version(stdout: &str) -> Option<String> {
    let first = stdout.lines().next()?.trim();
    let rest = first.strip_prefix("restic ")?;
    let ver = rest.split_whitespace().next()?;
    (!ver.is_empty()).then(|| ver.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResticBackupSummary {
    pub snapshot_id: String,
    pub short_id: String,
    pub total_bytes_processed: u64,
    pub data_added: u64,
    pub total_files_processed: u64,
}

#[derive(Deserialize)]
struct RawBackupSummary {
    message_type: Option<String>,
    snapshot_id: Option<String>,
    total_bytes_processed: Option<u64>,
    data_added: Option<u64>,
    total_files_processed: Option<u64>,
}

/// Parse NDJSON lines emitted by `restic backup --json` and return the final summary.
pub fn parse_backup_summary(stdout: &str) -> Result<ResticBackupSummary, BackupError> {
    let mut found = None;
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        if let Ok(raw) = serde_json::from_str::<RawBackupSummary>(trimmed)
            && raw.message_type.as_deref() == Some("summary")
        {
            found = Some(raw);
        }
    }
    let s = found.ok_or_else(|| {
        BackupError::Internal("restic backup did not emit a summary JSON record.".into())
    })?;
    let snapshot_id = s.snapshot_id.filter(|id| !id.is_empty()).ok_or_else(|| {
        BackupError::Internal("restic summary did not include snapshot_id.".into())
    })?;
    let short_id = snapshot_id.chars().take(8).collect();
    Ok(ResticBackupSummary {
        snapshot_id,
        short_id,
        total_bytes_processed: s.total_bytes_processed.unwrap_or(0),
        data_added: s.data_added.unwrap_or(0),
        total_files_processed: s.total_files_processed.unwrap_or(0),
    })
}

#[derive(Deserialize)]
struct RawSnapshotSummary {
    total_bytes_processed: Option<u64>,
}

#[derive(Deserialize)]
struct RawSnapshot {
    id: String,
    short_id: Option<String>,
    time: String,
    hostname: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    summary: Option<RawSnapshotSummary>,
}

fn parse_rfc3339_unix(s: &str, fallback: i64) -> i64 {
    OffsetDateTime::parse(s.trim(), &Rfc3339)
        .map(|dt| dt.unix_timestamp())
        .unwrap_or(fallback)
}

/// Parse `restic snapshots --json` output.
pub fn parse_snapshots(
    stdout: &str,
    fallback_now: i64,
) -> Result<Vec<ResticSnapshotInfo>, BackupError> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(Vec::new());
    }
    let raw: Vec<RawSnapshot> = serde_json::from_str(trimmed).map_err(|e| {
        BackupError::Internal(format!("Could not parse restic snapshots output: {e}"))
    })?;
    Ok(raw
        .into_iter()
        .map(|s| {
            let short_id = s
                .short_id
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| s.id.chars().take(8).collect());
            let (job_id, kind, what) = parse_job_tags(&s.tags);
            let taken_at = parse_rfc3339_unix(&s.time, fallback_now);
            ResticSnapshotInfo {
                restic_id: s.id,
                short_id,
                taken_at,
                hostname: s.hostname.unwrap_or_default(),
                paths: s.paths,
                tags: s.tags,
                size_bytes: s
                    .summary
                    .and_then(|sum| sum.total_bytes_processed)
                    .unwrap_or(0),
                job_id,
                kind,
                what,
            }
        })
        .collect())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ResticForgetSummary {
    pub kept_count: u32,
    pub removed_snapshot_ids: Vec<String>,
}

#[derive(Deserialize)]
struct RawForgetGroup {
    #[serde(default)]
    keep: Vec<RawForgetSnap>,
    #[serde(default)]
    remove: Vec<RawForgetSnap>,
}

#[derive(Deserialize)]
struct RawForgetSnap {
    id: Option<String>,
    short_id: Option<String>,
}

/// Parse `restic forget --prune --json` output (handling both JSON array and NDJSON + prune text).
pub fn parse_forget_output(stdout: &str) -> ResticForgetSummary {
    let mut out = ResticForgetSummary::default();
    let consume_groups = |groups: Vec<RawForgetGroup>, out: &mut ResticForgetSummary| {
        for g in groups {
            out.kept_count = out
                .kept_count
                .saturating_add(u32::try_from(g.keep.len()).unwrap_or(u32::MAX));
            for r in g.remove {
                if let Some(id) = r.id.or(r.short_id) {
                    out.removed_snapshot_ids.push(id);
                }
            }
        }
    };
    let trimmed = stdout.trim();
    if trimmed.starts_with('[')
        && let Ok(groups) = serde_json::from_str::<Vec<RawForgetGroup>>(trimmed)
    {
        consume_groups(groups, &mut out);
        return out;
    }
    for line in stdout.lines() {
        let l = line.trim();
        if l.starts_with('[')
            && let Ok(groups) = serde_json::from_str::<Vec<RawForgetGroup>>(l)
        {
            consume_groups(groups, &mut out);
        } else if l.starts_with('{')
            && let Ok(group) = serde_json::from_str::<RawForgetGroup>(l)
        {
            consume_groups(vec![group], &mut out);
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResticCheckSummary {
    pub num_errors: u64,
    pub broken_packs: Vec<String>,
    pub suggest_repair_index: bool,
    pub suggest_prune: bool,
}

#[derive(Deserialize)]
struct RawCheckSummary {
    message_type: Option<String>,
    num_errors: Option<u64>,
    #[serde(default)]
    broken_packs: Vec<String>,
    #[serde(default)]
    suggest_repair_index: bool,
    #[serde(default)]
    suggest_prune: bool,
}

/// Parse `restic check --json` NDJSON summary.
pub fn parse_check_output(stdout: &str) -> Option<ResticCheckSummary> {
    let mut found = None;
    for line in stdout.lines() {
        let l = line.trim();
        if !l.starts_with('{') {
            continue;
        }
        if let Ok(raw) = serde_json::from_str::<RawCheckSummary>(l)
            && raw.message_type.as_deref() == Some("summary")
        {
            found = Some(ResticCheckSummary {
                num_errors: raw.num_errors.unwrap_or(0),
                broken_packs: raw.broken_packs,
                suggest_repair_index: raw.suggest_repair_index,
                suggest_prune: raw.suggest_prune,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_derivation_is_deterministic_and_secret() {
        let k1 = [7u8; 32];
        let k2 = [8u8; 32];
        let p1a = derive_repo_password(&k1);
        let p1b = derive_repo_password(&k1);
        let p2 = derive_repo_password(&k2);
        assert_eq!(p1a.as_str(), p1b.as_str());
        assert_ne!(p1a.as_str(), p2.as_str());
        assert_eq!(p1a.len(), 43);
    }

    #[test]
    fn parses_version_backup_snapshots_forget_and_check() {
        assert_eq!(
            parse_version("restic 0.19.1 compiled with go1.24.2 on linux/amd64\n").as_deref(),
            Some("0.19.1")
        );

        let backup_ndjson = concat!(
            "{\"message_type\":\"status\",\"percent_done\":0.5}\n",
            "{\"message_type\":\"summary\",\"snapshot_id\":\"43deae25abcdef12\",\"total_bytes_processed\":1048576,\"data_added\":65536,\"total_files_processed\":42}\n"
        );
        let sum = parse_backup_summary(backup_ndjson).unwrap();
        assert_eq!(sum.snapshot_id, "43deae25abcdef12");
        assert_eq!(sum.short_id, "43deae25");
        assert_eq!(sum.total_bytes_processed, 1_048_576);
        assert_eq!(sum.data_added, 65_536);

        let snaps_json = r#"[{
            "time": "2026-11-18T02:00:05.123456Z",
            "paths": ["/pg-main.pg_dump.fc"],
            "hostname": "srv-1",
            "tags": ["opsbox-job:job1", "opsbox-kind:database", "opsbox-what:pg-main"],
            "id": "43deae2500000000000000000000000000000000000000000000000000000001",
            "short_id": "43deae25",
            "summary": { "total_bytes_processed": 2048 }
        }]"#;
        let snaps = parse_snapshots(snaps_json, 0).unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].job_id.as_deref(), Some("job1"));
        assert_eq!(snaps[0].kind, Some(BackupTargetKind::Database));
        assert_eq!(snaps[0].what.as_deref(), Some("pg-main"));
        assert_eq!(snaps[0].size_bytes, 2048);

        let forget_out = concat!(
            "[{\"keep\":[{\"id\":\"aaaa1111\",\"short_id\":\"aaaa1111\"}],\"remove\":[{\"id\":\"bbbb2222\",\"short_id\":\"bbbb2222\"}]}]\n",
            "4 snapshots have been removed, running prune\n"
        );
        let fg = parse_forget_output(forget_out);
        assert_eq!(fg.kept_count, 1);
        assert_eq!(fg.removed_snapshot_ids, vec!["bbbb2222"]);

        let chk_out = "{\"message_type\":\"summary\",\"num_errors\":0,\"broken_packs\":[]}\n";
        let chk = parse_check_output(chk_out).unwrap();
        assert_eq!(chk.num_errors, 0);
        assert!(chk.broken_packs.is_empty());
    }
}
