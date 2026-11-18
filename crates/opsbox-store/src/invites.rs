//! Invitations for new panel accounts (migration 0004). Plain persistence: the policy
//! (expiry, who may invite) is in `opsbox-auth`.

use rusqlite::{OptionalExtension, params};

use crate::accounts::{USER_COLS, User, user_row_at};
use crate::{Store, StoreError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub id: String,
    pub user_id: String,
    pub expires_at: i64,
    pub created_by: String,
}

impl Store {
    /// Make (or replace) the invitation for `user_id`.
    pub fn put_invite(
        &self,
        user_id: &str,
        token_hash: &str,
        expires_at: i64,
        created_by: &str,
        now: i64,
    ) -> Result<(), StoreError> {
        let id = ulid::Ulid::generate().to_string();
        let conn = self.lock();
        conn.execute("DELETE FROM invites WHERE user_id = ?1", params![user_id])?;
        conn.execute(
            "INSERT INTO invites (id, user_id, token_hash, expires_at, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, user_id, token_hash, expires_at, created_by, now],
        )?;
        Ok(())
    }

    /// The invitation for a token hash, with the invited account.
    pub fn invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<(Invite, User)>, StoreError> {
        let cols: String = USER_COLS
            .split(", ")
            .map(|c| format!("u.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT i.id, i.user_id, i.expires_at, i.created_by, {cols} FROM invites i JOIN users u ON u.id = i.user_id WHERE i.token_hash = ?1"
        ))?;
        Ok(stmt
            .query_row(params![token_hash], |r| {
                Ok((
                    Invite {
                        id: r.get(0)?,
                        user_id: r.get(1)?,
                        expires_at: r.get(2)?,
                        created_by: r.get(3)?,
                    },
                    user_row_at(r, 4)?,
                ))
            })
            .optional()?)
    }

    /// Whether `user_id` still has an unclaimed invitation.
    pub fn has_invite(&self, user_id: &str) -> Result<bool, StoreError> {
        let n: i64 = self.lock().query_row(
            "SELECT COUNT(*) FROM invites WHERE user_id = ?1",
            params![user_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }

    pub fn delete_invite(&self, id: &str) -> Result<(), StoreError> {
        self.lock()
            .execute("DELETE FROM invites WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Remove expired invitations and the accounts that never got past them.
    pub fn purge_invites(&self, now: i64) -> Result<usize, StoreError> {
        let conn = self.lock();
        let n = conn.execute(
            "DELETE FROM users WHERE id IN (SELECT user_id FROM invites WHERE expires_at <= ?1) AND password_hash = ''",
            params![now],
        )?;
        conn.execute("DELETE FROM invites WHERE expires_at <= ?1", params![now])?;
        Ok(n)
    }

    /// Delete an account with its sessions, invitation and notification state.
    pub fn delete_user(&self, user_id: &str) -> Result<bool, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM attention_state WHERE user_id = ?1",
            params![user_id],
        )?;
        let deleted = tx.execute("DELETE FROM users WHERE id = ?1", params![user_id])? > 0;
        tx.commit()?;
        Ok(deleted)
    }
}
