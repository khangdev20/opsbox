//! Types for the Cleanup page (docs/BACKEND_SPEC.md 7.11).

use serde::{Deserialize, Serialize};

/// One thing that can be cleaned, with an estimate of what it frees.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupItem {
    /// Stable name: `journal`, `pkg-cache`, `slots`, `tmp`, `coredumps`, `docker-images`, ...
    pub id: String,
    pub title: String,
    /// What exactly would go, in words.
    pub desc: String,
    pub bytes: u64,
    /// Part of the set the weekly automatic clean may run.
    pub safe: bool,
    /// Can lose data someone may still want: never selected by default, never automatic.
    pub warn: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupScan {
    pub items: Vec<CleanupItem>,
    /// Size of the operating system's own files (an estimate; 0 when it could not be taken).
    pub system_bytes: u64,
    /// The weekly automatic clean is switched on.
    pub auto: bool,
}

/// What one scanner freed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupFreed {
    pub id: String,
    pub bytes: u64,
    /// Set when this scanner could not finish; the others still ran.
    pub error: Option<String>,
}
