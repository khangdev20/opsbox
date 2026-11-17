//! Storage for GitHub App connection, manifest states, deploy keys, and webhook deliveries.

use rusqlite::{OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use crate::{Store, StoreError};

const MAX_PENDING_MANIFEST_STATES: i64 = 32;
const MAX_WEBHOOK_DELIVERIES: i64 = 100;

/// Stored GitHub App connection (`private_key_enc` and `webhook_secret_enc` are sealed by `priv`).
#[derive(Clone, PartialEq, Eq)]
pub struct GithubAppRecord {
    pub app_id: i64,
    pub slug: String,
    pub name: String,
    pub owner_login: String,
    pub owner_type: String,
    pub html_url: String,
    pub private_key_enc: String,
    pub webhook_secret_enc: String,
    pub webhook_url: Option<String>,
    pub webhook_active: bool,
    pub installation_id: Option<i64>,
    pub installation_account: Option<String>,
    pub installation_html_url: Option<String>,
    pub repository_selection: Option<String>,
    pub last_webhook_at: Option<i64>,
    pub last_webhook_event: Option<String>,
    pub created_by: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl std::fmt::Debug for GithubAppRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GithubAppRecord")
            .field("app_id", &self.app_id)
            .field("slug", &self.slug)
            .field("name", &self.name)
            .field("owner_login", &self.owner_login)
            .field("installation_id", &self.installation_id)
            .field("private_key_enc", &"[sealed]")
            .field("webhook_secret_enc", &"[sealed]")
            .finish_non_exhaustive()
    }
}

