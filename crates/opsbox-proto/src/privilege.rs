//! The privileged-operation protocol between `serve` (and later `proxy`) and `priv`.
//!
//! `PrivOp` is a closed enum. `priv` rejects anything that does not deserialize
//! into it; there is no "run this shell string" operation and there never will be.

use serde::{Deserialize, Serialize};

/// A string that must never reach a log: `Debug` prints `[redacted]`. Serializes as a
/// plain string.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    /// The plaintext. Call sites are the only places a secret becomes visible.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

/// A request frame. `id` correlates the response and the logs on both sides; `serve`
/// reuses the HTTP request id so one request can be followed across processes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivRequest {
    pub id: String,
    /// Who the operation is performed for, recorded by `priv` for audit.
    #[serde(default)]
    pub actor: Option<String>,
    pub op: PrivOp,
}

/// Every operation `priv` can perform. Add a variant here, then a module in
/// `opsbox-priv`, and nothing else becomes possible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "args", rename_all = "snake_case")]
pub enum PrivOp {
    /// Liveness check. Answered with [`PrivReply::Pong`].
    Ping,
    Capacity,
    CapacityImageSize {
        image: String,
    },
    WorkloadMemory {
        container: String,
        service: String,
        memory_mb: u64,
    },
    CapacityAddSwap {
        size_mb: u64,
    },
    PortHolders {
        ports: Vec<u16>,
    },
    ProxyNetwork,
    ProxyTakeover {
        port: u16,
        target: crate::PortTarget,
        confirm: bool,
    },
    ProxyUndo {
        id: String,
        confirm: bool,
    },
    ProxyKeepPorts {
        confirm: bool,
    },
    /// Facts about the machine. Answered with [`PrivReply::HostInfo`].
    HostInfo,
    /// Bounded setup notice written by the privileged helper only.
    ConsoleNotice {
        text: Secret,
    },
    /// Read-only diagnostics for `opsbox doctor`.
    Doctor,
    /// Create a TOTP secret. `priv` keeps the plaintext only long enough to return it
    /// once (for the QR code) and as a sealed value to store. `context` binds the
    /// sealed value to its owner, e.g. `totp:<user id>`.
    TotpNewSecret {
        context: String,
    },
    /// Check a 6-digit code against a sealed secret (RFC 6238, SHA-1, 30 s, +-1 step).
    /// Only steps newer than `after_step` match, so a code cannot be used twice.
    /// The secret never leaves `priv`.
    TotpVerify {
        sealed: String,
        context: String,
        code: Secret,
        after_step: u64,
    },
    /// Seal a secret with the master key under a namespaced `context` (for proxy keys).
    /// `priv` refuses `totp:*` contexts so callers cannot access TOTP secrets generically.
    SecretSeal {
        context: String,
        plaintext: Secret,
    },
    /// Open a secret sealed with [`PrivOp::SecretSeal`] for the same `context`.
    SecretOpen {
        context: String,
        sealed: String,
    },
    /// Switch `[api] listen` in `config.toml` between `127.0.0.1` (local-only) and
    /// `0.0.0.0` (public) and schedule a brief restart of `opsbox.service`.
    WebLocalOnly {
        local_only: bool,
    },
    /// Cumulative machine counters (CPU, memory, disks, network, ...). `procs` also
    /// scans every process, which is the expensive part, so it is asked for less often.
    MetricsSample {
        procs: bool,
    },
    /// Reboot-required flag, failed systemd units and pending package updates.
    SystemFacts,
    /// Pending package updates, reboot flag and the automatic-update setting.
    PkgList,
    /// Install updates. Names are never taken from the caller: `priv` works out what is
    /// pending itself and installs the security subset or everything.
    PkgInstall {
        which: crate::system::PkgSelect,
    },
    /// Install only these currently pending package names, revalidated in `priv`.
    PkgInstallSelected {
        names: Vec<String>,
    },
    /// Turn automatic security updates on or off.
    AutoUpdatesSet {
        on: bool,
    },
    UnitList,
    UnitAction {
        name: String,
        action: crate::system::UnitActionKind,
    },
    /// Read the journal, newest first. `after` is a cursor: only newer entries are returned.
    JournalQuery {
        unit: Option<String>,
        max_priority: u8,
        text: Option<String>,
        limit: u32,
        after: Option<String>,
    },
    JobList,
    JobAdd {
        name: String,
        cron: String,
        command: String,
    },
    JobRun {
        id: String,
    },
    JobRemove {
        id: String,
    },
    FirewallList,
    /// `caller_ip` is the address the request came from; a change that could cut it off
    /// needs `confirm` and is undone after 60 s unless `FirewallConfirm` follows.
    FirewallAdd {
        rule: crate::system::NewRule,
        caller_ip: Option<String>,
        confirm: bool,
    },
    FirewallRemove {
        id: String,
        caller_ip: Option<String>,
        confirm: bool,
    },
    FirewallConfirm,
    /// Open SSH to one address for an hour.
    SshEmergency {
        caller_ip: String,
    },
    /// Reboot after `delay_min` minutes (1 to 60); can be cancelled until then.
    Reboot {
        delay_min: u32,
    },
    RebootCancel,
    /// Create or resize the swap file; 0 turns swap off and removes it.
    SwapSet {
        size_mb: u64,
    },
    HostnameSet {
        name: String,
    },
    TimezoneSet {
        zone: String,
    },
    /// File operations. `jail`, when set, is a directory the path must stay inside after
    /// all symbolic links are resolved (used to confine deployers to their apps).
    FileList {
        path: String,
        jail: Option<String>,
    },
    FileRead {
        path: String,
        jail: Option<String>,
    },
    /// Replace a file atomically. `expect_mtime` guards against overwriting someone else's edit.
    FileWrite {
        path: String,
        content: String,
        expect_mtime: Option<i64>,
        jail: Option<String>,
    },
    FileChmod {
        path: String,
        mode: String,
        jail: Option<String>,
    },
    FileMkdir {
        path: String,
        jail: Option<String>,
    },
    /// Move to the trash (kept 7 days), or delete for good.
    FileRemove {
        paths: Vec<String>,
        permanent: bool,
        jail: Option<String>,
    },
    /// Copy a file into the transfer directory so `serve` can stream it to the browser.
    FileStageDownload {
        path: String,
        jail: Option<String>,
    },
    /// Move an uploaded file from the transfer directory to `dir/name`.
    FileInstallUpload {
        staging: String,
        dir: String,
        name: String,
        overwrite: bool,
        jail: Option<String>,
    },
    /// Look for things that can be cleaned and estimate what each frees.
    CleanupScan,
    /// Force a fresh Cleanup scan (bypassing the 60 s cache).
    CleanupScanFresh,
    /// Compute unused Docker volume sizes on demand.
    CleanupVolumeSizes,
    /// Clean the named things, in a fixed order. Unknown or unavailable ones are reported, not run.
    CleanupRun {
        ids: Vec<String>,
    },
    /// Switch the weekly automatic clean (a systemd timer) on or off.
    CleanupAutoSet {
        on: bool,
    },
    /// The time zone (and the zones available) and the swap in use.
    SettingsGet,
    /// Install a verified staged opsbox binary into a version slot, switch symlink, and restart.
    SelfUpdateInstall {
        staging: String,
        version: String,
        target_schema: u32,
    },
    /// Roll back to the previous (or named) opsbox version slot and restart.
    SelfUpdateRollback {
        to_version: Option<String>,
    },
    /// Look at the things the security checks need.
    SecurityScan {
        apps_dir: String,
    },
    /// Apply the automatic fix of one check: `secrets`, `ssh-password` or `ssh-root`.
    SecurityFix {
        id: String,
        apps_dir: String,
    },
    /// Accounts, their SSH keys and the SSH daemon's settings.
    AccessList,
    /// Add one public key to a user's `authorized_keys`.
    SshKeyAdd {
        user: String,
        key: String,
    },
    /// Remove the key with this fingerprint from a user's `authorized_keys`.
    SshKeyRemove {
        user: String,
        fingerprint: String,
    },
    /// Lock (expire) or unlock a Linux account.
    UserLock {
        name: String,
        lock: bool,
    },
    /// Local accounts, for the terminal and the Access page.
    UserList,
    /// Start a login shell on a new PTY as `user`. On success the connection becomes a
    /// PTY stream (see [`crate::pty`]) and is no longer used for requests.
    PtyOpen {
        user: String,
        cols: u16,
        rows: u16,
    },
    /// Read the watchdog's latest health snapshot (read-only).
    WatchdogStatus,
    /// Docker daemon installation/running status.
    DockerStatus,
    /// Install Docker Engine from signed distribution or Docker repositories (never convenience script).
    DockerInstall,
    /// List containers (running and stopped).
    DockerListContainers,
    /// Inspect one container in detail.
    DockerInspect {
        id: String,
    },
    /// Start, stop, restart, or remove a container.
    DockerContainerAction {
        id: String,
        action: crate::docker::DockerContainerAction,
    },
    /// Fetch the last `tail` container log lines.
    DockerLogs {
        id: String,
        tail: u32,
    },
    /// Follow container logs live on this socket connection.
    DockerLogsFollow {
        id: String,
        tail: u32,
    },
    /// Sample CPU, memory, network, and PID counters for running containers.
    DockerStats,
    /// Run a command inside a running container.
    DockerExec(crate::docker::DockerExecRequest),
    /// List Docker images.
    DockerListImages,
    /// List Docker volumes.
    DockerListVolumes,
    /// List Docker networks.
    DockerListNetworks,
    /// Ensure `/var/lib/opsbox/apps/<service>` directory exists with safe ownership/mode.
    DeployEnsureDir {
        service: String,
    },
    /// Streaming clone/detect/build operation for a service deploy.
    DeployBuild(crate::deploy::DeployBuildRequest),
    /// Pull and retag a Docker image under `opsbox/<service>:<deploy_id>`.
    DeployPullImage(crate::deploy::DeployPullRequest),
    /// Start a release container on the project bridge network and verify HTTP/TCP health.
    DeployStartAndCheck(crate::deploy::DeployStartRequest),
    /// Stop and remove a container (used after zero-downtime drain).
    DeployStopContainer {
        container_id: String,
    },
    /// Check whether a retained local Docker image tag exists for rollback.
    DeployImageExists {
        image_ref: String,
    },
    /// Remove pruned image tags for a service while sparing any still in use.
    DeployPruneImages {
        service: String,
        image_refs: Vec<String>,
    },
    /// Remove a service's containers, retained images, and app directory.
    DeployDeleteService {
        service: String,
        container_ids: Vec<String>,
        image_refs: Vec<String>,
    },
    /// Pull image, create named volume(s), start a template service on the project network,
    /// and verify its health check.
    TemplateStart(crate::deploy::TemplateStartRequest),
    /// Rotate credentials inside a running database container via `docker exec` and return
    /// newly sealed variables.
    DatabaseRotate(crate::deploy::DatabaseRotateRequest),
    /// Read databases, users, grants, extensions, table sizes, connections and slow queries
    /// from a managed PostgreSQL container.
    DatabaseInspect(crate::database::DatabaseInspectRequest),
    /// Create/rotate/remove per-app logins and databases, or toggle an extension.
    DatabaseManage(crate::database::DatabaseManageRequest),
    /// Start or remove the PgBouncer container in front of a database.
    DatabasePooler(crate::database::DatabasePoolerRequest),
    /// Major version upgrade: snapshot first, then dump, new volume, restore, verify.
    DatabaseUpgrade(Box<crate::database::DatabaseUpgradeRequest>),
    /// Validate a Compose YAML file with `docker compose config` and safety policies.
    ComposeValidate(crate::deploy::ComposeValidateRequest),
    /// Validate, write `compose.yml`, and run `docker compose -p opsbox-<stack> up -d`.
    ComposeDeploy(crate::deploy::ComposeDeployRequest),
    /// Tear down a Compose stack (`docker compose -p opsbox-<stack> down`) and optionally remove files.
    ComposeDown {
        stack: String,
        remove_files: bool,
    },
    /// Stop and remove a single service container belonging to a Compose stack.
    ComposeRemoveService {
        stack: String,
        service: String,
    },
    /// Inspect local backup storage (`/var/backups/opsbox`), restic status, and same-disk mount checks.
    BackupLocalInfo {
        check_paths: Vec<String>,
    },
    /// Execute a backup job via restic (streamed DB dump, volume, path, or opsbox self-backup).
    BackupRun(crate::backup::BackupRunSpec),
    /// List restic snapshots in a destination repository.
    BackupSnapshots(crate::backup::BackupSnapshotsRequest),
    /// Enforce retention policy via `restic forget --keep-daily --keep-weekly --prune`.
    BackupForget(crate::backup::BackupForgetRequest),
    /// Run `restic check` to verify repository integrity and pack checksums.
    BackupCheck(crate::backup::BackupCheckRequest),
    /// Test connectivity, repo init, write/read probe, and delete permission for an offsite destination.
    BackupTestDestination(crate::backup::BackupDestinationTestRequest),
    /// Generate an Ed25519 SSH keypair inside `priv` and return it sealed for SFTP backups.
    BackupGenerateSftpKey(crate::backup::BackupGenerateSftpKeyRequest),
    /// Derive and return the deterministic restic repository recovery password.
    BackupRecoveryKey,
    /// Restore a snapshot (`copy` by default, or `replace` with typed confirmation + safety snapshot first).
    BackupRestore(crate::backup::BackupRestoreRequest),
    /// Verify a snapshot by restoring it into an ephemeral temporary container or directory.
    BackupRestoreTest(crate::backup::BackupRestoreTestRequest),
}

