//! Watchdog status and recovery types shared with `priv` and `serve` (docs/BACKEND_SPEC.md 7.16).

use serde::{Deserialize, Serialize};

/// One health check row displayed in Safe Mode and the panel's Recovery screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchdogCheck {
    pub name: String,
    /// `"ok"`, `"warn"`, `"fail"`, or `"off"`.
    pub status: String,
    pub state: String,
    pub note: String,
}

/// Snapshot published periodically by `opsbox-watchdog` and read by `opsbox-priv`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchdogStatus {
    pub running: bool,
    pub failed: bool,
    pub degraded: bool,
    pub updated_at: i64,
    pub current_version: String,
    pub previous_version: Option<String>,
    pub can_rollback: bool,
    pub update_message: Option<String>,
    pub checks: Vec<WatchdogCheck>,
    pub recent_events: Vec<String>,
    pub safe_mode_listen: Option<String>,
    pub ssh_open_until: Option<i64>,
}