pub struct NewGithubApp {
    pub app_id: i64,
    pub slug: String,
    pub name: String,
    pub owner_login: String,
    pub owner_type: String,
    pub html_url: String,
    pub private_key_enc: String,
    pub webhook_secret_enc: String,
    pub webhook_url: Option<String>,
    pub webhook_active: bool,
    pub installation_id: Option<i64>,
    pub installation_account: Option<String>,
    pub installation_html_url: Option<String>,
    pub repository_selection: Option<String>,
    pub created_by: String,
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingManifestState {
    pub org: Option<String>,
    pub created_by: String,
    pub expires_at: i64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DeployKeyRecord {
    pub id: String,
    pub repo: String,
    pub ssh_url: String,
    pub public_key: String,
    pub fingerprint: String,
    pub private_key_enc: String,
    pub created_by: String,
    pub created_at: i64,
}

impl std::fmt::Debug for DeployKeyRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeployKeyRecord")
            .field("id", &self.id)
            .field("repo", &self.repo)
            .field("ssh_url", &self.ssh_url)
            .field("fingerprint", &self.fingerprint)
            .field("private_key_enc", &"[sealed]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployKeyInfo {
    pub id: String,
    pub repo: String,
    pub ssh_url: String,
    pub public_key: String,
    pub fingerprint: String,
    pub created_by: String,
    pub created_at: i64,
}

pub struct NewDeployKey {
    pub id: String,
    pub repo: String,
    pub ssh_url: String,
    pub public_key: String,
    pub fingerprint: String,
    pub private_key_enc: String,
    pub created_by: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookDelivery {
    pub id: String,
    pub delivery_id: Option<String>,
    pub event: String,
    pub repo: Option<String>,
    pub git_ref: Option<String>,
    pub commit_sha: Option<String>,
    pub pusher: Option<String>,
    pub received_at: i64,
}

pub struct NewWebhookDelivery {
    pub delivery_id: Option<String>,
    pub event: String,
    pub repo: Option<String>,
    pub git_ref: Option<String>,
    pub commit_sha: Option<String>,
    pub pusher: Option<String>,
    pub received_at: i64,
}

pub fn hash_state(state: &str) -> String {
    let digest = Sha256::digest(state.as_bytes());
    let mut out = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

impl Store {
    pub fn save_github_manifest_state(
        &self,
        state: &str,
        org: Option<&str>,
        created_by: &str,
        expires_at: i64,
        now: i64,
    ) -> Result<(), StoreError> {
        let hash = hash_state(state);
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM github_manifest_states WHERE expires_at <= ?1",
            [now],
        )?;
        tx.execute(
            "INSERT INTO github_manifest_states (state_hash, org, created_by, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (&hash, org, created_by, expires_at, now),
        )?;
        // Cap pending manifest states so repeated clicks cannot grow the table unboundedly.
        tx.execute(
            "DELETE FROM github_manifest_states WHERE state_hash NOT IN (
                SELECT state_hash FROM github_manifest_states ORDER BY created_at DESC LIMIT ?1
            )",
            [MAX_PENDING_MANIFEST_STATES],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn consume_github_manifest_state(
        &self,
        state: &str,
        now: i64,
    ) -> Result<Option<PendingManifestState>, StoreError> {
        let hash = hash_state(state);
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM github_manifest_states WHERE expires_at <= ?1",
            [now],
        )?;
        let row = tx
            .query_row(
                "SELECT org, created_by, expires_at FROM github_manifest_states WHERE state_hash = ?1",
                [&hash],
                |r| {
                    Ok(PendingManifestState {
                        org: r.get(0)?,
                        created_by: r.get(1)?,
                        expires_at: r.get(2)?,
                    })
                },
            )
            .optional()?;
        if row.is_some() {
            tx.execute(
                "DELETE FROM github_manifest_states WHERE state_hash = ?1",
                [&hash],
            )?;
        }
        tx.commit()?;
        Ok(row.filter(|r| r.expires_at > now))
    }

    pub fn get_github_app(&self) -> Result<Option<GithubAppRecord>, StoreError> {
        self.lock()
            .query_row(
                "SELECT app_id, slug, name, owner_login, owner_type, html_url,
                        private_key_enc, webhook_secret_enc, webhook_url, webhook_active,
                        installation_id, installation_account, installation_html_url,
                        repository_selection, last_webhook_at, last_webhook_event,
                        created_by, created_at, updated_at
                 FROM github_app WHERE singleton = 1",
                [],
                |r| {
                    Ok(GithubAppRecord {
                        app_id: r.get(0)?,
                        slug: r.get(1)?,
                        name: r.get(2)?,
                        owner_login: r.get(3)?,
                        owner_type: r.get(4)?,
                        html_url: r.get(5)?,
                        private_key_enc: r.get(6)?,
                        webhook_secret_enc: r.get(7)?,
                        webhook_url: r.get(8)?,
                        webhook_active: r.get::<_, i64>(9)? != 0,
                        installation_id: r.get(10)?,
                        installation_account: r.get(11)?,
                        installation_html_url: r.get(12)?,
                        repository_selection: r.get(13)?,
                        last_webhook_at: r.get(14)?,
                        last_webhook_event: r.get(15)?,
                        created_by: r.get(16)?,
                        created_at: r.get(17)?,
                        updated_at: r.get(18)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn upsert_github_app(&self, app: &NewGithubApp) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO github_app (
                singleton, app_id, slug, name, owner_login, owner_type, html_url,
                private_key_enc, webhook_secret_enc, webhook_url, webhook_active,
                installation_id, installation_account, installation_html_url,
                repository_selection, created_by, created_at, updated_at
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16)
             ON CONFLICT (singleton) DO UPDATE SET
                app_id = excluded.app_id,
                slug = excluded.slug,
                name = excluded.name,
                owner_login = excluded.owner_login,
                owner_type = excluded.owner_type,
                html_url = excluded.html_url,
                private_key_enc = excluded.private_key_enc,
                webhook_secret_enc = excluded.webhook_secret_enc,
                webhook_url = excluded.webhook_url,
                webhook_active = excluded.webhook_active,
                installation_id = excluded.installation_id,
                installation_account = excluded.installation_account,
                installation_html_url = excluded.installation_html_url,
                repository_selection = excluded.repository_selection,
                last_webhook_at = NULL,
                last_webhook_event = NULL,
                created_by = excluded.created_by,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at",
            (
                app.app_id,
                &app.slug,
                &app.name,
                &app.owner_login,
                &app.owner_type,
                &app.html_url,
                &app.private_key_enc,
                &app.webhook_secret_enc,
                &app.webhook_url,
                i64::from(app.webhook_active),
                app.installation_id,
                &app.installation_account,
                &app.installation_html_url,
                &app.repository_selection,
                &app.created_by,
                app.now,
            ),
        )?;
        Ok(())
    }

    pub fn update_github_installation(
        &self,
        installation_id: Option<i64>,
        installation_account: Option<&str>,
        installation_html_url: Option<&str>,
        repository_selection: Option<&str>,
        updated_at: i64,
    ) -> Result<bool, StoreError> {
        let changed = self.lock().execute(
            "UPDATE github_app
             SET installation_id = ?1,
                 installation_account = ?2,
                 installation_html_url = ?3,
                 repository_selection = ?4,
                 updated_at = ?5
             WHERE singleton = 1",
            (
                installation_id,
                installation_account,
                installation_html_url,
                repository_selection,
                updated_at,
            ),
        )?;
        Ok(changed > 0)
    }

    pub fn update_github_webhook(
        &self,
        webhook_url: Option<&str>,
        webhook_active: bool,
        updated_at: i64,
    ) -> Result<bool, StoreError> {
        let changed = self.lock().execute(
            "UPDATE github_app
             SET webhook_url = ?1,
                 webhook_active = ?2,
                 updated_at = ?3
             WHERE singleton = 1",
            (webhook_url, i64::from(webhook_active), updated_at),
        )?;
        Ok(changed > 0)
    }

    pub fn delete_github_app(&self) -> Result<bool, StoreError> {
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let deleted = tx.execute("DELETE FROM github_app WHERE singleton = 1", [])?;
        tx.execute("DELETE FROM github_manifest_states", [])?;
        tx.commit()?;
        Ok(deleted > 0)
    }

    pub fn record_github_webhook_delivery(
        &self,
        d: &NewWebhookDelivery,
    ) -> Result<String, StoreError> {
        let id = ulid::Ulid::generate().to_string();
        let mut conn = self.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO github_webhook_deliveries (
                id, delivery_id, event, repo, git_ref, commit_sha, pusher, received_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                &id,
                &d.delivery_id,
                &d.event,
                &d.repo,
                &d.git_ref,
                &d.commit_sha,
                &d.pusher,
                d.received_at,
            ),
        )?;
        tx.execute(
            "UPDATE github_app
             SET last_webhook_at = ?1, last_webhook_event = ?2
             WHERE singleton = 1",
            (d.received_at, &d.event),
        )?;
        tx.execute(
            "DELETE FROM github_webhook_deliveries WHERE id NOT IN (
                SELECT id FROM github_webhook_deliveries ORDER BY received_at DESC, id DESC LIMIT ?1
            )",
            [MAX_WEBHOOK_DELIVERIES],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn list_github_webhook_deliveries(
        &self,
        limit: usize,
    ) -> Result<Vec<WebhookDelivery>, StoreError> {
        let limit = i64::try_from(limit.min(100)).unwrap_or(100);
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, delivery_id, event, repo, git_ref, commit_sha, pusher, received_at
             FROM github_webhook_deliveries
             ORDER BY received_at DESC, id DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit], |r| {
                Ok(WebhookDelivery {
                    id: r.get(0)?,
                    delivery_id: r.get(1)?,
                    event: r.get(2)?,
                    repo: r.get(3)?,
                    git_ref: r.get(4)?,
                    commit_sha: r.get(5)?,
                    pusher: r.get(6)?,
                    received_at: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn create_deploy_key(&self, k: &NewDeployKey) -> Result<(), StoreError> {
        self.lock().execute(
            "INSERT INTO deploy_keys (
                id, repo, ssh_url, public_key, fingerprint, private_key_enc, created_by, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                &k.id,
                &k.repo,
                &k.ssh_url,
                &k.public_key,
                &k.fingerprint,
                &k.private_key_enc,
                &k.created_by,
                k.created_at,
            ),
        )?;
        Ok(())
    }

    pub fn list_deploy_keys(&self) -> Result<Vec<DeployKeyInfo>, StoreError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT id, repo, ssh_url, public_key, fingerprint, created_by, created_at
             FROM deploy_keys
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(DeployKeyInfo {
                    id: r.get(0)?,
                    repo: r.get(1)?,
                    ssh_url: r.get(2)?,
                    public_key: r.get(3)?,
                    fingerprint: r.get(4)?,
                    created_by: r.get(5)?,
                    created_at: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn get_deploy_key(&self, id: &str) -> Result<Option<DeployKeyRecord>, StoreError> {
        self.lock()
            .query_row(
                "SELECT id, repo, ssh_url, public_key, fingerprint, private_key_enc, created_by, created_at
                 FROM deploy_keys WHERE id = ?1",
                [id],
                |r| {
                    Ok(DeployKeyRecord {
                        id: r.get(0)?,
                        repo: r.get(1)?,
                        ssh_url: r.get(2)?,
                        public_key: r.get(3)?,
                        fingerprint: r.get(4)?,
                        private_key_enc: r.get(5)?,
                        created_by: r.get(6)?,
                        created_at: r.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn delete_deploy_key(&self, id: &str) -> Result<bool, StoreError> {
        let deleted = self
            .lock()
            .execute("DELETE FROM deploy_keys WHERE id = ?1", [id])?;
        Ok(deleted > 0)
    }
}
