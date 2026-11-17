//! Key/value settings stored as JSON.

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::{Store, StoreError, now_rfc3339};

impl Store {
    pub fn get_setting(&self, key: &str) -> Result<Option<Value>, StoreError> {
        let raw: Option<String> = self
            .lock()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        raw.map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(Into::into)
    }

    pub fn set_setting(&self, key: &str, value: &Value) -> Result<(), StoreError> {
        let now = now_rfc3339();
        self.lock().execute(
            "INSERT INTO settings (key, value, created_at, updated_at) VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            (key, serde_json::to_string(value)?, now),
        )?;
        Ok(())
    }

    pub fn delete_setting(&self, key: &str) -> Result<(), StoreError> {
        self.lock()
            .execute("DELETE FROM settings WHERE key = ?1", [key])?;
        Ok(())
    }
}
