//! API tokens (migration 0004). Only the SHA-256 of a token is stored.

use std::borrow::Cow;

use rusqlite::{OptionalExtension, Row, params};

use crate::{Store, StoreError};

/// Validated service slug rule (lowercase ASCII alphanumeric and hyphens, 1..=63 chars).
fn is_valid_service_slug(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
        && b[b.len() - 1] != b'-'
}

/// What a token may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenScope {
    /// Read-only, like a viewer.
    Read,
    /// Deploy any service and read logs, like a deployer.
    Deploy,
    /// Deploy and read logs for a single named service only.
    DeployService(String),
    /// Everything an administrator can do except the account and terminal endpoints.
    Full,
}

impl TokenScope {
    pub fn as_str(&self) -> Cow<'_, str> {
        match self {
            Self::Read => Cow::Borrowed("read"),
            Self::Deploy => Cow::Borrowed("deploy"),
            Self::DeployService(svc) => Cow::Owned(format!("deploy:{svc}")),
            Self::Full => Cow::Borrowed("full"),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Self::Read),
            "deploy" => Some(Self::Deploy),
            "full" => Some(Self::Full),
            _ => s
                .strip_prefix("deploy:")
                .filter(|svc| is_valid_service_slug(svc))
                .map(|svc| Self::DeployService(svc.to_owned())),
        }
    }

    pub fn service_name(&self) -> Option<&str> {
        match self {
            Self::DeployService(svc) => Some(svc),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    pub scope: TokenScope,
    pub created_by: String,
    pub created_at: i64,
    pub last_used_at: Option<i64>,
}

const COLS: &str = "id, name, scope, created_by, created_at, last_used_at";

fn row(r: &Row<'_>) -> rusqlite::Result<ApiToken> {
    let scope: String = r.get(2)?;
    Ok(ApiToken {
        id: r.get(0)?,
        name: r.get(1)?,
        scope: TokenScope::parse(&scope).unwrap_or(TokenScope::Read),
        created_by: r.get(3)?,
        created_at: r.get(4)?,
        last_used_at: r.get(5)?,
    })
}

impl Store {
    /// Fails with a unique-constraint error if the name is taken.
    pub fn create_api_token(
        &self,
        name: &str,
        token_hash: &str,
        scope: TokenScope,
        created_by: &str,
        now: i64,
    ) -> Result<ApiToken, StoreError> {
        let id = ulid::Ulid::generate().to_string();
        let scope_str = scope.as_str().into_owned();
        self.lock().execute(
            "INSERT INTO api_tokens (id, name, token_hash, scope, created_by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, name, token_hash, scope_str, created_by, now],
        )?;
        Ok(ApiToken {
            id,
            name: name.to_owned(),
            scope,
            created_by: created_by.to_owned(),
            created_at: now,
            last_used_at: None,
        })
    }

    pub fn list_api_tokens(&self) -> Result<Vec<ApiToken>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM api_tokens ORDER BY created_at, name"
        ))?;
        Ok(stmt.query_map([], row)?.collect::<Result<_, _>>()?)
    }

    pub fn api_token_by_hash(&self, token_hash: &str) -> Result<Option<ApiToken>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {COLS} FROM api_tokens WHERE token_hash = ?1"),
                params![token_hash],
                row,
            )
            .optional()?)
    }

    pub fn api_token_by_id(&self, id: &str) -> Result<Option<ApiToken>, StoreError> {
        Ok(self
            .lock()
            .query_row(
                &format!("SELECT {COLS} FROM api_tokens WHERE id = ?1"),
                [id],
                row,
            )
            .optional()?)
    }

    /// Remember when a token was last used (at most once a minute per token).
    pub fn touch_api_token(&self, id: &str, now: i64) -> Result<(), StoreError> {
        self.lock().execute(
            "UPDATE api_tokens SET last_used_at = ?2 WHERE id = ?1 AND (last_used_at IS NULL OR last_used_at < ?2 - 60)",
            params![id, now],
        )?;
        Ok(())
    }

    pub fn delete_api_token(&self, id: &str) -> Result<Option<ApiToken>, StoreError> {
        let conn = self.lock();
        let found = conn
            .query_row(
                &format!("SELECT {COLS} FROM api_tokens WHERE id = ?1"),
                params![id],
                row,
            )
            .optional()?;
        conn.execute("DELETE FROM api_tokens WHERE id = ?1", params![id])?;
        Ok(found)
    }
}
