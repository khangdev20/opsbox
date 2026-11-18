//! restic orchestration, backup schedules, retention policies, and database dump specs
//! (docs/BACKEND_SPEC.md 7.12).

pub mod cron;
pub mod dumps;
pub mod restic;
pub mod validation;

pub use restic::derive_repo_password;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    Internal(String),
}
