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

/// Filters for listing or exporting the audit log. Empty fields mean "no filter".
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    /// Exact actor string (e.g. `admin` or `Ada (admin)`).
    pub actor: Option<String>,
    /// `action` must start with this prefix (e.g. `auth.`).
    pub action_prefix: Option<String>,
    /// Exact result: `ok`, `denied`, or `failed`.
    pub result: Option<String>,
    /// Inclusive lower bound on `ts` (RFC 3339).
    pub from: Option<String>,
    /// Inclusive upper bound on `ts` (RFC 3339).
    pub to: Option<String>,
    /// When set (list only): return rows with `seq` strictly less than this (newer-first cursor).
    pub before_seq: Option<i64>,
    /// Page size for [`Store::list_audit`]. Ignored by the export walk.
    pub limit: u32,
}

/// One page of audit rows, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPage {
    pub entries: Vec<AuditEntry>,
    /// Pass as `before_seq` for the next page. `None` when this was the last page.
    pub next_cursor: Option<i64>,
}

/// Result of verifying every link in a contiguous `seq` range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRangeReport {
    pub first_hash: String,
    pub last_hash: String,
    pub entries: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error("audit chain broken at entry {seq}: {reason}")]
    Broken { seq: i64, reason: &'static str },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Default page size for `GET /api/v1/audit` (and the Security page preview keeps 12).
pub const DEFAULT_AUDIT_PAGE: u32 = 50;
/// Hard cap so a caller cannot ask for an unbounded page.
pub const MAX_AUDIT_PAGE: u32 = 200;

/// CSV column header for the audit export (prompt 16l).
pub const AUDIT_CSV_HEADER: &str = "seq,ts,actor,actor_kind,ip,action,target,result,detail,hash";

/// RFC 4180 cell escaping, with spreadsheet formula injection neutralised by
/// prefixing a leading `'` when the value starts with `=`, `+`, `-`, or `@`.
pub fn csv_cell(raw: &str) -> String {
    let neutralized = match raw.chars().next() {
        Some('=' | '+' | '-' | '@') => format!("'{raw}"),
        _ => raw.to_owned(),
    };
    if neutralized.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", neutralized.replace('"', "\"\""))
    } else {
        neutralized
    }
}

