//! The append-only, hash-chained audit log (docs/BACKEND_SPEC.md section 7.14).
//!
//! Each row stores `hash = sha256(prev_hash || canonical_json(fields))`, where
//! `prev_hash` is the previous row's hash (or [`GENESIS_HASH`] for the first row).
//! Editing or removing any row except the newest breaks every hash after it, which
//! [`Store::verify_audit`] reports. Triggers in the schema also refuse UPDATE and
//! DELETE, so only someone who drops the triggers can tamper, and the chain still
//! shows it. Removing rows from the *end* is not detectable without an external
//! anchor; exporting the head hash periodically is the planned answer.

use rusqlite::TransactionBehavior;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{Store, StoreError, now_rfc3339};

/// `prev_hash` of the first entry.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    User,
    System,
    Token,
    Device,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
            Self::Token => "token",
            Self::Device => "device",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditResult {
    Ok,
    Denied,
    Failed,
}

impl AuditResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

/// What a caller supplies; the store adds id, timestamp and hashes.
#[derive(Debug, Clone)]
pub struct NewAudit {
    pub actor: String,
    pub actor_kind: ActorKind,
    pub ip: Option<String>,
    /// Dotted verb, e.g. `system.start`, `auth.login`.
    pub action: String,
    pub target: Option<String>,
    pub result: AuditResult,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub seq: i64,
    pub id: String,
    pub ts: String,
    pub actor: String,
    pub actor_kind: String,
    pub ip: Option<String>,
    pub action: String,
    pub target: Option<String>,
    pub result: String,
    /// The exact JSON text that was hashed.
    pub detail: String,
    pub prev_hash: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditReport {
    pub entries: u64,
    /// Hash of the newest entry ([`GENESIS_HASH`] when the log is empty).
    pub head: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("audit chain broken at entry {seq}: {reason}")]
    Broken { seq: i64, reason: &'static str },
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl From<rusqlite::Error> for AuditError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Store(e.into())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The hash of an entry given the hash before it. The fields are serialized as a
/// JSON array, which is unambiguous (strings are escaped) and order-fixed.
fn compute_hash(prev_hash: &str, e: &AuditEntry) -> String {
    let canonical = serde_json::to_vec(&(
        &e.id,
        &e.ts,
        &e.actor,
        &e.actor_kind,
        &e.ip,
        &e.action,
        &e.target,
        &e.result,
        &e.detail,
    ))
    .expect("serializing strings to JSON cannot fail");
    let mut h = Sha256::new();
    h.update(prev_hash.as_bytes());
    h.update(&canonical);
    hex(&h.finalize())
}

const COLUMNS: &str =
    "seq, id, ts, actor, actor_kind, ip, action, target, result, detail, prev_hash, hash";

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
    Ok(AuditEntry {
        seq: row.get(0)?,
        id: row.get(1)?,
        ts: row.get(2)?,
        actor: row.get(3)?,
        actor_kind: row.get(4)?,
        ip: row.get(5)?,
        action: row.get(6)?,
        target: row.get(7)?,
        result: row.get(8)?,
        detail: row.get(9)?,
        prev_hash: row.get(10)?,
        hash: row.get(11)?,
    })
}

impl Store {
    /// Append one entry to the chain and return it.
    pub fn append_audit(&self, new: NewAudit) -> Result<AuditEntry, StoreError> {
        let mut conn = self.lock();
        // IMMEDIATE: reading the previous hash and inserting must be one atomic
        // step, or two writers could chain from the same parent.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prev_hash: String = tx
            .query_row(
                "SELECT hash FROM audit ORDER BY seq DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(GENESIS_HASH.to_owned()),
                other => Err(other),
            })?;
        let mut entry = AuditEntry {
            seq: 0,
            id: ulid::Ulid::generate().to_string(),
            ts: now_rfc3339(),
            actor: new.actor,
            actor_kind: new.actor_kind.as_str().to_owned(),
            ip: new.ip,
            action: new.action,
            target: new.target,
            result: new.result.as_str().to_owned(),
            detail: serde_json::to_string(&new.detail)?,
            prev_hash,
            hash: String::new(),
        };
        entry.hash = compute_hash(&entry.prev_hash, &entry);
        tx.execute(
            "INSERT INTO audit (id, ts, actor, actor_kind, ip, action, target, result, detail, prev_hash, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            (
                &entry.id,
                &entry.ts,
                &entry.actor,
                &entry.actor_kind,
                &entry.ip,
                &entry.action,
                &entry.target,
                &entry.result,
                &entry.detail,
                &entry.prev_hash,
                &entry.hash,
            ),
        )?;
        entry.seq = tx.last_insert_rowid();
        tx.commit()?;
        Ok(entry)
    }

    /// The newest `limit` entries, newest first.
    pub fn recent_audit(&self, limit: u32) -> Result<Vec<AuditEntry>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM audit ORDER BY seq DESC LIMIT ?1"
        ))?;
        let rows = stmt.query_map([limit], row_to_entry)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Walk the whole chain from the first entry and check every link and hash.
    pub fn verify_audit(&self) -> Result<AuditReport, AuditError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM audit ORDER BY seq ASC"))?;
        let mut rows = stmt.query([])?;
        let mut expected_prev = GENESIS_HASH.to_owned();
        let mut entries = 0u64;
        while let Some(row) = rows.next()? {
            let e = row_to_entry(row)?;
            if e.prev_hash != expected_prev {
                return Err(AuditError::Broken {
                    seq: e.seq,
                    reason: "previous hash does not match the preceding entry (entry removed or reordered)",
                });
            }
            if compute_hash(&e.prev_hash, &e) != e.hash {
                return Err(AuditError::Broken {
                    seq: e.seq,
                    reason: "entry contents do not match its hash (entry modified)",
                });
            }
            expected_prev = e.hash;
            entries += 1;
        }
        Ok(AuditReport {
            entries,
            head: expected_prev,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    fn entry(action: &str) -> NewAudit {
        NewAudit {
            actor: "admin".into(),
            actor_kind: ActorKind::User,
            ip: Some("203.0.113.5".into()),
            action: action.into(),
            target: Some("web".into()),
            result: AuditResult::Ok,
            detail: json!({"note": "unicode ✓ and \"quotes\""}),
        }
    }

    /// A file-backed store plus a second raw connection for tampering with it.
    fn file_store_with(n: usize) -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let store = Store::open(&path).unwrap();
        for i in 0..n {
            store.append_audit(entry(&format!("test.{i}"))).unwrap();
        }
        let raw = Connection::open(&path).unwrap();
        (dir, store, raw)
    }

    #[test]
    fn empty_log_verifies() {
        let store = Store::open_in_memory().unwrap();
        let report = store.verify_audit().unwrap();
        assert_eq!(report.entries, 0);
        assert_eq!(report.head, GENESIS_HASH);
    }

    #[test]
    fn chain_links_and_verifies() {
        let store = Store::open_in_memory().unwrap();
        let a = store.append_audit(entry("a")).unwrap();
        let b = store.append_audit(entry("b")).unwrap();
        let c = store.append_audit(entry("c")).unwrap();
        assert_eq!(a.prev_hash, GENESIS_HASH);
        assert_eq!(b.prev_hash, a.hash);
        assert_eq!(c.prev_hash, b.hash);
        assert_ne!(a.hash, b.hash);
        let report = store.verify_audit().unwrap();
        assert_eq!(report.entries, 3);
        assert_eq!(report.head, c.hash);
        let recent = store.recent_audit(2).unwrap();
        assert_eq!(
            recent.iter().map(|e| e.action.as_str()).collect::<Vec<_>>(),
            ["c", "b"]
        );
    }

    #[test]
    fn schema_refuses_update_and_delete() {
        let (_dir, store, raw) = file_store_with(2);
        let upd = raw.execute("UPDATE audit SET actor = 'mallory' WHERE seq = 1", []);
        assert!(upd.unwrap_err().to_string().contains("append-only"));
        let del = raw.execute("DELETE FROM audit WHERE seq = 1", []);
        assert!(del.unwrap_err().to_string().contains("append-only"));
        assert_eq!(store.verify_audit().unwrap().entries, 2);
    }

    #[test]
    fn modified_entry_is_detected() {
        let (_dir, store, raw) = file_store_with(4);
        raw.execute_batch("DROP TRIGGER audit_no_update").unwrap();
        raw.execute("UPDATE audit SET actor = 'mallory' WHERE seq = 2", [])
            .unwrap();
        match store.verify_audit() {
            Err(AuditError::Broken { seq, .. }) => assert_eq!(seq, 2),
            other => panic!("expected a broken chain at 2, got {other:?}"),
        }
    }

    #[test]
    fn removed_entry_is_detected() {
        let (_dir, store, raw) = file_store_with(4);
        raw.execute_batch("DROP TRIGGER audit_no_delete").unwrap();
        raw.execute("DELETE FROM audit WHERE seq = 2", []).unwrap();
        match store.verify_audit() {
            Err(AuditError::Broken { seq, .. }) => assert_eq!(seq, 3),
            other => panic!("expected a broken chain at 3, got {other:?}"),
        }
    }

    #[test]
    fn removing_the_first_entry_is_detected() {
        let (_dir, store, raw) = file_store_with(3);
        raw.execute_batch("DROP TRIGGER audit_no_delete").unwrap();
        raw.execute("DELETE FROM audit WHERE seq = 1", []).unwrap();
        assert!(matches!(
            store.verify_audit(),
            Err(AuditError::Broken { seq: 2, .. })
        ));
    }

    #[test]
    fn recomputing_a_tampered_hash_still_breaks_the_next_link() {
        // An attacker who also fixes up the edited row's own hash is still caught,
        // because the following row embeds the old hash as its prev_hash.
        let (_dir, store, raw) = file_store_with(3);
        raw.execute_batch("DROP TRIGGER audit_no_update").unwrap();
        raw.execute(
            "UPDATE audit SET result = 'denied', hash = ?1 WHERE seq = 1",
            ["f".repeat(64)],
        )
        .unwrap();
        assert!(matches!(
            store.verify_audit(),
            Err(AuditError::Broken { seq: 1, .. })
        ));
    }
}
