-- Phase 3 (prompt 08): GitHub App manifest state, connection, deploy keys, and webhook deliveries.
-- Secret values (private_key_enc, webhook_secret_enc) are sealed by opsbox-priv using
-- XChaCha20-Poly1305 with the root-only master key; plaintext secrets never touch SQLite.

CREATE TABLE github_app (
    singleton             INTEGER PRIMARY KEY NOT NULL CHECK (singleton = 1),
    app_id                INTEGER NOT NULL,
    slug                  TEXT NOT NULL,
    name                  TEXT NOT NULL,
    owner_login           TEXT NOT NULL,
    owner_type            TEXT NOT NULL,
    html_url              TEXT NOT NULL,
    private_key_enc       TEXT NOT NULL,
    webhook_secret_enc    TEXT NOT NULL,
    webhook_url           TEXT,
    webhook_active        INTEGER NOT NULL DEFAULT 0 CHECK (webhook_active IN (0, 1)),
    installation_id       INTEGER,
    installation_account  TEXT,
    installation_html_url TEXT,
    repository_selection  TEXT,
    last_webhook_at       INTEGER,
    last_webhook_event    TEXT,
    created_by            TEXT NOT NULL,
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL
) STRICT;

-- Short-lived CSRF state tokens for the GitHub App manifest handshake (hashed with SHA-256).
CREATE TABLE github_manifest_states (
    state_hash TEXT PRIMARY KEY NOT NULL,
    org        TEXT,
    created_by TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

-- Per-repo SSH Ed25519 deploy keys (private_key_enc sealed by opsbox-priv).
CREATE TABLE deploy_keys (
    id              TEXT PRIMARY KEY NOT NULL,
    repo            TEXT NOT NULL,
    ssh_url         TEXT NOT NULL,
    public_key      TEXT NOT NULL,
    fingerprint     TEXT NOT NULL,
    private_key_enc TEXT NOT NULL,
    created_by      TEXT NOT NULL,
    created_at      INTEGER NOT NULL
) STRICT;
CREATE INDEX deploy_keys_created ON deploy_keys (created_at DESC);

-- Verified GitHub webhook delivery records (bounded in application code).
CREATE TABLE github_webhook_deliveries (
    id          TEXT PRIMARY KEY NOT NULL,
    delivery_id TEXT,
    event       TEXT NOT NULL,
    repo        TEXT,
    git_ref     TEXT,
    commit_sha  TEXT,
    pusher      TEXT,
    received_at INTEGER NOT NULL
) STRICT;
CREATE INDEX github_webhook_deliveries_at ON github_webhook_deliveries (received_at DESC);