/// One CSV data row for an audit entry (no trailing newline).
pub fn audit_csv_row(e: &AuditEntry) -> String {
    [
        csv_cell(&e.seq.to_string()),
        csv_cell(&e.ts),
        csv_cell(&e.actor),
        csv_cell(&e.actor_kind),
        csv_cell(e.ip.as_deref().unwrap_or("")),
        csv_cell(&e.action),
        csv_cell(e.target.as_deref().unwrap_or("")),
        csv_cell(&e.result),
        csv_cell(&e.detail),
        csv_cell(&e.hash),
    ]
    .join(",")
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

pub(crate) const COLUMNS: &str =
    "seq, id, ts, actor, actor_kind, ip, action, target, result, detail, prev_hash, hash";

pub(crate) fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditEntry> {
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
        self.list_audit(&AuditFilter {
            limit,
            ..AuditFilter::default()
        })
        .map(|p| p.entries)
    }

    /// Newest-first page with optional filters and a `before_seq` cursor.
    pub fn list_audit(&self, filter: &AuditFilter) -> Result<AuditPage, StoreError> {
        let limit = filter.limit.clamp(1, MAX_AUDIT_PAGE);
        let conn = self.lock();
        let (sql, params) = filter_sql(filter, Order::Desc, Some(limit.saturating_add(1)));
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), row_to_entry)?;
        let mut entries: Vec<AuditEntry> = rows.collect::<Result<_, _>>()?;
        let next_cursor = if entries.len() > limit as usize {
            entries.truncate(limit as usize);
            entries.last().map(|e| e.seq)
        } else {
            None
        };
        Ok(AuditPage {
            entries,
            next_cursor,
        })
    }

    /// Walk matching rows oldest-first in small batches, without collecting the whole
    /// range. Used by the CSV export so a large export never sits in memory and the
    /// store lock is not held across slow I/O.
    pub fn for_each_audit<F>(&self, filter: &AuditFilter, mut f: F) -> Result<(), StoreError>
    where
        F: FnMut(&AuditEntry) -> Result<(), StoreError>,
    {
        const BATCH: u32 = 100;
        let mut after_seq: Option<i64> = None;
        loop {
            let batch = {
                let conn = self.lock();
                let (sql, params) = filter_sql_after(filter, after_seq, BATCH);
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(params), row_to_entry)?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            if batch.is_empty() {
                break;
            }
            for e in &batch {
                f(e)?;
                after_seq = Some(e.seq);
            }
            if batch.len() < BATCH as usize {
                break;
            }
        }
        Ok(())
    }

    /// Alert security inputs, independent of unrelated audit traffic.
    pub fn alert_security_events(&self, since: i64) -> Result<Vec<AuditEntry>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM audit WHERE action IN ('auth.login', 'system.ssh.emergency') AND unixepoch(ts) >= ?1 ORDER BY seq DESC"
        ))?;
        Ok(stmt
            .query_map([since], row_to_entry)?
            .collect::<Result<_, _>>()?)
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

    /// Verify every entry with `from_seq <= seq <= to_seq` (inclusive), chaining from
    /// the hash of the row just before `from_seq` (or genesis when `from_seq` is first).
    pub fn verify_audit_range(
        &self,
        from_seq: i64,
        to_seq: i64,
    ) -> Result<AuditRangeReport, AuditError> {
        if to_seq < from_seq {
            return Ok(AuditRangeReport {
                first_hash: GENESIS_HASH.to_owned(),
                last_hash: GENESIS_HASH.to_owned(),
                entries: 0,
            });
        }
        let conn = self.lock();
        let expected_prev: String = conn
            .query_row(
                "SELECT hash FROM audit WHERE seq < ?1 ORDER BY seq DESC LIMIT 1",
                [from_seq],
                |row| row.get(0),
            )
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(GENESIS_HASH.to_owned()),
                other => Err(other),
            })?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM audit WHERE seq >= ?1 AND seq <= ?2 ORDER BY seq ASC"
        ))?;
        let mut rows = stmt.query((from_seq, to_seq))?;
        let mut expected_prev = expected_prev;
        let mut entries = 0u64;
        let mut first_hash = None;
        let mut last_hash = expected_prev.clone();
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
            if first_hash.is_none() {
                first_hash = Some(e.hash.clone());
            }
            expected_prev = e.hash.clone();
            last_hash = e.hash;
            entries += 1;
        }
        Ok(AuditRangeReport {
            first_hash: first_hash.unwrap_or_else(|| GENESIS_HASH.to_owned()),
            last_hash,
            entries,
        })
    }
}

#[derive(Clone, Copy)]
#[allow(dead_code)]
enum Order {
    Asc,
    Desc,
}

/// Build `SELECT … FROM audit WHERE … ORDER BY … [LIMIT …]` and the bound parameters.
fn filter_sql(
    filter: &AuditFilter,
    order: Order,
    limit: Option<u32>,
) -> (String, Vec<rusqlite::types::Value>) {
    let mut sql = format!("SELECT {COLUMNS} FROM audit WHERE 1=1");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    push_filter_preds(filter, &mut sql, &mut params);
    if matches!(order, Order::Desc)
        && let Some(before) = filter.before_seq
    {
        params.push(before.into());
        sql.push_str(&format!(" AND seq < ?{}", params.len()));
    }
    sql.push_str(match order {
        Order::Asc => " ORDER BY seq ASC",
        Order::Desc => " ORDER BY seq DESC",
    });
    if let Some(n) = limit {
        params.push((n as i64).into());
        sql.push_str(&format!(" LIMIT ?{}", params.len()));
    }
    (sql, params)
}

/// Oldest-first page for the export walk: same filters, plus `seq > after`.
fn filter_sql_after(
    filter: &AuditFilter,
    after_seq: Option<i64>,
    limit: u32,
) -> (String, Vec<rusqlite::types::Value>) {
    let mut sql = format!("SELECT {COLUMNS} FROM audit WHERE 1=1");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    push_filter_preds(filter, &mut sql, &mut params);
    if let Some(after) = after_seq {
        params.push(after.into());
        sql.push_str(&format!(" AND seq > ?{}", params.len()));
    }
    params.push((limit as i64).into());
    sql.push_str(&format!(" ORDER BY seq ASC LIMIT ?{}", params.len()));
    (sql, params)
}

