//! Shared wire types for backup jobs, restic operations, and local disk inspection
//! (docs/BACKEND_SPEC.md 7.12).

use serde::{Deserialize, Serialize};

use crate::deploy::DeployEnvSpec;

/// What kind of resource a backup job captures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupTargetKind {
    Database,
    Volume,
    Path,
    Opsbox,
}

impl BackupTargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Volume => "volume",
            Self::Path => "path",
            Self::Opsbox => "opsbox",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "database" => Some(Self::Database),
            "volume" => Some(Self::Volume),
            "path" => Some(Self::Path),
            "opsbox" => Some(Self::Opsbox),
            _ => None,
        }
    }
}

/// Supported database engines for logical dump backup streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupDbKind {
    Postgres,
    Mysql,
    Mariadb,
    Redis,
    Mongo,
}

impl BackupDbKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
            Self::Mariadb => "mariadb",
            Self::Redis => "redis",
            Self::Mongo => "mongo",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "postgres" => Some(Self::Postgres),
            "mysql" => Some(Self::Mysql),
            "mariadb" => Some(Self::Mariadb),
            "redis" => Some(Self::Redis),
            "mongo" => Some(Self::Mongo),
            _ => None,
        }
    }

    pub fn method_name(self) -> &'static str {
        match self {
            Self::Postgres => "pg_dump",
            Self::Mysql | Self::Mariadb => "mysqldump",
            Self::Redis => "redis-cli --rdb",
            Self::Mongo => "mongodump",
        }
    }
}

/// Integrity verification state of a snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackupCheckState {
    #[default]
    Unverified,
    ChecksumOk,
    ChecksumFailed,
    RestoreTested,
}

impl BackupCheckState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::ChecksumOk => "checksum_ok",
            Self::ChecksumFailed => "checksum_failed",
            Self::RestoreTested => "restore_tested",
        }
    }

    pub fn display_label(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::ChecksumOk => "checksum ok",
            Self::ChecksumFailed => "checksum failed",
            Self::RestoreTested => "restore-tested",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unverified" => Some(Self::Unverified),
            "checksum_ok" => Some(Self::ChecksumOk),
            "checksum_failed" => Some(Self::ChecksumFailed),
            "restore_tested" => Some(Self::RestoreTested),
            _ => None,
        }
    }
}

/// Target specification passed to `opsbox-priv` for `PrivOp::BackupRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackupRunTarget {
    Database {
        template: BackupDbKind,
        service_id: String,
        service_name: String,
        container_id: String,
        vars: Vec<DeployEnvSpec>,
    },
    Volume {
        volumes: Vec<String>,
        #[serde(default)]
        pause_containers: bool,
    },
    Path {
        paths: Vec<String>,
    },
    Opsbox,
}

/// Request payload for `PrivOp::BackupRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRunSpec {
    pub destination_id: String,
    pub job_id: String,
    pub job_name: String,
    pub target: BackupRunTarget,
    pub retain_daily: u32,
    pub retain_weekly: u32,
    #[serde(default)]
    pub run_prune: bool,
    #[serde(default)]
    pub run_check: bool,
}

/// Outcome returned by `PrivOp::BackupRun`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRunResult {
    pub snapshot_id: String,
    pub short_id: String,
    pub taken_at: i64,
    pub size_bytes: u64,
    pub data_added_bytes: u64,
    pub files_processed: u64,
    pub duration_ms: u64,
    pub pruned_count: u32,
    pub check_state: BackupCheckState,
    pub check_error: Option<String>,
    pub log: Vec<String>,
}

/// Request payload for `PrivOp::BackupForget`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupForgetRequest {
    pub destination_id: String,
    pub job_id: Option<String>,
    pub retain_daily: u32,
    pub retain_weekly: u32,
}

/// Outcome returned by `PrivOp::BackupForget`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupForgetResult {
    pub removed_snapshot_ids: Vec<String>,
    pub kept_count: u32,
    pub removed_count: u32,
    pub log: Vec<String>,
}

/// Request payload for `PrivOp::BackupSnapshots`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSnapshotsRequest {
    pub destination_id: String,
}

/// Snapshot metadata returned from `restic snapshots --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResticSnapshotInfo {
    pub restic_id: String,
    pub short_id: String,
    pub taken_at: i64,
    pub hostname: String,
    pub paths: Vec<String>,
    pub tags: Vec<String>,
    pub size_bytes: u64,
    pub job_id: Option<String>,
    pub kind: Option<BackupTargetKind>,
    pub what: Option<String>,
}

/// Request payload for `PrivOp::BackupCheck`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupCheckRequest {
    pub destination_id: String,
    pub read_data_subset: Option<String>,
}

/// Outcome returned by `PrivOp::BackupCheck`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupCheckResult {
    pub ok: bool,
    pub num_errors: u64,
    pub broken_packs: Vec<String>,
    pub summary: String,
}

/// Filesystem and restic readiness information for `/var/backups/opsbox`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupLocalInfo {
    pub path: String,
    pub exists: bool,
    pub repo_initialized: bool,
    pub restic_installed: bool,
    pub restic_version: Option<String>,
    pub mount_point: Option<String>,
    pub fs_type: Option<String>,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub same_disk: bool,
    pub same_disk_paths: Vec<String>,
}

/// Validate a backup destination/job identifier slug (`a-z`, `0-9`, `-`, `_`, 1..=64 chars).
pub fn valid_backup_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_enums_round_trip() {
        for k in [
            BackupTargetKind::Database,
            BackupTargetKind::Volume,
            BackupTargetKind::Path,
            BackupTargetKind::Opsbox,
        ] {
            assert_eq!(BackupTargetKind::parse(k.as_str()), Some(k));
        }
        for db in [
            BackupDbKind::Postgres,
            BackupDbKind::Mysql,
            BackupDbKind::Mariadb,
            BackupDbKind::Redis,
            BackupDbKind::Mongo,
        ] {
            assert_eq!(BackupDbKind::parse(db.as_str()), Some(db));
        }
        for st in [
            BackupCheckState::Unverified,
            BackupCheckState::ChecksumOk,
            BackupCheckState::ChecksumFailed,
            BackupCheckState::RestoreTested,
        ] {
            assert_eq!(BackupCheckState::parse(st.as_str()), Some(st));
        }
        assert!(valid_backup_id("opsbox-self"));
        assert!(valid_backup_id("01J9ABCDEFGHJKMNPQRSTVWXYZ"));
        assert!(!valid_backup_id("../etc"));
        assert!(!valid_backup_id(""));
    }
}
