//! SQLite storage for opsbox.
//!
//! One [`Store`] owns one connection behind a mutex. Calls are short and
//! synchronous; async callers wrap them in `spawn_blocking`.

mod accounts;
mod audit;
mod migrations;
mod settings;

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, OpenFlags};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub use accounts::{Session, SessionKind, User};
pub use audit::{
    ActorKind, AuditEntry, AuditError, AuditReport, AuditResult, GENESIS_HASH, NewAudit,
};
pub use migrations::{LATEST_VERSION, MigrationError};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("cannot prepare database location: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Migration(#[from] MigrationError),
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Open (creating if needed) the database at `path`, apply pragmas and run
    /// every pending migration.
    ///
    /// The file is created with mode 0600 before SQLite touches it; the `-wal` and
    /// `-shm` files inherit that mode.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        create_private_file(path)?;
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Self::init(conn)
    }

    /// A private in-memory database, for tests and `selftest`.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL is a no-op ("memory") for in-memory databases, which is fine.
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get::<_, String>(0))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Must be set outside a transaction, so before migrations.
        conn.pragma_update(None, "foreign_keys", true)?;
        migrations::run(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic while holding the lock cannot leave the database inconsistent
        // (every write is a transaction), so recover instead of cascading.
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Cheap liveness probe used by `/readyz`.
    pub fn ping(&self) -> Result<(), StoreError> {
        self.lock().query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    /// Highest migration version applied to this database.
    pub fn schema_version(&self) -> Result<u32, StoreError> {
        Ok(migrations::current_version(&self.lock())?)
    }

    /// The active SQLite journal mode, lower case (`wal` for file databases).
    pub fn journal_mode(&self) -> Result<String, StoreError> {
        Ok(self
            .lock()
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))?)
    }
}

/// Current time as RFC 3339 UTC with millisecond precision.
pub fn now_rfc3339() -> String {
    let now = OffsetDateTime::now_utc();
    let millis = now.nanosecond() / 1_000_000 * 1_000_000;
    now.replace_nanosecond(millis)
        .unwrap_or(now)
        .format(&Rfc3339)
        .expect("RFC 3339 formatting of a valid timestamp cannot fail")
}

fn create_private_file(path: &Path) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path).map(drop)
}
