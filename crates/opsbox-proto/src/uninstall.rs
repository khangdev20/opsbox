//! Uninstall plan: what stays, what is removed, and which domains are affected.

use serde::{Deserialize, Serialize};

/// Preview of an uninstall (keep-data or purge).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallPlan {
    /// When true, `/var/lib/opsbox` and `/etc/opsbox` are removed after a typed confirmation.
    pub purge: bool,
    /// Human-readable list of things that will be removed.
    pub removed: Vec<String>,
    /// Human-readable list of things that stay (apps, data dirs, etc.).
    pub kept: Vec<String>,
    /// Proxy domains that stop answering on 80/443 once opsbox is gone.
    pub domains: Vec<String>,
    /// An adopted (moved-behind-opsbox) web server that will be restored first, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopted: Option<UninstallAdopted>,
    /// Managed containers that will be switched to Docker restart policy `unless-stopped`.
    pub containers: Vec<String>,
    /// Extra warnings for the panel (proxy domains, purge, etc.).
    pub warnings: Vec<String>,
}

/// An existing web server that 16c moved behind opsbox; uninstall restores its public ports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallAdopted {
    pub holder: String,
    pub domains: Vec<String>,
    pub will_restore: bool,
}