fn push_filter_preds(
    filter: &AuditFilter,
    sql: &mut String,
    params: &mut Vec<rusqlite::types::Value>,
) {
    if let Some(actor) = filter.actor.as_ref().filter(|s| !s.is_empty()) {
        params.push(actor.clone().into());
        sql.push_str(&format!(" AND actor = ?{}", params.len()));
    }
    if let Some(prefix) = filter.action_prefix.as_ref().filter(|s| !s.is_empty()) {
        // Escape LIKE metacharacters so a literal prefix cannot widen the match.
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        params.push(format!("{escaped}%").into());
        sql.push_str(&format!(" AND action LIKE ?{} ESCAPE '\\'", params.len()));
    }
    if let Some(result) = filter.result.as_ref().filter(|s| !s.is_empty()) {
        params.push(result.clone().into());
        sql.push_str(&format!(" AND result = ?{}", params.len()));
    }
    if let Some(from) = filter.from.as_ref().filter(|s| !s.is_empty()) {
        params.push(from.clone().into());
        sql.push_str(&format!(" AND ts >= ?{}", params.len()));
    }
    if let Some(to) = filter.to.as_ref().filter(|s| !s.is_empty()) {
        params.push(to.clone().into());
        sql.push_str(&format!(" AND ts <= ?{}", params.len()));
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

    #[test]
    fn csv_cells_escape_and_neutralise_formulas() {
        assert_eq!(csv_cell("plain"), "plain");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("=1+1"), "'=1+1");
        assert_eq!(csv_cell("+cmd"), "'+cmd");
        assert_eq!(csv_cell("-2"), "'-2");
        assert_eq!(csv_cell("@sum"), "'@sum");
        assert_eq!(csv_cell("=a,b"), "\"'=a,b\"");
        let e = AuditEntry {
            seq: 1,
            id: "id".into(),
            ts: "2026-01-01T00:00:00Z".into(),
            actor: "admin".into(),
            actor_kind: "user".into(),
            ip: Some("=1+1".into()),
            action: "auth.login".into(),
            target: None,
            result: "ok".into(),
            detail: r#"{"n":"x"}"#.into(),
            prev_hash: GENESIS_HASH.into(),
            hash: "abc".into(),
        };
        let row = audit_csv_row(&e);
        assert!(row.contains("'=1+1"), "{row}");
        assert!(
            row.starts_with("1,2026-01-01T00:00:00Z,admin,user,"),
            "{row}"
        );
    }

    #[test]
    fn filters_and_cursor_paging() {
        let store = Store::open_in_memory().unwrap();
        for i in 0..5 {
            store
                .append_audit(NewAudit {
                    actor: if i % 2 == 0 { "admin" } else { "other" }.into(),
                    actor_kind: ActorKind::User,
                    ip: Some("203.0.113.5".into()),
                    action: format!("auth.step{i}"),
                    target: None,
                    result: if i == 3 {
                        AuditResult::Denied
                    } else {
                        AuditResult::Ok
                    },
                    detail: json!({}),
                })
                .unwrap();
        }
        let page = store
            .list_audit(&AuditFilter {
                actor: Some("admin".into()),
                limit: 10,
                ..AuditFilter::default()
            })
            .unwrap();
        assert_eq!(page.entries.len(), 3);
        assert!(page.entries.iter().all(|e| e.actor == "admin"));

        let denied = store
            .list_audit(&AuditFilter {
                result: Some("denied".into()),
                limit: 10,
                ..AuditFilter::default()
            })
            .unwrap();
        assert_eq!(denied.entries.len(), 1);
        assert_eq!(denied.entries[0].action, "auth.step3");

        let prefix = store
            .list_audit(&AuditFilter {
                action_prefix: Some("auth.step".into()),
                limit: 2,
                ..AuditFilter::default()
            })
            .unwrap();
        assert_eq!(prefix.entries.len(), 2);
        assert!(prefix.next_cursor.is_some());
        let next = store
            .list_audit(&AuditFilter {
                action_prefix: Some("auth.step".into()),
                before_seq: prefix.next_cursor,
                limit: 10,
                ..AuditFilter::default()
            })
            .unwrap();
        assert_eq!(next.entries.len(), 3);
        assert!(next.next_cursor.is_none());

        let mut walked = 0u32;
        store
            .for_each_audit(&AuditFilter::default(), |_| {
                walked += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(walked, 5);

        let first = store.recent_audit(5).unwrap().last().unwrap().seq;
        let last = store.recent_audit(1).unwrap()[0].seq;
        let range = store.verify_audit_range(first, last).unwrap();
        assert_eq!(range.entries, 5);
        assert_eq!(range.last_hash, store.verify_audit().unwrap().head);
    }
}
