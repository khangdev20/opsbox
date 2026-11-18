//! Shared wire types for backup jobs, destination presets, restic operations,
//! recovery key export, and restore (docs/BACKEND_SPEC.md 7.12).

use serde::{Deserialize, Serialize};

use crate::deploy::DeployEnvSpec;
use crate::privilege::Secret;

/// Supported backup destination provider IDs (matches UI `PROVIDERS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackupProviderId {
    R2,
    S3,
    B2,
    Wasabi,
    Do,
    Hetzner,
    Gcs,
    Minio,
    Sftp,
    Custom,
}

impl BackupProviderId {
    pub const ALL: [Self; 10] = [
        Self::R2,
        Self::S3,
        Self::B2,
        Self::Wasabi,
        Self::Do,
        Self::Hetzner,
        Self::Gcs,
        Self::Minio,
        Self::Sftp,
        Self::Custom,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::R2 => "r2",
            Self::S3 => "s3",
            Self::B2 => "b2",
            Self::Wasabi => "wasabi",
            Self::Do => "do",
            Self::Hetzner => "hetzner",
            Self::Gcs => "gcs",
            Self::Minio => "minio",
            Self::Sftp => "sftp",
            Self::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "r2" => Some(Self::R2),
            "s3" => Some(Self::S3),
            "b2" => Some(Self::B2),
            "wasabi" => Some(Self::Wasabi),
            "do" | "spaces" => Some(Self::Do),
            "hetzner" => Some(Self::Hetzner),
            "gcs" => Some(Self::Gcs),
            "minio" => Some(Self::Minio),
            "sftp" => Some(Self::Sftp),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub fn is_sftp(self) -> bool {
        self == Self::Sftp
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::R2 => "Cloudflare R2",
            Self::S3 => "AWS S3",
            Self::B2 => "Backblaze B2",
            Self::Wasabi => "Wasabi",
            Self::Do => "DigitalOcean Spaces",
            Self::Hetzner => "Hetzner Storage",
            Self::Gcs => "Google Cloud Storage",
            Self::Minio => "MinIO",
            Self::Sftp => "SFTP server",
            Self::Custom => "Other S3-compatible",
        }
    }
}

/// Form field metadata for a backup destination provider preset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupProviderField {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub secret: bool,
}

/// Provider preset exposed by `GET /api/v1/backups/providers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupProviderPreset {
    pub id: BackupProviderId,
    pub name: String,
    pub sub: String,
    pub mono: String,
    pub sftp: bool,
    pub endpoint_template: String,
    pub fields: Vec<BackupProviderField>,
    pub steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip: Option<String>,
}

fn pfield(
    key: &str,
    label: &str,
    hint: Option<&str>,
    optional: bool,
    secret: bool,
) -> BackupProviderField {
    BackupProviderField {
        key: key.to_owned(),
        label: label.to_owned(),
        hint: hint.map(str::to_owned),
        optional,
        secret,
    }
}

