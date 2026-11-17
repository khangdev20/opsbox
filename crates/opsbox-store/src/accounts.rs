//! Panel accounts, sessions and sign-in failures (migration 0002).
//!
//! Plain persistence: no hashing, no policy. Those live in `opsbox-auth`.

use opsbox_proto::Role;
use rusqlite::{OptionalExtension, Row, params};

use crate::{Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub password_hash: String,
    /// Sealed by `priv`; `None` until enrolment starts.
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub totp_last_step: u64,
    pub locked_until: Option<i64>,
    pub failed_count: u32,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Web,
    Enroll,
}

impl SessionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Enroll => "enroll",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub kind: SessionKind,
    pub ip: Option<String>,
    pub created_at: i64,
    pub last_seen: i64,
}

pub(crate) const USER_COLS: &str = "id, name, role, password_hash, totp_secret, totp_enabled, totp_last_step, locked_until, failed_count, created_at";

fn user_row(r: &Row<'_>) -> rusqlite::Result<User> {
    user_row_at(r, 0)
}

impl Store {
    // ------------------------------------------------------------------ users

    /// Insert a new account. Fails (unique constraint) if the name is taken.
    pub fn create_user(
        &self,
        name: &str,
        role: Role,
        password_hash: &str,
        now: i64,
    ) -> Result<User, StoreError> {
        let id = ulid::Ulid::generate().to_string();
        self.lock().execute(
            "INSERT INTO users (id, name, role, password_hash, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
            params![id, name, role.as_str(), password_hash, now],
        )?;
        Ok(self.user_by_id(&id)?.expect("just inserted"))
    }

