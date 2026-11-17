//! SQLite storage for opsbox.
//!
//! One [`Store`] owns one connection behind a mutex. Calls are short and
//! synchronous; async callers wrap them in `spawn_blocking`.

mod accounts;
mod audit;
mod deploy;
mod github;
mod invites;
mod metrics;
mod migrations;
mod settings;
mod tokens;

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub use accounts::{Session, SessionKind, User};
pub use audit::{
    ActorKind, AuditEntry, AuditError, AuditReport, AuditResult, GENESIS_HASH, NewAudit,
};
pub use deploy::{
    DeployLogRow, DeployRecord, DeployStatus, DeployTrigger, EnvironmentRecord, NewService,
    ProjectRecord, ServiceKind, ServiceRecord, ServiceState, ServiceVarRecord, SourceKind,
    UpdateService,
};
pub use github::{
    DeployKeyInfo, DeployKeyRecord, GithubAppRecord, NewDeployKey, NewGithubApp,
    NewWebhookDelivery, PendingManifestState, WebhookDelivery,
};
pub use invites::Invite;
pub use metrics::{Granularity, MetricRow};
pub use migrations::{LATEST_VERSION, MigrationError};
pub use tokens::{ApiToken, TokenScope};

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
        let cur = migrations::current_version(&conn)?;
        if cur > 0 && cur < migrations::LATEST_VERSION {
            let bak = path.with_file_name(format!(
                "{}.pre-v{}.bak",
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("opsbox.db"),
                migrations::LATEST_VERSION
            ));
            create_private_file(&bak)?;
            let mut dst = Connection::open_with_flags(
                &bak,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?;
            let backup = rusqlite::backup::Backup::new(&conn, &mut dst)?;
            backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
        }
        Self::init_conn(conn)
    }

    /// A private in-memory database, for tests and `selftest`.
    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init_conn(Connection::open_in_memory()?)
    }

    fn init_conn(mut conn: Connection) -> Result<Self, StoreError> {
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

/// Read the current schema version of an existing database without modifying it.
pub fn db_schema_version(path: &Path) -> Result<u32, StoreError> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    Ok(migrations::current_version(&conn)?)
}

/// Strictly read-only handle on the SQLite database for `opsbox-watchdog` (spec 7.16).
///
/// Never runs migrations, never writes, preserves `-shm` ownership when opened as root,
/// and falls back to `immutable=1` when no live WAL exists or the disk is full.
pub struct ReadOnlyStore {
    conn: Connection,
}

impl ReadOnlyStore {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let meta = std::fs::metadata(path)?;
        let wal_path = sidecar_path(path, "-wal");
        let shm_path = sidecar_path(path, "-shm");
        let shm_existed = shm_path.exists();
        let wal_non_empty = std::fs::metadata(&wal_path).is_ok_and(|m| m.len() > 0);

        let conn = if !shm_existed && !wal_non_empty {
            Connection::open_with_flags(
                sqlite_immutable_uri(path),
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?
        } else {
            match Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            ) {
                Ok(c) => {
                    if !shm_existed && shm_path.exists() {
                        preserve_owner_and_mode(&meta, &shm_path);
                    }
                    c
                }
                Err(_) => Connection::open_with_flags(
                    sqlite_immutable_uri(path),
                    OpenFlags::SQLITE_OPEN_READ_ONLY
                        | OpenFlags::SQLITE_OPEN_URI
                        | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?,
            }
        };

        conn.busy_timeout(std::time::Duration::from_secs(2))?;
        let _ = conn.pragma_update(None, "query_only", true);
        Ok(Self { conn })
    }

    pub fn schema_version(&self) -> Result<u32, StoreError> {
        Ok(migrations::current_version(&self.conn)?)
    }

    /// Run `PRAGMA quick_check(1)` to confirm database integrity without modifying anything.
    pub fn check_integrity(&self) -> Result<(), String> {
        let res: String = self
            .conn
            .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
            .map_err(|e| format!("database integrity check failed: {e}"))?;
        if res.eq_ignore_ascii_case("ok") {
            Ok(())
        } else {
            Err(format!("database integrity check failed: {res}"))
        }
    }

    pub fn user_by_name(&self, name: &str) -> Result<Option<User>, StoreError> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {} FROM users WHERE name = ?1", accounts::USER_COLS),
                [name],
                |r| accounts::user_row_at(r, 0),
            )
            .optional()?)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        raw.map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(Into::into)
    }
}

fn sidecar_path(base: &Path, suffix: &str) -> std::path::PathBuf {
    let mut s = base.as_os_str().to_os_string();
    s.push(suffix);
    std::path::PathBuf::from(s)
}

fn sqlite_immutable_uri(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut esc = String::with_capacity(raw.len() + 20);
    esc.push_str("file:");
    for ch in raw.chars() {
        match ch {
            '%' => esc.push_str("%25"),
            '?' => esc.push_str("%3F"),
            '#' => esc.push_str("%23"),
            _ => esc.push(ch),
        }
    }
    esc.push_str("?immutable=1");
    esc
}

fn preserve_owner_and_mode(source_meta: &std::fs::Metadata, target: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let _ = std::os::unix::fs::chown(target, Some(source_meta.uid()), Some(source_meta.gid()));
        let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = (source_meta, target);
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
