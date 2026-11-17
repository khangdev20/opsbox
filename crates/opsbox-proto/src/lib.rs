//! Shared types for opsbox.
//!
//! This crate has no I/O and no async runtime so the desktop app, `serve`, `priv`
//! and (later) the watchdog can all depend on it.

pub mod api;
pub mod frame;
pub mod metrics;
pub mod privilege;
pub mod role;
pub mod system;

pub use api::{ApiError, ApiErrorBody, ErrorCode, HostView};
pub use metrics::{CpuTimes, DiskIo, DiskUsage, MemInfo, NetIf, RawProc, RawSample, SystemFacts};
pub use privilege::{
    HostInfo, PrivError, PrivErrorCode, PrivOp, PrivReply, PrivRequest, PrivResponse, PrivResult,
    Secret,
};
pub use role::Role;
pub use system::{
    CommandOutcome, FirewallChange, FirewallRule, FirewallState, JobInfo, JournalEntry, NewRule,
    PkgSelect, PkgState, PkgUpdate, UnitActionKind, UnitInfo,
};
