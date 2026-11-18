use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use opsbox_proto::frame;
use opsbox_proto::{
    AccessState, BackupCheckRequest, BackupCheckResult, BackupForgetRequest, BackupForgetResult,
    BackupLocalInfo, BackupRunResult, BackupRunSpec, BackupSnapshotsRequest, CleanupFreed,
    CleanupScan, CommandOutcome, ComposeDeployRequest, ComposeDeployResult, ComposeValidateRequest,
    ComposeValidationResult, DatabaseInspectRequest, DatabaseInspectResult, DatabaseManageRequest,
    DatabaseManageResult, DatabasePoolerRequest, DatabasePoolerResult, DatabaseRotateRequest,
    DatabaseRotateResult, DatabaseUpgradeRequest, DatabaseUpgradeResult, DeployBuildFrame,
    DeployBuildRequest, DeployPullRequest, DeployStartRequest, DeployStartResult, DirListing,
    DockerContainer, DockerContainerAction, DockerContainerInspect, DockerContainerStat,
    DockerExecOutput, DockerExecRequest, DockerImage, DockerLogFrame, DockerLogLine, DockerNetwork,
    DockerStatus, DockerVolume, FileContent, FirewallChange, FirewallState, HostInfo, HostSettings,
    JobInfo, JournalEntry, NewRule, PkgSelect, PkgState, PrivError, PrivErrorCode, PrivOp,
    PrivReply, PrivRequest, PrivResponse, PrivResult, PtyFrame, RawSample, ResticSnapshotInfo,
    Secret, SecurityScan, Staged, SystemFacts, SystemUser, TemplateStartRequest, UnitActionKind,
    UnitInfo, WatchdogStatus,
};
use tokio::net::{
    UnixStream,
    unix::{OwnedReadHalf, OwnedWriteHalf},
};

use crate::io::{
    PtyIoError, ReadError, WriteError, read_body, read_pty_frame, write_frame, write_pty_frame,
};

/// How long a single call may take, connect included.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Could not connect. [`ClientError::connect_kind`] says why (not running,
    /// permission denied, ...).
    #[error("cannot reach the privileged helper at {path}: {source}")]
    Connect { path: String, source: io::Error },
    #[error("the privileged helper did not answer in time")]
    Timeout,
    #[error("communication with the privileged helper failed: {0}")]
    Io(String),
    #[error("the privileged helper sent something unexpected: {0}")]
    Protocol(String),
    /// `priv` understood the request and refused or failed it.
    #[error("the privileged helper reported an error: {}", .0.message)]
    Remote(PrivError),
}

/// One log line per failed call, with what a person needs to tell "helper down" from
/// "helper slow". A refusal `priv` words itself (a name already taken, say) is the
/// caller's mistake, not a helper failure, so it stays at debug level.
fn log_failure(request_id: &str, op: &str, elapsed: Duration, error: &ClientError) {
    let elapsed_ms = elapsed.as_millis() as u64;
    match error {
        ClientError::Remote(remote) if remote.code != PrivErrorCode::Forbidden => tracing::debug!(
            request_id,
            op,
            elapsed_ms,
            error = %error,
            "privileged helper call failed"
        ),
        _ => tracing::warn!(
            request_id,
            op,
            elapsed_ms,
            error = %error,
            "privileged helper call failed"
        ),
    }
}

impl ClientError {
    /// For `Connect` errors, the underlying I/O error kind.
    pub fn connect_kind(&self) -> Option<io::ErrorKind> {
        match self {
            Self::Connect { source, .. } => Some(source.kind()),
            _ => None,
        }
    }
}

impl From<ReadError> for ClientError {
    fn from(e: ReadError) -> Self {
        match e {
            ReadError::Frame(e) => Self::Protocol(e.to_string()),
            ReadError::Io(e) => Self::Io(e.to_string()),
        }
    }
}

impl From<WriteError> for ClientError {
    fn from(e: WriteError) -> Self {
        match e {
            WriteError::Frame(e) => Self::Protocol(e.to_string()),
            WriteError::Io(e) => Self::Io(e.to_string()),
        }
    }
}

/// What to install from the transfer folder (see [`PrivClient::file_install_upload`]).
#[derive(Debug, Clone, Copy)]
pub struct InstallUpload<'a> {
    /// The staged file, inside the transfer folder.
    pub staging: &'a str,
    /// The destination folder.
    pub dir: &'a str,
    /// The file name there.
    pub name: &'a str,
    pub overwrite: bool,
}

/// Live container log follow stream returned by [`PrivClient::docker_logs_follow`].
#[derive(Debug)]
pub struct DockerLogSession {
    pub id: String,
    reader: OwnedReadHalf,
    _writer: OwnedWriteHalf,
}

impl DockerLogSession {
    /// Read the next [`DockerLogFrame`]; `Ok(None)` means the stream ended cleanly.
    pub async fn recv(&mut self) -> Result<Option<DockerLogFrame>, ClientError> {
        let Some(body) = read_body(&mut self.reader).await? else {
            return Ok(None);
        };
        let frame: DockerLogFrame =
            frame::decode(&body).map_err(|e| ClientError::Protocol(e.to_string()))?;
        Ok(Some(frame))
    }
}

/// Streaming deploy build session returned by [`PrivClient::deploy_build`].
#[derive(Debug)]
pub struct DeployBuildSession {
    pub deploy_id: String,
    reader: OwnedReadHalf,
    _writer: OwnedWriteHalf,
}

impl DeployBuildSession {
    /// Read the next [`DeployBuildFrame`]; `Ok(None)` means the stream closed.
    pub async fn recv(&mut self) -> Result<Option<DeployBuildFrame>, ClientError> {
        let Some(body) = read_body(&mut self.reader).await? else {
            return Ok(None);
        };
        let frame: DeployBuildFrame =
            frame::decode(&body).map_err(|e| ClientError::Protocol(e.to_string()))?;
        Ok(Some(frame))
    }
}

/// An open terminal: the shell's output arrives through `reader`, keystrokes go out through
/// `writer`. Dropping both ends the shell.
#[derive(Debug)]
pub struct PtySession {
    pub user: String,
    pub shell: String,
    pub reader: PtyReader,
    pub writer: PtyWriter,
}

