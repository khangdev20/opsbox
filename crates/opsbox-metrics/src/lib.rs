//! Machine metrics: counters to rates, the live ring, SQLite rollups and the
//! "needs attention" rules (docs/BACKEND_SPEC.md 7.3).

pub mod attention;
pub mod engine;
pub mod proc_kind;
pub mod rollup;
mod service;

pub use attention::{Attention, Severity};
pub use engine::{DiskView, Engine, Point, ProcView, Snapshot};
pub use proc_kind::{DockerIndexEntry, ProcContainer, ProcKind, ProcKindCache};
pub use service::Metrics;
