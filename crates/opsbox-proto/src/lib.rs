//! Shared types for opsbox.
//!
//! This crate has no I/O and no async runtime so the desktop app, `serve`, `priv`
//! and (later) the watchdog can all depend on it.

pub mod access;
pub mod api;
pub mod cleanup;
pub mod deploy;
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
pub mod watchdog;

pub use access::{AccessState, AccessUser, AuthorizedKey, SshStatus};
pub use api::{ApiError, ApiErrorBody, ErrorCode, HostView};
pub use cleanup::{CleanupFreed, CleanupItem, CleanupScan};
pub use deploy::{
    ComposeDeployRequest, ComposeDeployResult, ComposeDeployedService, ComposeServiceInfo,
    ComposeValidateRequest, ComposeValidationResult, DeployBuildFrame, DeployBuildRequest,
    DeployBuildResult, DeployEnvSpec, DeployGitSource, DeployPullRequest, DeployStartRequest,
    DeployStartResult, TemplateStartRequest, valid_commit_sha, valid_compose_service_name,
    valid_docker_image_ref, valid_domain_name, valid_env_key, valid_git_branch, valid_health_path,
    valid_rel_repo_path, valid_service_name,
};
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
pub use watchdog::{WatchdogCheck, WatchdogStatus};
