//! Managed-database state: per-app logins, pooler and upgrade volume (migration 0011).

use rusqlite::{OptionalExtension, Row, TransactionBehavior, params};

use crate::{Store, StoreError};

/// Pooler, volume and mode for one database service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseSettings {
    pub service_id: String,
    pub data_volume: Option<String>,
    pub pooler_enabled: bool,
    pub pooler_container: Option<String>,
    pub pooler_mode: String,
}

/// One app's database and login on a managed database server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseAppRecord {
    pub db_service_id: String,
    pub app_service_id: String,
    pub db_name: String,
    pub role_name: String,
    pub var_prefix: String,
    pub pooled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

fn app_from_row(r: &Row<'_>) -> rusqlite::Result<DatabaseAppRecord> {
    Ok(DatabaseAppRecord {
        db_service_id: r.get(0)?,
        app_service_id: r.get(1)?,
        db_name: r.get(2)?,
        role_name: r.get(3)?,
        var_prefix: r.get(4)?,
        pooled: r.get::<_, i64>(5)? != 0,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

const APP_COLS: &str =
    "db_service_id, app_service_id, db_name, role_name, var_prefix, pooled, created_at, updated_at";

impl Store {
    /// Settings for a database service (defaults when none were saved yet).
    pub fn get_database_settings(&self, service_id: &str) -> Result<DatabaseSettings, StoreError> {
        let found = self
            .lock()
            .query_row(
                "SELECT service_id, data_volume, pooler_enabled, pooler_container, pooler_mode
                 FROM database_settings WHERE service_id = ?1",
                params![service_id],
                |r| {
                    Ok(DatabaseSettings {
                        service_id: r.get(0)?,
                        data_volume: r.get(1)?,
                        pooler_enabled: r.get::<_, i64>(2)? != 0,
                        pooler_container: r.get(3)?,
                        pooler_mode: r.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(found.unwrap_or_else(|| DatabaseSettings {
            service_id: service_id.to_owned(),
            data_volume: None,
            pooler_enabled: false,
            pooler_container: None,
            pooler_mode: "transaction".to_owned(),
        }))
    }

    pub fn set_database_data_volume(
        &self,
        service_id: &str,
        volume: Option<&str>,
        now: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO database_settings (service_id, data_volume, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(service_id) DO UPDATE SET data_volume = ?2, updated_at = ?3",
            params![service_id, volume, now],
        )?;
        Ok(())
    }

    pub fn set_database_pooler(
        &self,
        service_id: &str,
        enabled: bool,
        container: Option<&str>,
        mode: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO database_settings (service_id, pooler_enabled, pooler_container, pooler_mode, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(service_id) DO UPDATE SET
               pooler_enabled = ?2, pooler_container = ?3, pooler_mode = ?4, updated_at = ?5",
            params![service_id, i64::from(enabled), container, mode, now],
        )?;
        Ok(())
    }

    pub fn list_database_apps(
        &self,
        db_service_id: &str,
    ) -> Result<Vec<DatabaseAppRecord>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {APP_COLS} FROM database_apps WHERE db_service_id = ?1 ORDER BY role_name ASC"
        ))?;
        let rows = stmt.query_map(params![db_service_id], app_from_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn get_database_app(
        &self,
        db_service_id: &str,
        app_service_id: &str,
    ) -> Result<Option<DatabaseAppRecord>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!(
                    "SELECT {APP_COLS} FROM database_apps WHERE db_service_id = ?1 AND app_service_id = ?2"
                ),
                params![db_service_id, app_service_id],
                app_from_row,
            )
            .optional()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_database_app(
        &self,
        db_service_id: &str,
        app_service_id: &str,
        db_name: &str,
        role_name: &str,
        var_prefix: &str,
        pooled: bool,
        now: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO database_apps
               (db_service_id, app_service_id, db_name, role_name, var_prefix, pooled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(db_service_id, app_service_id) DO UPDATE SET
               db_name = ?3, role_name = ?4, var_prefix = ?5, pooled = ?6, updated_at = ?7",
            params![
                db_service_id,
                app_service_id,
                db_name,
                role_name,
                var_prefix,
                i64::from(pooled),
                now
            ],
        )?;
        Ok(())
    }

    pub fn set_database_app_pooled(
        &self,
        db_service_id: &str,
        app_service_id: &str,
        pooled: bool,
        now: i64,
    ) -> Result<bool, StoreError> {
        let n = self.lock().execute(
            "UPDATE database_apps SET pooled = ?3, updated_at = ?4
             WHERE db_service_id = ?1 AND app_service_id = ?2",
            params![db_service_id, app_service_id, i64::from(pooled), now],
        )?;
        Ok(n > 0)
    }

    /// Forget an app login and remove its stored connection variables from the database service.
    pub fn remove_database_app(
        &self,
        db_service_id: &str,
        app_service_id: &str,
    ) -> Result<(), StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prefix: Option<String> = tx
            .query_row(
                "SELECT var_prefix FROM database_apps WHERE db_service_id = ?1 AND app_service_id = ?2",
                params![db_service_id, app_service_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prefix) = prefix {
            // Exact keys only: `OPSBOX_APP_WEB_` is a text prefix of `OPSBOX_APP_WEB_API_`.
            for (suffix, _, _, _) in opsbox_proto::APP_CONNECTION_VARS {
                tx.execute(
                    "DELETE FROM service_vars WHERE service_id = ?1 AND key = ?2",
                    params![db_service_id, format!("{prefix}{suffix}")],
                )?;
            }
        }
        tx.execute(
            "DELETE FROM database_apps WHERE db_service_id = ?1 AND app_service_id = ?2",
            params![db_service_id, app_service_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Point the service at a new template version and running container after a major upgrade.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_database_upgrade(
        &self,
        service_id: &str,
        version: &str,
        image_ref: &str,
        container_id: &str,
        host_port: u16,
        data_volume: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE services SET branch = ?2, repo = ?3, image_ref = ?3, active_container = ?4,
                    active_host_port = ?5, state = 'active', updated_at = ?6
             WHERE id = ?1",
            params![
                service_id,
                version,
                image_ref,
                container_id,
                (host_port > 0).then_some(i64::from(host_port)),
                now
            ],
        )?;
        tx.execute(
            "INSERT INTO database_settings (service_id, data_volume, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(service_id) DO UPDATE SET data_volume = ?2, updated_at = ?3",
            params![service_id, data_volume, now],
        )?;
        tx.commit()?;
        Ok(())
    }
}
