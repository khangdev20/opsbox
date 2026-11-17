-- Phase 1 / M7: invitations for new panel accounts, and API tokens.

-- An invited person has a `users` row with an empty password hash (which never verifies)
-- until they claim the invitation and choose a password. The invitation holds only the
-- SHA-256 of the link's token, so a read of this table cannot be turned into a sign-in.
CREATE TABLE invites (
    id         TEXT PRIMARY KEY NOT NULL,
    user_id    TEXT NOT NULL UNIQUE REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at INTEGER NOT NULL,
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL
) STRICT;

-- API tokens: shown once when created, stored as a SHA-256 hash. `scope` decides what the
-- token may do: read (viewer), deploy (deployer) or full (administrator, without the
-- account and terminal endpoints, which no token can reach).
CREATE TABLE api_tokens (
    id           TEXT PRIMARY KEY NOT NULL,
    name         TEXT NOT NULL UNIQUE COLLATE NOCASE,
    token_hash   TEXT NOT NULL UNIQUE,
    scope        TEXT NOT NULL CHECK (scope IN ('read', 'deploy', 'full')),
    created_by   TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
) STRICT;