/// Return the canonical list of supported offsite backup destination presets.
pub fn backup_provider_presets() -> Vec<BackupProviderPreset> {
    vec![
        BackupProviderPreset {
            id: BackupProviderId::R2,
            name: "Cloudflare R2".into(),
            sub: "Cloudflare".into(),
            mono: "R2".into(),
            sftp: false,
            endpoint_template: "https://{account}.r2.cloudflarestorage.com".into(),
            fields: vec![
                pfield(
                    "account",
                    "Account ID",
                    Some("Cloudflare dashboard › R2 › Overview, right-hand side."),
                    false,
                    false,
                ),
                pfield("bucket", "Bucket", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "Access Key ID", None, false, false),
                pfield("secret", "Secret Access Key", None, false, true),
            ],
            steps: vec![
                "Open R2 Object Storage and create a bucket.".into(),
                "Manage R2 API Tokens › Create API token.".into(),
                "Choose Object Read & Write, limited to that bucket.".into(),
                "Copy the Access Key ID and Secret here. The secret is shown only once.".into(),
            ],
            tip: Some("R2 charges nothing to download, so restoring costs only storage.".into()),
        },
        BackupProviderPreset {
            id: BackupProviderId::S3,
            name: "AWS S3".into(),
            sub: "Amazon".into(),
            mono: "S3".into(),
            sftp: false,
            endpoint_template: "https://s3.{region}.amazonaws.com".into(),
            fields: vec![
                pfield(
                    "region",
                    "Region",
                    Some("For example ap-southeast-2."),
                    false,
                    false,
                ),
                pfield("bucket", "Bucket", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "Access Key ID", None, false, false),
                pfield("secret", "Secret Access Key", None, false, true),
            ],
            steps: vec![
                "Create a bucket in S3 (block public access stays on).".into(),
                "In IAM create a user with a policy limited to that bucket.".into(),
                "Create an access key for that user.".into(),
                "Copy the key and secret here.".into(),
            ],
            tip: None,
        },
        BackupProviderPreset {
            id: BackupProviderId::B2,
            name: "Backblaze B2".into(),
            sub: "Backblaze".into(),
            mono: "B2".into(),
            sftp: false,
            endpoint_template: "https://s3.{region}.backblazeb2.com".into(),
            fields: vec![
                pfield(
                    "region",
                    "Region",
                    Some("Shown on the bucket page, e.g. us-west-004."),
                    false,
                    false,
                ),
                pfield("bucket", "Bucket", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "Application Key ID", None, false, false),
                pfield("secret", "Application Key", None, false, true),
            ],
            steps: vec![
                "Create a private bucket.".into(),
                "App Keys › Add a New Application Key, limited to that bucket.".into(),
                "Copy the keyID and applicationKey here.".into(),
            ],
            tip: None,
        },
        BackupProviderPreset {
            id: BackupProviderId::Wasabi,
            name: "Wasabi".into(),
            sub: "Wasabi".into(),
            mono: "WA".into(),
            sftp: false,
            endpoint_template: "https://s3.{region}.wasabisys.com".into(),
            fields: vec![
                pfield("region", "Region", None, false, false),
                pfield("bucket", "Bucket", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "Access Key", None, false, false),
                pfield("secret", "Secret Key", None, false, true),
            ],
            steps: vec![
                "Create a bucket.".into(),
                "Access Keys › Create new access key.".into(),
                "Copy both keys here.".into(),
            ],
            tip: None,
        },
        BackupProviderPreset {
            id: BackupProviderId::Do,
            name: "DigitalOcean Spaces".into(),
            sub: "DigitalOcean".into(),
            mono: "DO".into(),
            sftp: false,
            endpoint_template: "https://{region}.digitaloceanspaces.com".into(),
            fields: vec![
                pfield("region", "Region", Some("e.g. sgp1"), false, false),
                pfield("bucket", "Space name", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "Access Key", None, false, false),
                pfield("secret", "Secret", None, false, true),
            ],
            steps: vec![
                "Create a Space.".into(),
                "API › Spaces Keys › Generate New Key.".into(),
                "Copy both values here.".into(),
            ],
            tip: None,
        },
        BackupProviderPreset {
            id: BackupProviderId::Hetzner,
            name: "Hetzner Storage".into(),
            sub: "Hetzner".into(),
            mono: "HZ".into(),
            sftp: false,
            endpoint_template: "https://{region}.your-objectstorage.com".into(),
            fields: vec![
                pfield(
                    "region",
                    "Location",
                    Some("fsn1, nbg1 or hel1"),
                    false,
                    false,
                ),
                pfield("bucket", "Bucket", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "Access Key", None, false, false),
                pfield("secret", "Secret Key", None, false, true),
            ],
            steps: vec![
                "Create a bucket in Object Storage.".into(),
                "Security › S3 credentials › Generate.".into(),
                "Copy both values here.".into(),
            ],
            tip: None,
        },
        BackupProviderPreset {
            id: BackupProviderId::Gcs,
            name: "Google Cloud Storage".into(),
            sub: "Google Cloud".into(),
            mono: "GC".into(),
            sftp: false,
            endpoint_template: "https://storage.googleapis.com".into(),
            fields: vec![
                pfield("bucket", "Bucket", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "HMAC Access ID", None, false, false),
                pfield("secret", "HMAC Secret", None, false, true),
            ],
            steps: vec![
                "Create a bucket.".into(),
                "Settings › Interoperability › Create a key for a service account.".into(),
                "Copy the access ID and secret here.".into(),
            ],
            tip: None,
        },
        BackupProviderPreset {
            id: BackupProviderId::Minio,
            name: "MinIO".into(),
            sub: "Self-hosted".into(),
            mono: "MI".into(),
            sftp: false,
            endpoint_template: "{endpoint}".into(),
            fields: vec![
                pfield(
                    "endpoint",
                    "Endpoint URL",
                    Some("https://minio.example.com:9000"),
                    false,
                    false,
                ),
                pfield("bucket", "Bucket", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "Access Key", None, false, false),
                pfield("secret", "Secret Key", None, false, true),
            ],
            steps: vec![
                "Create a bucket and a user limited to it.".into(),
                "Create an access key for that user.".into(),
            ],
            tip: None,
        },
        BackupProviderPreset {
            id: BackupProviderId::Sftp,
            name: "SFTP server".into(),
            sub: "Any Linux box over SSH".into(),
            mono: "SF".into(),
            sftp: true,
            endpoint_template: "sftp://{user}@{host}".into(),
            fields: vec![
                pfield(
                    "host",
                    "Host",
                    Some("backup.example.com or an IP"),
                    false,
                    false,
                ),
                pfield("port", "SSH port", Some("22"), true, false),
                pfield("user", "User", None, false, false),
                pfield(
                    "path",
                    "Folder on that server",
                    Some("/srv/backups/srv-sgn-01"),
                    false,
                    false,
                ),
            ],
            steps: vec![
                "opsbox shows a public key after you save.".into(),
                "Add it to ~/.ssh/authorized_keys of that user on the backup server.".into(),
                "Run the test again.".into(),
            ],
            tip: None,
        },
        BackupProviderPreset {
            id: BackupProviderId::Custom,
            name: "Other S3-compatible".into(),
            sub: "Custom endpoint".into(),
            mono: "S3".into(),
            sftp: false,
            endpoint_template: "{endpoint}".into(),
            fields: vec![
                pfield("endpoint", "Endpoint URL", None, false, false),
                pfield("region", "Region", None, true, false),
                pfield("bucket", "Bucket", None, false, false),
                pfield("prefix", "Folder in bucket", None, true, false),
                pfield("key", "Access Key", None, false, false),
                pfield("secret", "Secret Key", None, false, true),
            ],
            steps: vec![
                "Use the S3 endpoint your provider documents.".into(),
                "Create a key limited to one bucket.".into(),
            ],
            tip: None,
        },
    ]
}