    pub fn user_by_name(&self, name: &str) -> Result<Option<User>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {USER_COLS} FROM users WHERE name = ?1"),
                [name],
                user_row,
            )
            .optional()?)
    }

    pub fn user_by_id(&self, id: &str) -> Result<Option<User>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {USER_COLS} FROM users WHERE id = ?1"),
                [id],
                user_row,
            )
            .optional()?)
    }

    pub fn list_users(&self) -> Result<Vec<User>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {USER_COLS} FROM users ORDER BY created_at, name"
        ))?;
        Ok(stmt.query_map([], user_row)?.collect::<Result<_, _>>()?)
    }

    /// Admins who finished two-factor enrolment. Setup is over once there is one.
    pub fn enrolled_admin_count(&self) -> Result<u64, StoreError> {
        let n: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM users WHERE role = 'admin' AND totp_enabled = 1",
            [],
            |r| r.get(0),
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    /// Remove admins that never finished enrolment (an abandoned setup).
    pub fn delete_unenrolled_admins(&self) -> Result<usize, StoreError> {
        Ok(self.lock().execute(
            "DELETE FROM users WHERE role = 'admin' AND totp_enabled = 0",
            [],
        )?)
    }

    pub fn set_password_hash(&self, user_id: &str, hash: &str, now: i64) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE users SET password_hash = ?2, updated_at = ?3 WHERE id = ?1",
            params![user_id, hash, now],
        )?;
        Ok(())
    }

    /// Store a freshly sealed TOTP secret; two-factor stays off until confirmed.
    pub fn set_pending_totp(
        &self,
        user_id: &str,
        sealed: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE users SET totp_secret = ?2, totp_enabled = 0, totp_last_step = 0, updated_at = ?3 WHERE id = ?1",
            params![user_id, sealed, now],
        )?;
        Ok(())
    }

    /// Switch two-factor on, recording the step of the confirming code.
    pub fn enable_totp(&self, user_id: &str, step: u64, now: i64) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE users SET totp_enabled = 1, totp_last_step = ?2, updated_at = ?3 WHERE id = ?1 AND totp_secret IS NOT NULL",
            params![user_id, i64::try_from(step).unwrap_or(i64::MAX), now],
        )?;
        Ok(())
    }

    /// Forget a user's second factor (used by `opsbox user reset-2fa`).
    pub fn clear_totp(&self, user_id: &str, now: i64) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE users SET totp_secret = NULL, totp_enabled = 0, totp_last_step = 0, updated_at = ?2 WHERE id = ?1",
            params![user_id, now],
        )?;
        Ok(())
    }

    /// Record that the code for `step` was used. Returns `false` if that step (or a
    /// newer one) was already recorded, which is how two simultaneous sign-ins with
    /// the same code are told apart: only one wins.
    pub fn advance_totp_step(&self, user_id: &str, step: u64) -> Result<bool, StoreError> {
        let step = i64::try_from(step).unwrap_or(i64::MAX);
        let changed = self.lock().execute(
            "UPDATE users SET totp_last_step = ?2 WHERE id = ?1 AND totp_last_step < ?2",
            params![user_id, step],
        )?;
        Ok(changed == 1)
    }

    pub fn set_lock(
        &self,
        user_id: &str,
        until: Option<i64>,
        failed_count: u32,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE users SET locked_until = ?2, failed_count = ?3 WHERE id = ?1",
            params![user_id, until, failed_count],
        )?;
        Ok(())
    }

    // --------------------------------------------------------------- sessions

    pub fn create_session(
        &self,
        user_id: &str,
        token_hash: &str,
        kind: SessionKind,
        ip: Option<&str>,
        user_agent: Option<&str>,
        now: i64,
    ) -> Result<Session, StoreError> {
        let id = ulid::Ulid::generate().to_string();
        self.lock().execute(
            "INSERT INTO sessions (id, user_id, token_hash, kind, ip, user_agent, created_at, last_seen) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![id, user_id, token_hash, kind.as_str(), ip, user_agent, now],
        )?;
        Ok(Session {
            id,
            user_id: user_id.to_owned(),
            kind,
            ip: ip.map(str::to_owned),
            created_at: now,
            last_seen: now,
        })
    }

    /// The session and its user for a token hash, if any (expiry is the caller's call).
    pub fn session_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<(Session, User)>, StoreError> {
        let conn = self.lock();
        let cols: String = USER_COLS
            .split(", ")
            .map(|c| format!("u.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT s.id, s.user_id, s.kind, s.ip, s.created_at, s.last_seen, {cols} FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?1"
        );
        Ok(conn
            .query_row(&sql, [token_hash], |r| {
                let kind: String = r.get(2)?;
                let session = Session {
                    id: r.get(0)?,
                    user_id: r.get(1)?,
                    kind: if kind == "enroll" {
                        SessionKind::Enroll
                    } else {
                        SessionKind::Web
                    },
                    ip: r.get(3)?,
                    created_at: r.get(4)?,
                    last_seen: r.get(5)?,
                };
                // User columns start after the six session columns.
                let user = user_row_at(r, 6)?;
                Ok((session, user))
            })
            .optional()?)
    }

    pub fn touch_session(&self, session_id: &str, now: i64) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE sessions SET last_seen = ?2 WHERE id = ?1",
            params![session_id, now],
        )?;
        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<(), StoreError> {
        self.lock()
            .execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
        Ok(())
    }

    /// Sign a user out everywhere; returns how many sessions were removed.
    pub fn delete_user_sessions(&self, user_id: &str) -> Result<usize, StoreError> {
        Ok(self
            .lock()
            .execute("DELETE FROM sessions WHERE user_id = ?1", [user_id])?)
    }

    /// Remove sessions that are past either limit.
    pub fn purge_sessions(
        &self,
        idle_before: i64,
        created_before: i64,
    ) -> Result<usize, StoreError> {
        Ok(self.lock().execute(
            "DELETE FROM sessions WHERE last_seen < ?1 OR created_at < ?2",
            params![idle_before, created_before],
        )?)
    }

    pub fn session_count(&self, user_id: &str) -> Result<u64, StoreError> {
        let n: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM sessions WHERE user_id = ?1",
            [user_id],
            |r| r.get(0),
        )?;
        Ok(u64::try_from(n).unwrap_or(0))
    }

    // ------------------------------------------------------- sign-in failures

    pub fn record_login_failure(
        &self,
        user_name: &str,
        ip: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO login_failures (ts, user_name, ip) VALUES (?1, ?2, ?3)",
            params![now, user_name.to_lowercase(), ip],
        )?;
        Ok(())
    }

    /// (count, newest timestamp) of failures for a user name since `since`.
    pub fn failures_for_user(
        &self,
        user_name: &str,
        since: i64,
    ) -> Result<(u32, Option<i64>), StoreError> {
        self.failures("user_name = ?1", &user_name.to_lowercase(), since)
    }

    pub fn failures_for_ip(&self, ip: &str, since: i64) -> Result<(u32, Option<i64>), StoreError> {
        self.failures("ip = ?1", ip, since)
    }

    fn failures(
        &self,
        cond: &str,
        value: &str,
        since: i64,
    ) -> Result<(u32, Option<i64>), StoreError> {
        let (n, newest): (i64, Option<i64>) = self.lock().query_row(
            &format!("SELECT COUNT(*), MAX(ts) FROM login_failures WHERE {cond} AND ts >= ?2"),
            params![value, since],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((u32::try_from(n).unwrap_or(u32::MAX), newest))
    }

    pub fn clear_failures_for_user(&self, user_name: &str) -> Result<(), StoreError> {
        self.lock().execute(
            "DELETE FROM login_failures WHERE user_name = ?1",
            [user_name.to_lowercase()],
        )?;
        Ok(())
    }

    pub fn prune_failures(&self, before: i64) -> Result<usize, StoreError> {
        Ok(self
            .lock()
            .execute("DELETE FROM login_failures WHERE ts < ?1", [before])?)
    }
}

pub(crate) fn user_row_at(r: &Row<'_>, base: usize) -> rusqlite::Result<User> {
    let role: String = r.get(base + 2)?;
    Ok(User {
        id: r.get(base)?,
        name: r.get(base + 1)?,
        role: Role::parse(&role).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                base + 2,
                rusqlite::types::Type::Text,
                "unknown role".into(),
            )
        })?,
        password_hash: r.get(base + 3)?,
        totp_secret: r.get(base + 4)?,
        totp_enabled: r.get::<_, i64>(base + 5)? != 0,
        totp_last_step: u64::try_from(r.get::<_, i64>(base + 6)?).unwrap_or(0),
        locked_until: r.get(base + 7)?,
        failed_count: u32::try_from(r.get::<_, i64>(base + 8)?).unwrap_or(u32::MAX),
        created_at: r.get(base + 9)?,
    })
}
