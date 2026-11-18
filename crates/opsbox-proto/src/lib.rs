//! Shared types for opsbox.
//!
//! This crate has no I/O and no async runtime so the desktop app, `serve`, `priv`
//! and (later) the watchdog can all depend on it.

pub mod access;
pub mod api;
pub mod backup;
pub mod cleanup;
pub mod database;
pub mod deploy;
pub mod disk;
pub mod docker;
pub mod doctor;
pub mod files;
pub mod frame;
pub mod metrics;
pub mod privilege;
pub mod pty;
pub mod role;
pub mod security;
pub mod system;
pub mod uninstall;
pub mod watchdog;

pub use access::{AccessState, AccessUser, AuthorizedKey, SshStatus};
pub use api::{ApiError, ApiErrorBody, ErrorCode, HostView};
pub use backup::{
    BackupCheckRequest, BackupCheckResult, BackupCheckState, BackupDbKind,
    BackupDestinationTestRequest, BackupDestinationTestResult, BackupDestinationTestTarget,
    BackupForgetRequest, BackupForgetResult, BackupGenerateSftpKeyRequest, BackupLocalInfo,
    BackupProviderField, BackupProviderId, BackupProviderPreset, BackupRecoveryKeyResult,
    BackupRepoTarget, BackupRestoreMode, BackupRestoreRequest, BackupRestoreResult,
    BackupRestoreTarget, BackupRestoreTestRequest, BackupRestoreTestResult,
    BackupRestoreTestTarget, BackupRunResult, BackupRunSpec, BackupRunTarget, BackupS3Credentials,
    BackupSftpKeyResult, BackupSnapshotsRequest, BackupTargetKind, BackupTestStep,
    BackupTestStepStatus, ResticSnapshotInfo, backup_provider_presets, valid_backup_id,
    valid_restic_snapshot_id,
};
pub use cleanup::{CleanupFreed, CleanupItem, CleanupScan};
pub use database::{
    ALLOWED_EXTENSIONS, APP_CONNECTION_VARS, APP_VAR_PREFIX, DatabaseAction,
    DatabaseInspectRequest, DatabaseInspectResult, DatabaseManageRequest, DatabaseManageResult,
    DatabasePoolerRequest, DatabasePoolerResult, DatabaseUpgradeRequest, DatabaseUpgradeResult,
    DbDatabaseInfo, DbExtensionInfo, DbGrantInfo, DbSlowQuery, DbTableSize, DbTarget, DbUserInfo,
    INTERNAL_VAR_PREFIX, allowed_extension, app_connect_pairs, app_pg_name, app_var_prefix,
    pg_major, valid_pg_name,
};
pub use deploy::{
    ComposeDeployRequest, ComposeDeployResult, ComposeDeployedService, ComposeServiceInfo,
    ComposeValidateRequest, ComposeValidationResult, DatabaseRotateRequest, DatabaseRotateResult,
    DeployBuildFrame, DeployBuildRequest, DeployBuildResult, DeployEnvSpec, DeployGitSource,
    DeployPullRequest, DeployRefSource, DeployStartRequest, DeployStartResult,
    TemplateStartRequest, valid_commit_sha, valid_compose_service_name, valid_db_identifier,
    valid_docker_image_ref, valid_domain_name, valid_env_key, valid_git_branch, valid_health_path,
    valid_rel_repo_path, valid_service_name,
};
pub use disk::{DiskLayout, DiskState, DiskStep, DiskVolume};
pub use docker::{
    DockerContainer, DockerContainerAction, DockerContainerInspect, DockerContainerNetwork,
    DockerContainerStat, DockerEnvVar, DockerExecOutput, DockerExecRequest, DockerImage,
    DockerLogFrame, DockerLogLine, DockerLogStream, DockerMount, DockerNetwork, DockerPort,
    DockerStatus, DockerVolume, valid_docker_ref,
};
pub use doctor::{CheckStatus, DoctorCheck, DoctorReport};
pub use files::{DirListing, FileContent, FileEntry, FileKind, Staged};
pub use metrics::{CpuTimes, DiskIo, DiskUsage, MemInfo, NetIf, RawProc, RawSample, SystemFacts};
pub use privilege::{
    HostInfo, PrivError, PrivErrorCode, PrivOp, PrivReply, PrivRequest, PrivResponse, PrivResult,
    Secret,
};
pub use pty::{MAX_PTY_DIM, MAX_PTY_PAYLOAD, PtyFrame, PtyFrameError, SystemUser, clamp_size};
pub use role::Role;
pub use security::{HostSettings, SecurityScan};
pub use system::{
    CommandOutcome, FirewallChange, FirewallRule, FirewallState, JobInfo, JournalEntry, NewRule,
    PkgSelect, PkgState, PkgUpdate, UnitActionKind, UnitInfo,
};
pub use uninstall::{UninstallAdopted, UninstallPlan};
pub use watchdog::{WatchdogCheck, WatchdogStatus};

pub mod network;
pub use network::{
    AdoptedDomain, AdoptedRoutes, ListenerAction, PortContainer, PortHolder, PortTakeover,
    PortTarget, ProxyNetwork, WebDiff, WebMove, WebPlan,
};