/// Credentials passed to `opsbox-priv` for S3-compatible restic repositories.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackupS3Credentials {
    /// Sealed JSON `{"accessKeyId":...,"secretAccessKey":...}` under `backup:dest:<id>`.
    Sealed {
        seal_context: String,
        value_enc: String,
    },
    /// Ephemeral plaintext credentials used solely when testing an unsaved destination.
    Plaintext {
        access_key_id: Secret,
        secret_access_key: Secret,
    },
}

/// Target restic repository descriptor passed from `serve` to `opsbox-priv`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum BackupRepoTarget {
    #[default]
    Local,
    S3 {
        destination_id: String,
        endpoint: String,
        bucket: String,
        repo_path: String,
        #[serde(default)]
        region: Option<String>,
        credentials: BackupS3Credentials,
    },
    Sftp {
        destination_id: String,
        host: String,
        port: u16,
        user: String,
        repo_path: String,
        seal_context: String,
        private_key_enc: String,
    },
}

/// Step outcome state for destination connection tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupTestStepStatus {
    Ok,
    Warn,
    Fail,
}

/// Single checklist step returned by `PrivOp::BackupTestDestination`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupTestStep {
    pub status: BackupTestStepStatus,
    pub message: String,
}

/// What destination connection test to run inside `opsbox-priv`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BackupDestinationTestTarget {
    /// Full restic repo init + write/read probe + delete-permission test.
    Restic { repo: BackupRepoTarget },
    /// Pre-save SFTP reachability probe before the SSH public key has been installed remotely.
    SftpReachability {
        host: String,
        port: u16,
        user: String,
        repo_path: String,
    },
}

/// Request payload for `PrivOp::BackupTestDestination`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDestinationTestRequest {
    pub provider: BackupProviderId,
    pub target: BackupDestinationTestTarget,
}

/// Result returned by `PrivOp::BackupTestDestination`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupDestinationTestResult {
    pub ok: bool,
    pub verified: bool,
    pub steps: Vec<BackupTestStep>,
}

