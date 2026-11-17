-- Phase 1 / M2: panel accounts, sessions and sign-in failures.
-- Unlike the Phase 0 tables, the time columns here are unix seconds (INTEGER):
-- expiry and lockout decisions are comparisons in SQL, and RFC 3339 strings with a
-- variable number of fractional digits do not sort correctly.

CREATE TABLE users (
    id             TEXT PRIMARY KEY NOT NULL,
    name           TEXT NOT NULL UNIQUE COLLATE NOCASE,
    role           TEXT NOT NULL CHECK (role IN ('admin', 'deployer', 'viewer')),
    password_hash  TEXT NOT NULL,
    -- Sealed by priv with the master key ("enc:v1:..."); NULL until enrolment starts.
    totp_secret    TEXT,
    totp_enabled   INTEGER NOT NULL DEFAULT 0 CHECK (totp_enabled IN (0, 1)),
    -- Newest 30 s time step already accepted; a code for this step or an older one is refused.
    totp_last_step INTEGER NOT NULL DEFAULT 0,
    locked_until   INTEGER,
    failed_count   INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
) STRICT;

CREATE TABLE sessions (
    id          TEXT PRIMARY KEY NOT NULL,
    user_id     TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- sha256 of the bearer token; the token itself is never stored.
    token_hash  TEXT NOT NULL UNIQUE,
    -- web: a normal sign-in. enroll: password was right but 2FA must be set up first;
    -- only the enrolment endpoints accept it.
    kind        TEXT NOT NULL CHECK (kind IN ('web', 'enroll')),
    ip          TEXT,
    user_agent  TEXT,
    created_at  INTEGER NOT NULL,
    last_seen   INTEGER NOT NULL
) STRICT;
CREATE INDEX sessions_user ON sessions (user_id);

-- One row per failed sign-in. Lockout counts these per user name and per address.
CREATE TABLE login_failures (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    ts        INTEGER NOT NULL,
    user_name TEXT NOT NULL,
    ip        TEXT NOT NULL
) STRICT;
CREATE INDEX login_failures_user ON login_failures (user_name, ts);
CREATE INDEX login_failures_ip ON login_failures (ip, ts);