#[derive(Debug)]
pub struct PtyReader(OwnedReadHalf);

#[derive(Debug)]
pub struct PtyWriter(OwnedWriteHalf);

impl PtyReader {
    /// The next message from the shell side; `None` once the stream is closed. Not
    /// cancel-safe: do not use it as a `select!` branch that may lose the race.
    pub async fn recv(&mut self) -> Result<Option<PtyFrame>, PtyIoError> {
        read_pty_frame(&mut self.0).await
    }
}

impl PtyWriter {
    pub async fn send(&mut self, frame: &PtyFrame) -> Result<(), PtyIoError> {
        write_pty_frame(&mut self.0, frame)
            .await
            .map_err(Into::into)
    }
}

/// Calls `priv` over its Unix socket. Cheap to clone. Each call opens its own
/// connection, so a restarted `priv` is picked up without any reconnect logic.
#[derive(Debug, Clone)]
pub struct PrivClient {
    socket_path: Arc<PathBuf>,
    timeout: Duration,
}

impl PrivClient {
    pub async fn pkg_install_selected(
        &self,
        rid: &str,
        actor: &str,
        names: &[String],
    ) -> Result<CommandOutcome, ClientError> {
        self.expect(
            rid,
            Some(actor),
            PrivOp::PkgInstallSelected {
                names: names.to_vec(),
            },
            |r| match r {
                PrivReply::Outcome(outcome) => Ok(outcome),
                other => Err(other),
            },
        )
        .await
    }
    pub async fn doctor(&self, rid: &str) -> Result<opsbox_proto::DoctorReport, ClientError> {
        self.expect(rid, None, PrivOp::Doctor, |r| match r {
            PrivReply::Doctor(report) => Ok(report),
            other => Err(other),
        })
        .await
    }
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: Arc::new(socket_path.into()),
            timeout: CALL_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub async fn capacity(&self, rid: &str) -> Result<opsbox_capacity::Snapshot, ClientError> {
        match self.call(rid, None, PrivOp::Capacity).await? {
            PrivReply::Capacity(s) => Ok(*s),
            _ => Err(ClientError::Protocol("expected capacity profile".into())),
        }
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    /// Perform `op`. `request_id` appears in the logs of both processes; `actor`
    /// is who it is done for.
    pub async fn call(
        &self,
        request_id: &str,
        actor: Option<&str>,
        op: PrivOp,
    ) -> Result<PrivReply, ClientError> {
        let name = op.name();
        let started = Instant::now();
        let result = tokio::time::timeout(self.timeout, self.call_inner(request_id, actor, op))
            .await
            .map_err(|_| ClientError::Timeout)
            .and_then(|r| r);
        if let Err(e) = &result {
            log_failure(request_id, name, started.elapsed(), e);
        }
        result
    }

    async fn call_inner(
        &self,
        request_id: &str,
        actor: Option<&str>,
        op: PrivOp,
    ) -> Result<PrivReply, ClientError> {
        let mut stream = UnixStream::connect(&*self.socket_path)
            .await
            .map_err(|source| ClientError::Connect {
                path: self.socket_path.display().to_string(),
                source,
            })?;

        let request = PrivRequest {
            id: request_id.to_owned(),
            actor: actor.map(str::to_owned),
            op,
        };
        write_frame(&mut stream, &request).await?;

        let body = read_body(&mut stream)
            .await?
            .ok_or_else(|| ClientError::Protocol("connection closed without an answer".into()))?;
        let response: PrivResponse =
            frame::decode(&body).map_err(|e| ClientError::Protocol(e.to_string()))?;

        match response.result {
            PrivResult::Failure { error } => Err(ClientError::Remote(error)),
            PrivResult::Success { reply } if response.id == request_id => Ok(reply),
            PrivResult::Success { .. } => Err(ClientError::Protocol(format!(
                "answer is for request {:?}, not {request_id:?}",
                response.id
            ))),
        }
    }

    /// Liveness check. Returns the helper's version.
    pub async fn ping(&self, request_id: &str) -> Result<String, ClientError> {
        match self.call(request_id, None, PrivOp::Ping).await? {
            PrivReply::Pong { version } => Ok(version),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    pub async fn host_info(
        &self,
        request_id: &str,
        actor: Option<&str>,
    ) -> Result<HostInfo, ClientError> {
        match self.call(request_id, actor, PrivOp::HostInfo).await? {
            PrivReply::HostInfo(info) => Ok(info),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// Create a TOTP secret bound to `context` (e.g. `totp:<user id>`). Returns the
    /// sealed value to store and the base32 secret to show once.
    pub async fn totp_new_secret(
        &self,
        request_id: &str,
        context: &str,
    ) -> Result<(String, Secret), ClientError> {
        let op = PrivOp::TotpNewSecret {
            context: context.to_owned(),
        };
        match self.call(request_id, None, op).await? {
            PrivReply::TotpSecret {
                sealed,
                secret_base32,
            } => Ok((sealed, secret_base32)),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// The time step that `code` matches (newer than `after_step`), if any.
    pub async fn totp_verify(
        &self,
        request_id: &str,
        sealed: &str,
        context: &str,
        code: &str,
        after_step: u64,
    ) -> Result<Option<u64>, ClientError> {
        let op = PrivOp::TotpVerify {
            sealed: sealed.to_owned(),
            context: context.to_owned(),
            code: Secret::new(code),
            after_step,
        };
        match self.call(request_id, None, op).await? {
            PrivReply::TotpResult { matched_step } => Ok(matched_step),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// Cumulative machine counters. `procs` also scans every process.
    pub async fn metrics_sample(
        &self,
        request_id: &str,
        procs: bool,
    ) -> Result<RawSample, ClientError> {
        match self
            .call(request_id, None, PrivOp::MetricsSample { procs })
            .await?
        {
            PrivReply::Metrics(sample) => Ok(*sample),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    pub async fn system_facts(&self, request_id: &str) -> Result<SystemFacts, ClientError> {
        match self.call(request_id, None, PrivOp::SystemFacts).await? {
            PrivReply::SystemFacts(f) => Ok(f),
            other => Err(ClientError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// Perform `op` for `actor` and pick the expected kind of reply out of the answer.
    async fn expect<T>(
        &self,
        request_id: &str,
        actor: Option<&str>,
        op: PrivOp,
        pick: impl FnOnce(PrivReply) -> Result<T, PrivReply>,
    ) -> Result<T, ClientError> {
        let reply = self.call(request_id, actor, op).await?;
        pick(reply).map_err(|other| ClientError::Protocol(format!("unexpected reply {other:?}")))
    }

    pub async fn pkg_list(&self, rid: &str) -> Result<PkgState, ClientError> {
        self.expect(rid, None, PrivOp::PkgList, |r| match r {
            PrivReply::Pkgs(p) => Ok(p),
            o => Err(o),
        })
        .await
    }

    pub async fn pkg_install(
        &self,
        rid: &str,
        actor: &str,
        which: PkgSelect,
    ) -> Result<CommandOutcome, ClientError> {
        self.expect(
            rid,
            Some(actor),
            PrivOp::PkgInstall { which },
            |r| match r {
                PrivReply::Outcome(o) => Ok(o),
                o => Err(o),
            },
        )
        .await
    }

    pub async fn auto_updates_set(
        &self,
        rid: &str,
        actor: &str,
        on: bool,
    ) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::AutoUpdatesSet { on }).await
    }

    pub async fn unit_list(&self, rid: &str) -> Result<Vec<UnitInfo>, ClientError> {
        self.expect(rid, None, PrivOp::UnitList, |r| match r {
            PrivReply::Units(u) => Ok(u),
            o => Err(o),
        })
        .await
    }

    pub async fn unit_action(
        &self,
        rid: &str,
        actor: &str,
        name: &str,
        action: UnitActionKind,
    ) -> Result<(), ClientError> {
        self.done(
            rid,
            actor,
            PrivOp::UnitAction {
                name: name.to_owned(),
                action,
            },
        )
        .await
    }

    pub async fn journal(
        &self,
        rid: &str,
        unit: Option<&str>,
        max_priority: u8,
        text: Option<&str>,
        limit: u32,
        after: Option<&str>,
    ) -> Result<Vec<JournalEntry>, ClientError> {
        let op = PrivOp::JournalQuery {
            unit: unit.map(str::to_owned),
            max_priority,
            text: text.map(str::to_owned),
            limit,
            after: after.map(str::to_owned),
        };
        self.expect(rid, None, op, |r| match r {
            PrivReply::Journal(j) => Ok(j),
            o => Err(o),
        })
        .await
    }

    pub async fn job_list(&self, rid: &str) -> Result<Vec<JobInfo>, ClientError> {
        self.expect(rid, None, PrivOp::JobList, |r| match r {
            PrivReply::Jobs(j) => Ok(j),
            o => Err(o),
        })
        .await
    }

    pub async fn job_add(
        &self,
        rid: &str,
        actor: &str,
        name: &str,
        cron: &str,
        command: &str,
    ) -> Result<String, ClientError> {
        let op = PrivOp::JobAdd {
            name: name.to_owned(),
            cron: cron.to_owned(),
            command: command.to_owned(),
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::JobId(id) => Ok(id),
            o => Err(o),
        })
        .await
    }

    pub async fn job_run(&self, rid: &str, actor: &str, id: &str) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::JobRun { id: id.to_owned() })
            .await
    }

    pub async fn job_remove(&self, rid: &str, actor: &str, id: &str) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::JobRemove { id: id.to_owned() })
            .await
    }

    pub async fn firewall_list(&self, rid: &str) -> Result<FirewallState, ClientError> {
        self.expect(rid, None, PrivOp::FirewallList, |r| match r {
            PrivReply::Firewall(f) => Ok(f),
            o => Err(o),
        })
        .await
    }

    pub async fn firewall_add(
        &self,
        rid: &str,
        actor: &str,
        rule: NewRule,
        caller_ip: Option<&str>,
        confirm: bool,
    ) -> Result<FirewallChange, ClientError> {
        let op = PrivOp::FirewallAdd {
            rule,
            caller_ip: caller_ip.map(str::to_owned),
            confirm,
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::FirewallChange(c) => Ok(c),
            o => Err(o),
        })
        .await
    }

    pub async fn firewall_remove(
        &self,
        rid: &str,
        actor: &str,
        id: &str,
        caller_ip: Option<&str>,
        confirm: bool,
    ) -> Result<FirewallChange, ClientError> {
        let op = PrivOp::FirewallRemove {
            id: id.to_owned(),
            caller_ip: caller_ip.map(str::to_owned),
            confirm,
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::FirewallChange(c) => Ok(c),
            o => Err(o),
        })
        .await
    }

    pub async fn firewall_confirm(&self, rid: &str, actor: &str) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::FirewallConfirm).await
    }

    /// Open SSH to `caller_ip` for an hour; returns the unix time it closes.
    pub async fn ssh_emergency(
        &self,
        rid: &str,
        actor: &str,
        caller_ip: &str,
    ) -> Result<i64, ClientError> {
        let op = PrivOp::SshEmergency {
            caller_ip: caller_ip.to_owned(),
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::FirewallChange(FirewallChange { revert_at: Some(t) }) => Ok(t),
            o => Err(o),
        })
        .await
    }

    pub async fn reboot(&self, rid: &str, actor: &str, delay_min: u32) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::Reboot { delay_min }).await
    }