/// Request payload for `PrivOp::BackupGenerateSftpKey`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupGenerateSftpKeyRequest {
    pub destination_id: String,
}

/// Sealed Ed25519 SSH keypair metadata returned by `PrivOp::BackupGenerateSftpKey`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupSftpKeyResult {
    pub public_key: String,
    pub fingerprint: String,
    pub seal_context: String,
    pub private_key_enc: String,
}

/// Result returned by `PrivOp::BackupRecoveryKey`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRecoveryKeyResult {
    pub recovery_key: Secret,
    pub hostname: String,
}

/// Restore mode (`copy` by default; `replace` requires typed `"replace"` confirmation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackupRestoreMode {
    #[default]
    Copy,
    Replace,
}

/// Target specification for `PrivOp::BackupRestore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackupRestoreTarget {
    Database {
        template: BackupDbKind,
        service_id: String,
        service_name: String,
        container_id: String,
        stdin_filename: String,
        vars: Vec<DeployEnvSpec>,
    },
    Path {
        snapshot_paths: Vec<String>,
        copy_target: Option<String>,
    },
    Volume {
        source_volumes: Vec<String>,
        snapshot_paths: Vec<String>,
        copy_volume: Option<String>,
        project: String,
        service_label: String,
    },
    Opsbox {
        copy_target: Option<String>,
    },
}

/// Request payload for `PrivOp::BackupRestore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRestoreRequest {
    pub destination_id: String,
    #[serde(default)]
    pub repo: BackupRepoTarget,
    pub restic_id: String,
    pub mode: BackupRestoreMode,
    /// Must equal `"replace"` when `mode == BackupRestoreMode::Replace`.
    pub confirm_replace: Option<String>,
    pub target: BackupRestoreTarget,
    #[serde(default)]
    pub job_id: Option<String>,
}

/// Outcome returned by `PrivOp::BackupRestore`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRestoreResult {
    pub ok: bool,
    pub mode: BackupRestoreMode,
    pub target_summary: String,
    pub message: String,
    pub safety_snapshot_id: Option<String>,
    pub safety_snapshot_short_id: Option<String>,
    pub log: Vec<String>,
}

/// Target specification for automated or manual restore verification (`PrivOp::BackupRestoreTest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackupRestoreTestTarget {
    Database {
        template: BackupDbKind,
        version: String,
        stdin_filename: String,
        db_user: Option<String>,
        db_name: Option<String>,
    },
    Files {
        is_opsbox_self: bool,
    },
}

/// Request payload for `PrivOp::BackupRestoreTest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRestoreTestRequest {
    pub destination_id: String,
    #[serde(default)]
    pub repo: BackupRepoTarget,
    pub restic_id: String,
    pub target: BackupRestoreTestTarget,
}

/// Outcome returned by `PrivOp::BackupRestoreTest`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupRestoreTestResult {
    pub ok: bool,
    pub summary: String,
    pub log: Vec<String>,
}

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
    #[serde(default)]
    pub repo: BackupRepoTarget,
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
    #[serde(default)]
    pub repo: BackupRepoTarget,
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
    #[serde(default)]
    pub repo: BackupRepoTarget,
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
    #[serde(default)]
    pub repo: BackupRepoTarget,
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

/// Validate a restic snapshot ID or short ID (8..=128 ASCII hex digits).
pub fn valid_restic_snapshot_id(s: &str) -> bool {
    (8..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
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
        assert!(valid_restic_snapshot_id("43deae25"));
        assert!(!valid_restic_snapshot_id("bad;id"));
    }

    #[test]
    fn provider_presets_cover_all_ten_and_redact_secrets() {
        let presets = backup_provider_presets();
        assert_eq!(presets.len(), 10);
        for id in BackupProviderId::ALL {
            assert!(
                presets.iter().any(|p| p.id == id),
                "missing preset for {}",
                id.as_str()
            );
            assert_eq!(BackupProviderId::parse(id.as_str()), Some(id));
        }
        let creds = BackupS3Credentials::Plaintext {
            access_key_id: Secret::new("AKIATEST123"),
            secret_access_key: Secret::new("SUPERSECRETKEY999"),
        };
        let dbg = format!("{creds:?}");
        assert!(!dbg.contains("AKIATEST123") && !dbg.contains("SUPERSECRETKEY999"));
        assert!(dbg.contains("[redacted]"));
    }
}
