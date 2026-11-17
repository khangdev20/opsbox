//! Types for the Security page (docs/BACKEND_SPEC.md 7.14): what the privileged helper can
//! see about the machine that the checks need.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityScan {
    /// Files that hold secrets (`.env`, `*.pem`, `*.key`) and that anyone on the machine can
    /// read, inside the apps folder. At most 50.
    pub world_readable: Vec<String>,
    /// The key that seals stored secrets is readable by its owner only.
    pub master_key_private: bool,
    /// The SSH server reads `/etc/ssh/sshd_config.d`, so opsbox can change its settings
    /// without touching the main file.
    pub sshd_dropins: bool,
}

/// What the Settings page shows about the machine: the time zone and the swap in use.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSettings {
    pub timezone: String,
    /// Every zone the system knows, when it can be listed (empty otherwise).
    pub timezones: Vec<String>,
    /// Swap that is switched on, in megabytes (0 when none).
    pub swap_mb: u64,
}
