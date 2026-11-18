//! Per-user read and snooze state for `/api/v1/attention` items (migration 0012).

use std::collections::HashMap;

use rusqlite::{TransactionBehavior, params};

use crate::{Store, StoreError};

/// Stored read/snooze markers for one `(user_id, item_id)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionStateRecord {
    pub item_id: String,
    pub read_since: Option<i64>,
    pub read_severity: Option<i64>,
    pub read_at: Option<i64>,
    pub snooze_until: Option<i64>,
    pub snooze_severity: Option<i64>,
    pub updated_at: i64,
}

impl Store {
    /// Load every stored read/snooze record for `user_id`, keyed by `item_id`.
    pub fn list_attention_state(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, AttentionStateRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT item_id, read_since, read_severity, read_at, snooze_until, snooze_severity, updated_at
             FROM attention_state
             WHERE user_id = ?1",
        )?;
        let rows = stmt.query_map(params![user_id], |r| {
            Ok(AttentionStateRecord {
                item_id: r.get(0)?,
                read_since: r.get(1)?,
                read_severity: r.get(2)?,
                read_at: r.get(3)?,
                snooze_until: r.get(4)?,
                snooze_severity: r.get(5)?,
                updated_at: r.get(6)?,
            })
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let rec = row?;
            out.insert(rec.item_id.clone(), rec);
        }
        Ok(out)
    }

    /// Record that `user_id` has read `item_id` at `(since, severity_rank)`.
    pub fn mark_attention_read(
        &self,
        user_id: &str,
        item_id: &str,
        since: i64,
        severity_rank: i64,
        now: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO attention_state (user_id, item_id, read_since, read_severity, read_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?5)
             ON CONFLICT (user_id, item_id) DO UPDATE SET
                 read_since = excluded.read_since,
                 read_severity = excluded.read_severity,
                 read_at = excluded.read_at,
                 updated_at = excluded.updated_at",
            params![user_id, item_id, since, severity_rank, now],
        )?;
        Ok(())
    }

    /// Mark every `(item_id, since, severity_rank)` in `items` as read for `user_id`.
    pub fn mark_all_attention_read(
        &self,
        user_id: &str,
        items: &[(String, i64, i64)],
        now: i64,
    ) -> Result<(), StoreError> {
        if items.is_empty() {
            return Ok(());
        }
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO attention_state (user_id, item_id, read_since, read_severity, read_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                 ON CONFLICT (user_id, item_id) DO UPDATE SET
                     read_since = excluded.read_since,
                     read_severity = excluded.read_severity,
                     read_at = excluded.read_at,
                     updated_at = excluded.updated_at",
            )?;
            for (item_id, since, severity_rank) in items {
                stmt.execute(params![user_id, item_id, since, severity_rank, now])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Snooze `item_id` for `user_id` until `until` (unix seconds) at `severity_rank`.
    pub fn snooze_attention(
        &self,
        user_id: &str,
        item_id: &str,
        until: i64,
        severity_rank: i64,
        now: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO attention_state (user_id, item_id, snooze_until, snooze_severity, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (user_id, item_id) DO UPDATE SET
                 snooze_until = excluded.snooze_until,
                 snooze_severity = excluded.snooze_severity,
                 updated_at = excluded.updated_at",
            params![user_id, item_id, until, severity_rank, now],
        )?;
        Ok(())
    }
}
