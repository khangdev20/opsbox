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
