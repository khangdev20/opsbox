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
    /// Facts about the machine. Answered with [`PrivReply::HostInfo`].
    HostInfo,
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
    /// Clean the named things, in a fixed order. Unknown or unavailable ones are reported, not run.
    CleanupRun {
        ids: Vec<String>,
    },
    /// Switch the weekly automatic clean (a systemd timer) on or off.
    CleanupAutoSet {
        on: bool,
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
}

impl PrivOp {
    /// Stable name for logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::HostInfo => "host_info",
            Self::TotpNewSecret { .. } => "totp_new_secret",
            Self::TotpVerify { .. } => "totp_verify",
            Self::MetricsSample { .. } => "metrics_sample",
            Self::SystemFacts => "system_facts",
            Self::PkgList => "pkg_list",
            Self::PkgInstall { .. } => "pkg_install",
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
            Self::CleanupRun { .. } => "cleanup_run",
            Self::CleanupAutoSet { .. } => "cleanup_auto_set",
            Self::UserList => "user_list",
            Self::PtyOpen { .. } => "pty_open",
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
    Pong {
        /// Version of the `priv` process, so `status` can spot a stale helper.
        version: String,
    },
    HostInfo(HostInfo),
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
    Cleanup(crate::cleanup::CleanupScan),
    CleanupFreed(Vec<crate::cleanup::CleanupFreed>),
    /// A shell is running; the connection now carries the PTY stream.
    PtyStarted {
        user: String,
        shell: String,
    },
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