    pub async fn reboot_cancel(&self, rid: &str, actor: &str) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::RebootCancel).await
    }

    pub async fn swap_set(&self, rid: &str, actor: &str, size_mb: u64) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::SwapSet { size_mb }).await
    }

    pub async fn disk_state(&self, rid: &str) -> Result<opsbox_proto::DiskState, ClientError> {
        self.expect(rid, None, PrivOp::DiskState, |r| match r {
            PrivReply::Disk(d) => Ok(*d),
            o => Err(o),
        })
        .await
    }

    /// Run one step of growing the filesystem at `mount`; returns what the tool printed.
    pub async fn disk_grow(
        &self,
        rid: &str,
        actor: &str,
        mount: &str,
        step: opsbox_proto::DiskStep,
    ) -> Result<Vec<String>, ClientError> {
        let op = PrivOp::DiskGrow {
            mount: mount.to_owned(),
            step,
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::DiskStep { log } => Ok(log),
            o => Err(o),
        })
        .await
    }

    pub async fn hostname_set(
        &self,
        rid: &str,
        actor: &str,
        name: &str,
    ) -> Result<(), ClientError> {
        self.done(
            rid,
            actor,
            PrivOp::HostnameSet {
                name: name.to_owned(),
            },
        )
        .await
    }

    pub async fn timezone_set(
        &self,
        rid: &str,
        actor: &str,
        zone: &str,
    ) -> Result<(), ClientError> {
        self.done(
            rid,
            actor,
            PrivOp::TimezoneSet {
                zone: zone.to_owned(),
            },
        )
        .await
    }

    /// An operation whose only answer is "done".
    async fn done(&self, rid: &str, actor: &str, op: PrivOp) -> Result<(), ClientError> {
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::Done => Ok(()),
            o => Err(o),
        })
        .await
    }

    pub async fn cleanup_scan(&self, rid: &str) -> Result<CleanupScan, ClientError> {
        self.cleanup_scan_opts(rid, false).await
    }

    pub async fn cleanup_scan_opts(
        &self,
        rid: &str,
        fresh: bool,
    ) -> Result<CleanupScan, ClientError> {
        let op = if fresh {
            PrivOp::CleanupScanFresh
        } else {
            PrivOp::CleanupScan
        };
        self.expect(rid, None, op, |r| match r {
            PrivReply::Cleanup(s) => Ok(s),
            o => Err(o),
        })
        .await
    }

    pub async fn cleanup_volume_sizes(&self, rid: &str) -> Result<(u64, usize), ClientError> {
        self.expect(rid, None, PrivOp::CleanupVolumeSizes, |r| match r {
            PrivReply::CleanupVolumeSize { bytes, count } => Ok((bytes, count)),
            o => Err(o),
        })
        .await
    }

    pub async fn cleanup_run(
        &self,
        rid: &str,
        actor: &str,
        ids: &[String],
    ) -> Result<Vec<CleanupFreed>, ClientError> {
        let op = PrivOp::CleanupRun { ids: ids.to_vec() };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::CleanupFreed(f) => Ok(f),
            o => Err(o),
        })
        .await
    }

    pub async fn cleanup_auto_set(
        &self,
        rid: &str,
        actor: &str,
        on: bool,
    ) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::CleanupAutoSet { on }).await
    }

    pub async fn settings_get(&self, rid: &str) -> Result<HostSettings, ClientError> {
        self.expect(rid, None, PrivOp::SettingsGet, |r| match r {
            PrivReply::Settings(s) => Ok(s),
            o => Err(o),
        })
        .await
    }

    pub async fn self_update_install(
        &self,
        rid: &str,
        actor: &str,
        staging: &str,
        version: &str,
        target_schema: u32,
    ) -> Result<(), ClientError> {
        self.expect(
            rid,
            Some(actor),
            PrivOp::SelfUpdateInstall {
                staging: staging.to_owned(),
                version: version.to_owned(),
                target_schema,
            },
            |r| match r {
                PrivReply::Done => Ok(()),
                o => Err(o),
            },
        )
        .await
    }

    pub async fn self_update_rollback(
        &self,
        rid: &str,
        actor: &str,
        to_version: Option<&str>,
    ) -> Result<(), ClientError> {
        self.expect(
            rid,
            Some(actor),
            PrivOp::SelfUpdateRollback {
                to_version: to_version.map(ToOwned::to_owned),
            },
            |r| match r {
                PrivReply::Done => Ok(()),
                o => Err(o),
            },
        )
        .await
    }

    pub async fn security_scan(
        &self,
        rid: &str,
        apps_dir: &str,
    ) -> Result<SecurityScan, ClientError> {
        let op = PrivOp::SecurityScan {
            apps_dir: apps_dir.to_owned(),
        };
        self.expect(rid, None, op, |r| match r {
            PrivReply::Security(s) => Ok(s),
            o => Err(o),
        })
        .await
    }

    pub async fn security_fix(
        &self,
        rid: &str,
        actor: &str,
        id: &str,
        apps_dir: &str,
    ) -> Result<(), ClientError> {
        let op = PrivOp::SecurityFix {
            id: id.to_owned(),
            apps_dir: apps_dir.to_owned(),
        };
        self.done(rid, actor, op).await
    }

    pub async fn access_list(&self, rid: &str) -> Result<AccessState, ClientError> {
        self.expect(rid, None, PrivOp::AccessList, |r| match r {
            PrivReply::Access(a) => Ok(a),
            o => Err(o),
        })
        .await
    }

    pub async fn ssh_key_add(
        &self,
        rid: &str,
        actor: &str,
        user: &str,
        key: &str,
    ) -> Result<String, ClientError> {
        let op = PrivOp::SshKeyAdd {
            user: user.to_owned(),
            key: key.to_owned(),
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::KeyAdded { fingerprint } => Ok(fingerprint),
            o => Err(o),
        })
        .await
    }

    pub async fn ssh_key_remove(
        &self,
        rid: &str,
        actor: &str,
        user: &str,
        fingerprint: &str,
    ) -> Result<u32, ClientError> {
        let op = PrivOp::SshKeyRemove {
            user: user.to_owned(),
            fingerprint: fingerprint.to_owned(),
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::KeysRemoved(n) => Ok(n),
            o => Err(o),
        })
        .await
    }

    pub async fn user_lock(
        &self,
        rid: &str,
        actor: &str,
        name: &str,
        lock: bool,
    ) -> Result<(), ClientError> {
        let op = PrivOp::UserLock {
            name: name.to_owned(),
            lock,
        };
        self.done(rid, actor, op).await
    }

    /// Local accounts.
    pub async fn users(&self, rid: &str) -> Result<Vec<SystemUser>, ClientError> {
        self.expect(rid, None, PrivOp::UserList, |r| match r {
            PrivReply::Users(u) => Ok(u),
            o => Err(o),
        })
        .await
    }

    /// Start a login shell as `user`. The returned session owns the connection.
    pub async fn pty_open(
        &self,
        rid: &str,
        actor: Option<&str>,
        user: &str,
        cols: u16,
        rows: u16,
    ) -> Result<PtySession, ClientError> {
        let handshake = async {
            let mut stream = UnixStream::connect(&*self.socket_path)
                .await
                .map_err(|source| ClientError::Connect {
                    path: self.socket_path.display().to_string(),
                    source,
                })?;
            let request = PrivRequest {
                id: rid.to_owned(),
                actor: actor.map(str::to_owned),
                op: PrivOp::PtyOpen {
                    user: user.to_owned(),
                    cols,
                    rows,
                },
            };
            write_frame(&mut stream, &request).await?;
            let body = read_body(&mut stream).await?.ok_or_else(|| {
                ClientError::Protocol("connection closed without an answer".into())
            })?;
            let response: PrivResponse =
                frame::decode(&body).map_err(|e| ClientError::Protocol(e.to_string()))?;
            match response.result {
                PrivResult::Failure { error } => Err(ClientError::Remote(error)),
                PrivResult::Success {
                    reply: PrivReply::PtyStarted { user, shell },
                } => {
                    let (r, w) = stream.into_split();
                    Ok(PtySession {
                        user,
                        shell,
                        reader: PtyReader(r),
                        writer: PtyWriter(w),
                    })
                }
                PrivResult::Success { reply } => {
                    Err(ClientError::Protocol(format!("unexpected reply {reply:?}")))
                }
            }
        };
        tokio::time::timeout(self.timeout, handshake)
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    pub async fn file_list(
        &self,
        rid: &str,
        path: &str,
        jail: Option<&str>,
    ) -> Result<DirListing, ClientError> {
        let op = PrivOp::FileList {
            path: path.to_owned(),
            jail: jail.map(str::to_owned),
        };
        self.expect(rid, None, op, |r| match r {
            PrivReply::Listing(l) => Ok(l),
            o => Err(o),
        })
        .await
    }

    pub async fn file_read(
        &self,
        rid: &str,
        path: &str,
        jail: Option<&str>,
    ) -> Result<FileContent, ClientError> {
        let op = PrivOp::FileRead {
            path: path.to_owned(),
            jail: jail.map(str::to_owned),
        };
        self.expect(rid, None, op, |r| match r {
            PrivReply::FileContent(c) => Ok(c),
            o => Err(o),
        })
        .await
    }

    /// Returns the new modification time.
    pub async fn file_write(
        &self,
        rid: &str,
        actor: &str,
        path: &str,
        content: &str,
        expect_mtime: Option<i64>,
        jail: Option<&str>,
    ) -> Result<i64, ClientError> {
        let op = PrivOp::FileWrite {
            path: path.to_owned(),
            content: content.to_owned(),
            expect_mtime,
            jail: jail.map(str::to_owned),
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::Written { mtime } => Ok(mtime),
            o => Err(o),
        })
        .await
    }

    pub async fn file_chmod(
        &self,
        rid: &str,
        actor: &str,
        path: &str,
        mode: &str,
        jail: Option<&str>,
    ) -> Result<(), ClientError> {
        self.done(
            rid,
            actor,
            PrivOp::FileChmod {
                path: path.to_owned(),
                mode: mode.to_owned(),
                jail: jail.map(str::to_owned),
            },
        )
        .await
    }

    pub async fn file_mkdir(
        &self,
        rid: &str,
        actor: &str,
        path: &str,
        jail: Option<&str>,
    ) -> Result<(), ClientError> {
        self.done(
            rid,
            actor,
            PrivOp::FileMkdir {
                path: path.to_owned(),
                jail: jail.map(str::to_owned),
            },
        )
        .await
    }

    pub async fn file_remove(
        &self,
        rid: &str,
        actor: &str,
        paths: Vec<String>,
        permanent: bool,
        jail: Option<&str>,
    ) -> Result<u32, ClientError> {
        let op = PrivOp::FileRemove {
            paths,
            permanent,
            jail: jail.map(str::to_owned),
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::Removed(n) => Ok(n),
            o => Err(o),
        })
        .await
    }

    pub async fn file_stage_download(
        &self,
        rid: &str,
        actor: &str,
        path: &str,
        jail: Option<&str>,
    ) -> Result<Staged, ClientError> {
        let op = PrivOp::FileStageDownload {
            path: path.to_owned(),
            jail: jail.map(str::to_owned),
        };
        self.expect(rid, Some(actor), op, |r| match r {
            PrivReply::Staged(s) => Ok(s),
            o => Err(o),
        })
        .await
    }

    pub async fn file_install_upload(
        &self,
        rid: &str,
        actor: &str,
        upload: InstallUpload<'_>,
        jail: Option<&str>,
    ) -> Result<(), ClientError> {
        let op = PrivOp::FileInstallUpload {
            staging: upload.staging.to_owned(),
            dir: upload.dir.to_owned(),
            name: upload.name.to_owned(),
            overwrite: upload.overwrite,
            jail: jail.map(str::to_owned),
        };
        self.done(rid, actor, op).await
    }

    pub async fn secret_seal(
        &self,
        rid: &str,
        actor: Option<&str>,
        context: &str,
        plaintext: Secret,
    ) -> Result<String, ClientError> {
        let op = PrivOp::SecretSeal {
            context: context.to_owned(),
            plaintext,
        };
        self.expect(rid, actor, op, |r| match r {
            PrivReply::Sealed { sealed } => Ok(sealed),
            o => Err(o),
        })
        .await
    }

    pub async fn secret_open(
        &self,
        rid: &str,
        actor: Option<&str>,
        context: &str,
        sealed: &str,
    ) -> Result<Secret, ClientError> {
        let op = PrivOp::SecretOpen {
            context: context.to_owned(),
            sealed: sealed.to_owned(),
        };
        self.expect(rid, actor, op, |r| match r {
            PrivReply::Opened { plaintext } => Ok(plaintext),
            o => Err(o),
        })
        .await
    }

    pub async fn web_local_only(
        &self,
        rid: &str,
        actor: &str,
        local_only: bool,
    ) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::WebLocalOnly { local_only })
            .await
    }

    pub async fn watchdog_status(&self, rid: &str) -> Result<WatchdogStatus, ClientError> {
        self.expect(rid, None, PrivOp::WatchdogStatus, |r| match r {
            PrivReply::WatchdogStatus(s) => Ok(*s),
            o => Err(o),
        })
        .await
    }

    pub async fn docker_status(&self, rid: &str) -> Result<DockerStatus, ClientError> {
        self.expect(rid, None, PrivOp::DockerStatus, |r| match r {
            PrivReply::DockerStatus(s) => Ok(s),
            o => Err(o),
        })
        .await
    }

    pub async fn docker_install(
        &self,
        rid: &str,
        actor: &str,
    ) -> Result<CommandOutcome, ClientError> {
        self.expect(rid, Some(actor), PrivOp::DockerInstall, |r| match r {
            PrivReply::Outcome(o) => Ok(o),
            o => Err(o),
        })
        .await
    }

    pub async fn docker_list_containers(
        &self,
        rid: &str,
    ) -> Result<Vec<DockerContainer>, ClientError> {
        self.expect(rid, None, PrivOp::DockerListContainers, |r| match r {
            PrivReply::DockerContainers(c) => Ok(c),
            o => Err(o),
        })
        .await
    }

    pub async fn docker_inspect(
        &self,
        rid: &str,
        id: &str,
    ) -> Result<DockerContainerInspect, ClientError> {
        self.expect(
            rid,
            None,
            PrivOp::DockerInspect { id: id.to_owned() },
            |r| match r {
                PrivReply::DockerInspect(i) => Ok(*i),
                o => Err(o),
            },
        )
        .await
    }

    pub async fn docker_container_action(
        &self,
        rid: &str,
        actor: &str,
        id: &str,
        action: DockerContainerAction,
    ) -> Result<(), ClientError> {
        self.done(
            rid,
            actor,
            PrivOp::DockerContainerAction {
                id: id.to_owned(),
                action,
            },
        )
        .await
    }

    pub async fn docker_logs(
        &self,
        rid: &str,
        id: &str,
        tail: u32,
    ) -> Result<Vec<DockerLogLine>, ClientError> {
        self.expect(
            rid,
            None,
            PrivOp::DockerLogs {
                id: id.to_owned(),
                tail,
            },
            |r| match r {
                PrivReply::DockerLogs(lines) => Ok(lines),
                o => Err(o),
            },
        )
        .await
    }

    pub async fn docker_logs_follow(
        &self,
        rid: &str,
        actor: Option<&str>,
        id: &str,
        tail: u32,
    ) -> Result<DockerLogSession, ClientError> {
        let handshake = async {
            let mut stream = UnixStream::connect(&*self.socket_path)
                .await
                .map_err(|source| ClientError::Connect {
                    path: self.socket_path.display().to_string(),
                    source,
                })?;
            let request = PrivRequest {
                id: rid.to_owned(),
                actor: actor.map(str::to_owned),
                op: PrivOp::DockerLogsFollow {
                    id: id.to_owned(),
                    tail,
                },
            };
            write_frame(&mut stream, &request).await?;
            let body = read_body(&mut stream).await?.ok_or_else(|| {
                ClientError::Protocol("connection closed without an answer".into())
            })?;
            let response: PrivResponse =
                frame::decode(&body).map_err(|e| ClientError::Protocol(e.to_string()))?;
            match response.result {
                PrivResult::Failure { error } => Err(ClientError::Remote(error)),
                PrivResult::Success {
                    reply: PrivReply::DockerLogStreamStarted { id },
                } => {
                    let (reader, _writer) = stream.into_split();
                    Ok(DockerLogSession {
                        id,
                        reader,
                        _writer,
                    })
                }
                PrivResult::Success { reply } => {
                    Err(ClientError::Protocol(format!("unexpected reply {reply:?}")))
                }
            }
        };
        tokio::time::timeout(self.timeout, handshake)
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    pub async fn docker_stats(&self, rid: &str) -> Result<Vec<DockerContainerStat>, ClientError> {
        self.expect(rid, None, PrivOp::DockerStats, |r| match r {
            PrivReply::DockerStats(s) => Ok(s),
            o => Err(o),
        })
        .await
    }

    pub async fn docker_exec(
        &self,
        rid: &str,
        actor: &str,
        req: DockerExecRequest,
    ) -> Result<DockerExecOutput, ClientError> {
        self.expect(rid, Some(actor), PrivOp::DockerExec(req), |r| match r {
            PrivReply::DockerExec(out) => Ok(out),
            o => Err(o),
        })
        .await
    }

    pub async fn docker_list_images(&self, rid: &str) -> Result<Vec<DockerImage>, ClientError> {
        self.expect(rid, None, PrivOp::DockerListImages, |r| match r {
            PrivReply::DockerImages(imgs) => Ok(imgs),
            o => Err(o),
        })
        .await
    }

    pub async fn docker_list_volumes(&self, rid: &str) -> Result<Vec<DockerVolume>, ClientError> {
        self.expect(rid, None, PrivOp::DockerListVolumes, |r| match r {
            PrivReply::DockerVolumes(vols) => Ok(vols),
            o => Err(o),
        })
        .await
    }

    pub async fn docker_list_networks(&self, rid: &str) -> Result<Vec<DockerNetwork>, ClientError> {
        self.expect(rid, None, PrivOp::DockerListNetworks, |r| match r {
            PrivReply::DockerNetworks(nets) => Ok(nets),
            o => Err(o),
        })
        .await
    }

    pub async fn deploy_ensure_dir(
        &self,
        rid: &str,
        actor: &str,
        service: &str,
    ) -> Result<(), ClientError> {
        self.done(
            rid,
            actor,
            PrivOp::DeployEnsureDir {
                service: service.to_owned(),
            },
        )
        .await
    }

    pub async fn deploy_build(
        &self,
        rid: &str,
        actor: &str,
        req: DeployBuildRequest,
    ) -> Result<DeployBuildSession, ClientError> {
        let handshake = async {
            let mut stream = UnixStream::connect(&*self.socket_path)
                .await
                .map_err(|source| ClientError::Connect {
                    path: self.socket_path.display().to_string(),
                    source,
                })?;
            let request = PrivRequest {
                id: rid.to_owned(),
                actor: Some(actor.to_owned()),
                op: PrivOp::DeployBuild(req),
            };
            write_frame(&mut stream, &request).await?;
            let body = read_body(&mut stream).await?.ok_or_else(|| {
                ClientError::Protocol("connection closed without an answer".into())
            })?;
            let response: PrivResponse =
                frame::decode(&body).map_err(|e| ClientError::Protocol(e.to_string()))?;
            match response.result {
                PrivResult::Failure { error } => Err(ClientError::Remote(error)),
                PrivResult::Success {
                    reply: PrivReply::DeployBuildStreamStarted { deploy_id },
                } => {
                    let (reader, _writer) = stream.into_split();
                    Ok(DeployBuildSession {
                        deploy_id,
                        reader,
                        _writer,
                    })
                }
                PrivResult::Success { reply } => {
                    Err(ClientError::Protocol(format!("unexpected reply {reply:?}")))
                }
            }
        };
        tokio::time::timeout(self.timeout, handshake)
            .await
            .map_err(|_| ClientError::Timeout)?
    }

    pub async fn deploy_pull_image(
        &self,
        rid: &str,
        actor: &str,
        req: DeployPullRequest,
    ) -> Result<(), ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(300))
            .done(rid, actor, PrivOp::DeployPullImage(req))
            .await
    }

    pub async fn deploy_start_and_check(
        &self,
        rid: &str,
        actor: &str,
        req: DeployStartRequest,
    ) -> Result<DeployStartResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(120))
            .expect(
                rid,
                Some(actor),
                PrivOp::DeployStartAndCheck(req),
                |r| match r {
                    PrivReply::DeployStarted(res) => Ok(res),
                    o => Err(o),
                },
            )
            .await
    }

    pub async fn deploy_stop_container(
        &self,
        rid: &str,
        actor: &str,
        container_id: &str,
    ) -> Result<(), ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(60))
            .done(
                rid,
                actor,
                PrivOp::DeployStopContainer {
                    container_id: container_id.to_owned(),
                },
            )
            .await
    }

    pub async fn deploy_image_exists(
        &self,
        rid: &str,
        image_ref: &str,
    ) -> Result<bool, ClientError> {
        self.expect(
            rid,
            None,
            PrivOp::DeployImageExists {
                image_ref: image_ref.to_owned(),
            },
            |r| match r {
                PrivReply::Bool(b) => Ok(b),
                o => Err(o),
            },
        )
        .await
    }

    pub async fn deploy_prune_images(
        &self,
        rid: &str,
        actor: &str,
        service: &str,
        image_refs: Vec<String>,
    ) -> Result<Vec<String>, ClientError> {
        // Removing several large images can take longer than the 10 s default.
        self.clone()
            .with_timeout(Duration::from_secs(120))
            .expect(
                rid,
                Some(actor),
                PrivOp::DeployPruneImages {
                    service: service.to_owned(),
                    image_refs,
                },
                |r| match r {
                    PrivReply::PrunedImages(v) => Ok(v),
                    o => Err(o),
                },
            )
            .await
    }

    pub async fn deploy_delete_service(
        &self,
        rid: &str,
        actor: &str,
        service: &str,
        container_ids: Vec<String>,
        image_refs: Vec<String>,
    ) -> Result<(), ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(90))
            .done(
                rid,
                actor,
                PrivOp::DeployDeleteService {
                    service: service.to_owned(),
                    container_ids,
                    image_refs,
                },
            )
            .await
    }

    pub async fn template_start(
        &self,
        rid: &str,
        actor: &str,
        req: TemplateStartRequest,
    ) -> Result<DeployStartResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(300))
            .expect(rid, Some(actor), PrivOp::TemplateStart(req), |r| match r {
                PrivReply::DeployStarted(v) => Ok(v),
                o => Err(o),
            })
            .await
    }

    pub async fn database_rotate(
        &self,
        rid: &str,
        actor: &str,
        req: DatabaseRotateRequest,
    ) -> Result<DatabaseRotateResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(60))
            .expect(rid, Some(actor), PrivOp::DatabaseRotate(req), |r| match r {
                PrivReply::DatabaseRotated(v) => Ok(v),
                o => Err(o),
            })
            .await
    }

    pub async fn database_inspect(
        &self,
        rid: &str,
        actor: &str,
        req: DatabaseInspectRequest,
    ) -> Result<DatabaseInspectResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(60))
            .expect(
                rid,
                Some(actor),
                PrivOp::DatabaseInspect(req),
                |r| match r {
                    PrivReply::DatabaseInspected(v) => Ok(*v),
                    o => Err(o),
                },
            )
            .await
    }

    pub async fn database_manage(
        &self,
        rid: &str,
        actor: &str,
        req: DatabaseManageRequest,
    ) -> Result<DatabaseManageResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(120))
            .expect(rid, Some(actor), PrivOp::DatabaseManage(req), |r| match r {
                PrivReply::DatabaseManaged(v) => Ok(v),
                o => Err(o),
            })
            .await
    }

    pub async fn database_pooler(
        &self,
        rid: &str,
        actor: &str,
        req: DatabasePoolerRequest,
    ) -> Result<DatabasePoolerResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(300))
            .expect(rid, Some(actor), PrivOp::DatabasePooler(req), |r| match r {
                PrivReply::DatabasePooler(v) => Ok(v),
                o => Err(o),
            })
            .await
    }

    pub async fn database_upgrade(
        &self,
        rid: &str,
        actor: &str,
        req: DatabaseUpgradeRequest,
    ) -> Result<DatabaseUpgradeResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(4 * 3600))
            .expect(
                rid,
                Some(actor),
                PrivOp::DatabaseUpgrade(Box::new(req)),
                |r| match r {
                    PrivReply::DatabaseUpgraded(v) => Ok(*v),
                    o => Err(o),
                },
            )
            .await
    }

    pub async fn compose_validate(
        &self,
        rid: &str,
        actor: &str,
        req: ComposeValidateRequest,
    ) -> Result<ComposeValidationResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(45))
            .expect(
                rid,
                Some(actor),
                PrivOp::ComposeValidate(req),
                |r| match r {
                    PrivReply::ComposeValidated(v) => Ok(v),
                    o => Err(o),
                },
            )
            .await
    }

    pub async fn compose_deploy(
        &self,
        rid: &str,
        actor: &str,
        req: ComposeDeployRequest,
    ) -> Result<ComposeDeployResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(330))
            .expect(rid, Some(actor), PrivOp::ComposeDeploy(req), |r| match r {
                PrivReply::ComposeDeployed(v) => Ok(v),
                o => Err(o),
            })
            .await
    }

    pub async fn compose_down(
        &self,
        rid: &str,
        actor: &str,
        stack: &str,
        remove_files: bool,
    ) -> Result<(), ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(120))
            .done(
                rid,
                actor,
                PrivOp::ComposeDown {
                    stack: stack.to_owned(),
                    remove_files,
                },
            )
            .await
    }

    pub async fn compose_remove_service(
        &self,
        rid: &str,
        actor: &str,
        stack: &str,
        service: &str,
    ) -> Result<(), ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(60))
            .done(
                rid,
                actor,
                PrivOp::ComposeRemoveService {
                    stack: stack.to_owned(),
                    service: service.to_owned(),
                },
            )
            .await
    }

    pub async fn backup_local_info(
        &self,
        rid: &str,
        actor: Option<&str>,
        check_paths: Vec<String>,
    ) -> Result<BackupLocalInfo, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(30))
            .expect(
                rid,
                actor,
                PrivOp::BackupLocalInfo { check_paths },
                |r| match r {
                    PrivReply::BackupLocalInfo(v) => Ok(*v),
                    o => Err(o),
                },
            )
            .await
    }

    pub async fn backup_run(
        &self,
        rid: &str,
        actor: Option<&str>,
        spec: BackupRunSpec,
    ) -> Result<BackupRunResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(31 * 60))
            .expect(rid, actor, PrivOp::BackupRun(spec), |r| match r {
                PrivReply::BackupRun(v) => Ok(*v),
                o => Err(o),
            })
            .await
    }

    pub async fn backup_snapshots(
        &self,
        rid: &str,
        actor: Option<&str>,
        req: BackupSnapshotsRequest,
    ) -> Result<Vec<ResticSnapshotInfo>, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(120))
            .expect(rid, actor, PrivOp::BackupSnapshots(req), |r| match r {
                PrivReply::BackupSnapshots(v) => Ok(v),
                o => Err(o),
            })
            .await
    }

    pub async fn backup_forget(
        &self,
        rid: &str,
        actor: Option<&str>,
        req: BackupForgetRequest,
    ) -> Result<BackupForgetResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(31 * 60))
            .expect(rid, actor, PrivOp::BackupForget(req), |r| match r {
                PrivReply::BackupForget(v) => Ok(*v),
                o => Err(o),
            })
            .await
    }

    pub async fn backup_check(
        &self,
        rid: &str,
        actor: Option<&str>,
        req: BackupCheckRequest,
    ) -> Result<BackupCheckResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(31 * 60))
            .expect(rid, actor, PrivOp::BackupCheck(req), |r| match r {
                PrivReply::BackupCheck(v) => Ok(*v),
                o => Err(o),
            })
            .await
    }

    pub async fn backup_test_destination(
        &self,
        rid: &str,
        actor: Option<&str>,
        req: opsbox_proto::BackupDestinationTestRequest,
    ) -> Result<opsbox_proto::BackupDestinationTestResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(180))
            .expect(
                rid,
                actor,
                PrivOp::BackupTestDestination(req),
                |r| match r {
                    PrivReply::BackupTestDestination(v) => Ok(v),
                    o => Err(o),
                },
            )
            .await
    }

    pub async fn backup_generate_sftp_key(
        &self,
        rid: &str,
        actor: Option<&str>,
        req: opsbox_proto::BackupGenerateSftpKeyRequest,
    ) -> Result<opsbox_proto::BackupSftpKeyResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(30))
            .expect(
                rid,
                actor,
                PrivOp::BackupGenerateSftpKey(req),
                |r| match r {
                    PrivReply::BackupSftpKey(v) => Ok(v),
                    o => Err(o),
                },
            )
            .await
    }

    pub async fn backup_recovery_key(
        &self,
        rid: &str,
        actor: Option<&str>,
    ) -> Result<opsbox_proto::BackupRecoveryKeyResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(30))
            .expect(rid, actor, PrivOp::BackupRecoveryKey, |r| match r {
                PrivReply::BackupRecoveryKey(v) => Ok(v),
                o => Err(o),
            })
            .await
    }

    pub async fn backup_restore(
        &self,
        rid: &str,
        actor: Option<&str>,
        req: opsbox_proto::BackupRestoreRequest,
    ) -> Result<opsbox_proto::BackupRestoreResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(31 * 60))
            .expect(rid, actor, PrivOp::BackupRestore(req), |r| match r {
                PrivReply::BackupRestore(v) => Ok(*v),
                o => Err(o),
            })
            .await
    }

    pub async fn backup_restore_test(
        &self,
        rid: &str,
        actor: Option<&str>,
        req: opsbox_proto::BackupRestoreTestRequest,
    ) -> Result<opsbox_proto::BackupRestoreTestResult, ClientError> {
        self.clone()
            .with_timeout(Duration::from_secs(31 * 60))
            .expect(rid, actor, PrivOp::BackupRestoreTest(req), |r| match r {
                PrivReply::BackupRestoreTest(v) => Ok(*v),
                o => Err(o),
            })
            .await
    }

    pub async fn uninstall_plan(
        &self,
        rid: &str,
        purge: bool,
        domains: Vec<String>,
    ) -> Result<opsbox_proto::UninstallPlan, ClientError> {
        self.expect(
            rid,
            None,
            PrivOp::UninstallPlan { purge, domains },
            |r| match r {
                PrivReply::UninstallPlan(p) => Ok(*p),
                o => Err(o),
            },
        )
        .await
    }

    pub async fn uninstall(
        &self,
        rid: &str,
        actor: &str,
        purge: bool,
    ) -> Result<(), ClientError> {
        self.done(rid, actor, PrivOp::Uninstall { purge }).await
    }
}