impl PrivOp {
    /// Stable name for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Capacity => "capacity",
            Self::CapacityImageSize { .. } => "capacity_image_size",
            Self::WorkloadMemory { .. } => "workload_memory",
            Self::CapacityAddSwap { .. } => "capacity_add_swap",
            Self::PortHolders { .. } => "port_holders",
            Self::ProxyNetwork => "proxy_network",
            Self::ProxyTakeover { .. } => "proxy_takeover",
            Self::ProxyUndo { .. } => "proxy_undo",
            Self::ProxyKeepPorts { .. } => "proxy_keep_ports",
            Self::HostInfo => "host_info",
            Self::ConsoleNotice { .. } => "console_notice",
            Self::Doctor => "doctor",
            Self::TotpNewSecret { .. } => "totp_new_secret",
            Self::TotpVerify { .. } => "totp_verify",
            Self::SecretSeal { .. } => "secret_seal",
            Self::SecretOpen { .. } => "secret_open",
            Self::WebLocalOnly { .. } => "web_local_only",
            Self::MetricsSample { .. } => "metrics_sample",
            Self::SystemFacts => "system_facts",
            Self::PkgList => "pkg_list",
            Self::PkgInstall { .. } => "pkg_install",
            Self::PkgInstallSelected { .. } => "pkg_install_selected",
            Self::AutoUpdatesSet { .. } => "auto_updates_set",
            Self::UnitList => "unit_list",
            Self::UnitAction { .. } => "unit_action",
            Self::JournalQuery { .. } => "journal_query",
            Self::JobList => "job_list",
            Self::JobAdd { .. } => "job_add",
            Self::JobRun { .. } => "job_run",
            Self::JobRemove { .. } => "job_remove",
            Self::FirewallList => "firewall_list",
            Self::FirewallAdd { .. } => "firewall_add",
            Self::FirewallRemove { .. } => "firewall_remove",
            Self::FirewallConfirm => "firewall_confirm",
            Self::SshEmergency { .. } => "ssh_emergency",
            Self::Reboot { .. } => "reboot",
            Self::RebootCancel => "reboot_cancel",
            Self::SwapSet { .. } => "swap_set",
            Self::HostnameSet { .. } => "hostname_set",
            Self::TimezoneSet { .. } => "timezone_set",
            Self::FileList { .. } => "file_list",
            Self::FileRead { .. } => "file_read",
            Self::FileWrite { .. } => "file_write",
            Self::FileChmod { .. } => "file_chmod",
            Self::FileMkdir { .. } => "file_mkdir",
            Self::FileRemove { .. } => "file_remove",
            Self::FileStageDownload { .. } => "file_stage_download",
            Self::FileInstallUpload { .. } => "file_install_upload",
            Self::CleanupScan => "cleanup_scan",
            Self::CleanupScanFresh => "cleanup_scan_fresh",
            Self::CleanupVolumeSizes => "cleanup_volume_sizes",
            Self::CleanupRun { .. } => "cleanup_run",
            Self::CleanupAutoSet { .. } => "cleanup_auto_set",
            Self::SettingsGet => "settings_get",
            Self::SelfUpdateInstall { .. } => "self_update_install",
            Self::SelfUpdateRollback { .. } => "self_update_rollback",
            Self::SecurityScan { .. } => "security_scan",
            Self::SecurityFix { .. } => "security_fix",
            Self::AccessList => "access_list",
            Self::SshKeyAdd { .. } => "ssh_key_add",
            Self::SshKeyRemove { .. } => "ssh_key_remove",
            Self::UserLock { .. } => "user_lock",
            Self::UserList => "user_list",
            Self::PtyOpen { .. } => "pty_open",
            Self::WatchdogStatus => "watchdog_status",
            Self::DockerStatus => "docker_status",
            Self::DockerInstall => "docker_install",
            Self::DockerListContainers => "docker_list_containers",
            Self::DockerInspect { .. } => "docker_inspect",
            Self::DockerContainerAction { .. } => "docker_container_action",
            Self::DockerLogs { .. } => "docker_logs",
            Self::DockerLogsFollow { .. } => "docker_logs_follow",
            Self::DockerStats => "docker_stats",
            Self::DockerExec { .. } => "docker_exec",
            Self::DockerListImages => "docker_list_images",
            Self::DockerListVolumes => "docker_list_volumes",
            Self::DockerListNetworks => "docker_list_networks",
            Self::DeployEnsureDir { .. } => "deploy_ensure_dir",
            Self::DeployBuild { .. } => "deploy_build",
            Self::DeployPullImage { .. } => "deploy_pull_image",
            Self::DeployStartAndCheck { .. } => "deploy_start_and_check",
            Self::DeployStopContainer { .. } => "deploy_stop_container",
            Self::DeployImageExists { .. } => "deploy_image_exists",
            Self::DeployPruneImages { .. } => "deploy_prune_images",
            Self::DeployDeleteService { .. } => "deploy_delete_service",
            Self::TemplateStart(_) => "template_start",
            Self::DatabaseRotate(_) => "database_rotate",
            Self::DatabaseInspect(_) => "database_inspect",
            Self::DatabaseManage(_) => "database_manage",
            Self::DatabasePooler(_) => "database_pooler",
            Self::DatabaseUpgrade(_) => "database_upgrade",
            Self::ComposeValidate(_) => "compose_validate",
            Self::ComposeDeploy(_) => "compose_deploy",
            Self::ComposeDown { .. } => "compose_down",
            Self::ComposeRemoveService { .. } => "compose_remove_service",
            Self::BackupLocalInfo { .. } => "backup_local_info",
            Self::BackupRun(_) => "backup_run",
            Self::BackupSnapshots(_) => "backup_snapshots",
            Self::BackupForget(_) => "backup_forget",
            Self::BackupCheck(_) => "backup_check",
            Self::BackupTestDestination(_) => "backup_test_destination",
            Self::BackupGenerateSftpKey(_) => "backup_generate_sftp_key",
            Self::BackupRecoveryKey => "backup_recovery_key",
            Self::BackupRestore(_) => "backup_restore",
            Self::BackupRestoreTest(_) => "backup_restore_test",
        }
    }
}

