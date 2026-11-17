//! Numbered migrations, applied in order at startup, each inside a transaction.
//!
//! To add one: create `migrations/NNNN_name.sql` and append it to [`MIGRATIONS`].
//! Never edit a migration that has shipped.

use rusqlite::{Connection, TransactionBehavior};

struct Migration {
    version: u32,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "initial",
    sql: include_str!("../migrations/0001_initial.sql"),
}];

/// Version of the newest migration this binary knows.
pub const LATEST_VERSION: u32 = MIGRATIONS[MIGRATIONS.len() - 1].version;

#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    #[error(
        "the database is at schema version {found} but this opsbox only knows up to {latest}; \
         refusing to run an older binary against a newer database"
    )]
    DatabaseNewer { found: u32, latest: u32 },
    #[error("migration {version} ({name}) failed: {source}")]
    Failed {
        version: u32,
        name: &'static str,
        source: rusqlite::Error,
    },
    #[error("migration bookkeeping failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub(crate) fn current_version(conn: &Connection) -> rusqlite::Result<u32> {
    let table_exists: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations')",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(0);
    }
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )
}

pub(crate) fn run(conn: &mut Connection) -> Result<(), MigrationError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version    INTEGER PRIMARY KEY NOT NULL,
             name       TEXT NOT NULL,
             applied_at TEXT NOT NULL
         ) STRICT;",
    )?;
    let current = current_version(conn)?;
    if current > LATEST_VERSION {
        return Err(MigrationError::DatabaseNewer {
            found: current,
            latest: LATEST_VERSION,
        });
    }
    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        let failed = |source| MigrationError::Failed {
            version: m.version,
            name: m.name,
            source,
        };
        // IMMEDIATE takes the write lock up front so two processes starting at once
        // cannot both apply the same migration.
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(failed)?;
        // Re-check under the lock.
        if current_version(&tx).map_err(failed)? >= m.version {
            continue;
        }
        tx.execute_batch(m.sql).map_err(failed)?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            (m.version, m.name, crate::now_rfc3339()),
        )
        .map_err(failed)?;
        tx.commit().map_err(failed)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_contiguous_from_one() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                m.version as usize,
                i + 1,
                "migration {} is out of order",
                m.name
            );
        }
    }
}
