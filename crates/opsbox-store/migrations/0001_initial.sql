-- Phase 0: the settings and audit tables from docs/BACKEND_SPEC.md section 5.
-- Timestamps are RFC 3339 UTC text. Ids are ULIDs.

-- Settings that the UI edits. `value` is JSON.
-- The key is the primary key: a second `id` column would only duplicate it.
CREATE TABLE settings (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL CHECK (json_valid(value)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

-- Append-only, hash-chained audit log.
--   hash = sha256(prev_hash || canonical_json([id, ts, actor, actor_kind, ip, action, target, result, detail]))
-- The first row's prev_hash is 64 zeros. `seq` fixes the order of the chain.
-- `ts` is the creation time; rows are never updated so there is no updated_at.
CREATE TABLE audit (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    id         TEXT NOT NULL UNIQUE,
    ts         TEXT NOT NULL,
    actor      TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    ip         TEXT,
    action     TEXT NOT NULL,
    target     TEXT,
    result     TEXT NOT NULL,
    detail     TEXT NOT NULL CHECK (json_valid(detail)),
    prev_hash  TEXT NOT NULL,
    hash       TEXT NOT NULL UNIQUE
) STRICT;

CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit
BEGIN
    SELECT RAISE(ABORT, 'audit log is append-only');
END;

CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit
BEGIN
    SELECT RAISE(ABORT, 'audit log is append-only');
END;