/// A response frame. `id` echoes the request id (empty when the request was too
/// broken to carry one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrivResponse {
    pub id: String,
    pub result: PrivResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrivResult {
    #[serde(rename = "ok")]
    Success { reply: PrivReply },
    #[serde(rename = "error")]
    Failure { error: PrivError },
}

impl PrivResponse {
    pub fn success(id: impl Into<String>, reply: PrivReply) -> Self {
        Self {
            id: id.into(),
            result: PrivResult::Success { reply },
        }
    }

    pub fn failure(id: impl Into<String>, code: PrivErrorCode, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            result: PrivResult::Failure {
                error: PrivError {
                    code,
                    message: message.into(),
                },
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum PrivReply {
    Capacity(Box<opsbox_capacity::Snapshot>),
    CapacityImageSize {
        disk_mb: u64,
    },
    PortHolders(Vec<crate::PortHolder>),
    ProxyNetwork {
        network: Box<crate::ProxyNetwork>,
        undo: Vec<crate::PortTakeover>,
    },
    PortTakeover(crate::PortTakeover),
    Pong {
        /// Version of the `priv` process, so `status` can spot a stale helper.
        version: String,
    },
    HostInfo(HostInfo),
    Doctor(crate::doctor::DoctorReport),
    /// Answer to [`PrivOp::TotpNewSecret`].
    TotpSecret {
        /// Encrypted with the master key; store this.
        sealed: String,
        /// RFC 4648 base32, for the authenticator app. Shown once, never stored.
        secret_base32: Secret,
    },
    /// Answer to [`PrivOp::TotpVerify`]: the time step that matched, if any.
    TotpResult {
        matched_step: Option<u64>,
    },
    /// Answer to [`PrivOp::SecretSeal`].
    Sealed {
        sealed: String,
    },
    /// Answer to [`PrivOp::SecretOpen`].
    Opened {
        plaintext: Secret,
    },
    /// Answer to [`PrivOp::MetricsSample`].
    Metrics(Box<crate::metrics::RawSample>),
    /// Answer to [`PrivOp::SystemFacts`].
    SystemFacts(crate::metrics::SystemFacts),
    Pkgs(crate::system::PkgState),
    Outcome(crate::system::CommandOutcome),
    Units(Vec<crate::system::UnitInfo>),
    Journal(Vec<crate::system::JournalEntry>),
    Jobs(Vec<crate::system::JobInfo>),
    Firewall(crate::system::FirewallState),
    FirewallChange(crate::system::FirewallChange),
    /// A job was created; this is its id.
    JobId(String),
    /// Nothing to report: the operation succeeded.
    Done,
    Listing(crate::files::DirListing),
    FileContent(crate::files::FileContent),
    Staged(crate::files::Staged),
    /// How many items were removed.
    Removed(u32),
    /// A file was replaced; this is its new modification time (unix seconds).
    Written {
        mtime: i64,
    },
    Users(Vec<crate::pty::SystemUser>),
    Access(crate::access::AccessState),
    Security(crate::security::SecurityScan),
    Settings(crate::security::HostSettings),
    /// How many keys were removed.
    KeysRemoved(u32),
    /// A key was added; this is its fingerprint.
    KeyAdded {
        fingerprint: String,
    },
    Cleanup(crate::cleanup::CleanupScan),
    CleanupFreed(Vec<crate::cleanup::CleanupFreed>),
    CleanupVolumeSize {
        bytes: u64,
        count: usize,
    },
    /// A shell is running; the connection now carries the PTY stream.
    PtyStarted {
        user: String,
        shell: String,
    },
    /// Answer to [`PrivOp::WatchdogStatus`].
    WatchdogStatus(Box<crate::watchdog::WatchdogStatus>),
    /// Answer to [`PrivOp::DockerStatus`].
    DockerStatus(crate::docker::DockerStatus),
    /// Answer to [`PrivOp::DockerListContainers`].
    DockerContainers(Vec<crate::docker::DockerContainer>),
    /// Answer to [`PrivOp::DockerInspect`].
    DockerInspect(Box<crate::docker::DockerContainerInspect>),
    /// Answer to [`PrivOp::DockerLogs`].
    DockerLogs(Vec<crate::docker::DockerLogLine>),
    /// Answer to [`PrivOp::DockerLogsFollow`]; the connection now streams [`crate::docker::DockerLogFrame`]s.
    DockerLogStreamStarted {
        id: String,
    },
    /// Answer to [`PrivOp::DockerStats`].
    DockerStats(Vec<crate::docker::DockerContainerStat>),
    /// Answer to [`PrivOp::DockerExec`].
    DockerExec(crate::docker::DockerExecOutput),
    /// Answer to [`PrivOp::DockerListImages`].
    DockerImages(Vec<crate::docker::DockerImage>),
    /// Answer to [`PrivOp::DockerListVolumes`].
    DockerVolumes(Vec<crate::docker::DockerVolume>),
    /// Answer to [`PrivOp::DockerListNetworks`].
    DockerNetworks(Vec<crate::docker::DockerNetwork>),
    /// Answer to [`PrivOp::DeployBuild`]; stream transitions to [`crate::deploy::DeployBuildFrame`].
    DeployBuildStreamStarted {
        deploy_id: String,
    },
    /// Answer to [`PrivOp::DeployStartAndCheck`].
    DeployStarted(crate::deploy::DeployStartResult),
    /// Answer to [`PrivOp::DeployImageExists`].
    Bool(bool),
    /// Answer to [`PrivOp::DeployPruneImages`] (image refs actually removed).
    PrunedImages(Vec<String>),
    /// Answer to [`PrivOp::ComposeValidate`].
    ComposeValidated(crate::deploy::ComposeValidationResult),
    /// Answer to [`PrivOp::ComposeDeploy`].
    ComposeDeployed(crate::deploy::ComposeDeployResult),
    /// Answer to [`PrivOp::DatabaseRotate`].
    DatabaseRotated(crate::deploy::DatabaseRotateResult),
    /// Answer to [`PrivOp::DatabaseInspect`].
    DatabaseInspected(Box<crate::database::DatabaseInspectResult>),
    /// Answer to [`PrivOp::DatabaseManage`].
    DatabaseManaged(crate::database::DatabaseManageResult),
    /// Answer to [`PrivOp::DatabasePooler`].
    DatabasePooler(crate::database::DatabasePoolerResult),
    /// Answer to [`PrivOp::DatabaseUpgrade`].
    DatabaseUpgraded(Box<crate::database::DatabaseUpgradeResult>),
    /// Answer to [`PrivOp::BackupLocalInfo`].
    BackupLocalInfo(Box<crate::backup::BackupLocalInfo>),
    /// Answer to [`PrivOp::BackupRun`].
    BackupRun(Box<crate::backup::BackupRunResult>),
    /// Answer to [`PrivOp::BackupSnapshots`].
    BackupSnapshots(Vec<crate::backup::ResticSnapshotInfo>),
    /// Answer to [`PrivOp::BackupForget`].
    BackupForget(Box<crate::backup::BackupForgetResult>),
    /// Answer to [`PrivOp::BackupCheck`].
    BackupCheck(Box<crate::backup::BackupCheckResult>),
    /// Answer to [`PrivOp::BackupTestDestination`].
    BackupTestDestination(crate::backup::BackupDestinationTestResult),
    /// Answer to [`PrivOp::BackupGenerateSftpKey`].
    BackupSftpKey(crate::backup::BackupSftpKeyResult),
    /// Answer to [`PrivOp::BackupRecoveryKey`].
    BackupRecoveryKey(crate::backup::BackupRecoveryKeyResult),
    /// Answer to [`PrivOp::BackupRestore`].
    BackupRestore(Box<crate::backup::BackupRestoreResult>),
    /// Answer to [`PrivOp::BackupRestoreTest`].
    BackupRestoreTest(Box<crate::backup::BackupRestoreTestResult>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivError {
    pub code: PrivErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivErrorCode {
    /// The frame was not valid JSON or not a known operation.
    BadRequest,
    /// The peer's user id is not allowed to talk to `priv`.
    Forbidden,
    /// Known operation that this platform cannot perform.
    Unsupported,
    /// The arguments were rejected; the message says why, in words for a person.
    Invalid,
    /// Policy says no (for example stopping opsbox itself); the message says why.
    Refused,
    /// The change could cut the caller off. Repeat it with `confirm` to go ahead.
    ConfirmRequired,
    NotFound,
    /// The target changed or exists already (for example a file edited by someone else).
    Conflict,
    Internal,
}

/// Result of [`PrivOp::HostInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub hostname: String,
    /// Pretty OS name, from `PRETTY_NAME` in `/etc/os-release`.
    pub os: String,
    pub kernel: String,
    pub uptime_secs: u64,
    pub cpu_count: u32,
    pub mem_total_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_ops_have_a_simple_wire_form() {
        let req = PrivRequest {
            id: "01H".into(),
            actor: None,
            op: PrivOp::Ping,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"id": "01H", "actor": null, "op": {"type": "ping"}})
        );
    }

    #[test]
    fn unknown_operations_do_not_parse() {
        for bad in [
            r#"{"id":"1","op":{"type":"run_shell","args":{"cmd":"id"}}}"#,
            r#"{"id":"1","op":{"type":"Ping"}}"#,
            r#"{"id":"1","op":"ping"}"#,
            r#"{"id":"1","op":{"type":"ping"},"extra":true}"#,
        ] {
            assert!(serde_json::from_str::<PrivRequest>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn response_round_trips() {
        let ok = PrivResponse::success(
            "a",
            PrivReply::Pong {
                version: "1".into(),
            },
        );
        let err = PrivResponse::failure("b", PrivErrorCode::Forbidden, "no");
        for r in [ok, err] {
            let s = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_json::from_str::<PrivResponse>(&s).unwrap(), r);
        }
    }
}

#[cfg(test)]
mod secret_tests {
    use super::*;

    #[test]
    fn secrets_never_show_up_in_debug_output() {
        let reply = PrivReply::TotpSecret {
            sealed: "enc:v1:x".into(),
            secret_base32: Secret::new("JBSWY3DPEHPK3PXP"),
        };
        let op = PrivOp::TotpVerify {
            sealed: "enc:v1:x".into(),
            context: "totp:u".into(),
            code: Secret::new("123456"),
            after_step: 0,
        };
        let shown = format!("{reply:?} {op:?}");
        assert!(
            !shown.contains("JBSWY3DPEHPK3PXP") && !shown.contains("123456"),
            "{shown}"
        );
        assert!(shown.contains("[redacted]"));
        // ...but they still travel as plain strings.
        let json = serde_json::to_string(&op).unwrap();
        assert!(json.contains("\"123456\""), "{json}");
    }
}
