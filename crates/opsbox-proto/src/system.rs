//! Types for the System page: packages, services, journal, jobs, firewall
//! (docs/BACKEND_SPEC.md 7.8). All of it is performed by `priv`.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- packages

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkgUpdate {
    pub name: String,
    pub current: String,
    pub available: String,
    pub security: bool,
    /// A kernel image: installing it means a reboot is needed.
    pub kernel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PkgState {
    /// `apt`, `dnf`, or empty when no supported package manager was found.
    pub manager: String,
    pub updates: Vec<PkgUpdate>,
    pub reboot_required: bool,
    /// A reboot is scheduled for this unix time (see `Reboot`), if one is.
    pub reboot_at: Option<i64>,
    /// Whether security updates install themselves; `None` if it cannot be told.
    pub auto_security: Option<bool>,
    pub running_kernel: String,
    /// Unix seconds of the package index, when known.
    pub index_time: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PkgSelect {
    Security,
    All,
}

/// What a long-running command printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub ok: bool,
    /// The last lines of combined output.
    pub tail: Vec<String>,
    /// How many packages were installed, for `PkgInstall`.
    pub count: u32,
}

// ---------------------------------------------------------------- services

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitInfo {
    pub name: String,
    /// `active`, `inactive`, `failed`, `activating`, ...
    pub state: String,
    /// `enabled`, `disabled`, `static`, ... (from the unit file state).
    pub boot: String,
    pub description: String,
    pub mem_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitActionKind {
    Start,
    Stop,
    Restart,
    Enable,
    Disable,
}

impl UnitActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Enable => "enable",
            Self::Disable => "disable",
        }
    }
}

// ----------------------------------------------------------------- journal

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Unix milliseconds.
    pub t: i64,
    /// Unit or program, with pid when known: `sshd[9121]`.
    pub proc: String,
    /// 0 (emerg) to 7 (debug).
    pub prio: u8,
    pub msg: String,
    pub cursor: String,
}

// -------------------------------------------------------------------- jobs

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInfo {
    pub id: String,
    pub cron: String,
    pub name: String,
    pub command: String,
    /// Unix seconds of the last run, `None` if it never ran.
    pub last_run: Option<i64>,
    pub last_ok: bool,
    pub next_run: Option<i64>,
}

// --------------------------------------------------------------- firewall

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallRule {
    pub id: String,
    pub port: u16,
    pub proto: String,
    /// `anywhere` or an address / CIDR.
    pub src: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewRule {
    pub port: u16,
    pub proto: String,
    pub src: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallState {
    /// `ufw`, `firewalld` or `nftables`.
    pub backend: String,
    pub rules: Vec<FirewallRule>,
    /// Unix seconds when emergency SSH closes, if it is open.
    pub ssh_open_until: Option<i64>,
    /// A change is waiting for confirmation and will be undone at this time.
    pub revert_at: Option<i64>,
    /// Ports opsbox refuses to cut off without confirmation (SSH and the panel).
    pub protected_ports: Vec<u16>,
}

/// Result of a firewall change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirewallChange {
    /// If set, the change will be undone at this unix time unless confirmed.
    pub revert_at: Option<i64>,
}
