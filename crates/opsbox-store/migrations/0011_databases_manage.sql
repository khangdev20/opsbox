-- Prompt 16: Databases part B (per-app logins, connection pooler, major upgrade volume).

CREATE TABLE database_settings (
    service_id       TEXT PRIMARY KEY NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    -- Named Docker volume holding the data after a major upgrade (NULL = the default one).
    data_volume      TEXT,
    pooler_enabled   INTEGER NOT NULL DEFAULT 0 CHECK (pooler_enabled IN (0, 1)),
    pooler_container TEXT,
    pooler_mode      TEXT NOT NULL DEFAULT 'transaction' CHECK (pooler_mode IN ('transaction', 'session')),
    updated_at       INTEGER NOT NULL
) STRICT;

-- One database and one least-privilege login per connected app.
CREATE TABLE database_apps (
    db_service_id  TEXT NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    app_service_id TEXT NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    db_name        TEXT NOT NULL,
    role_name      TEXT NOT NULL,
    var_prefix     TEXT NOT NULL,
    pooled         INTEGER NOT NULL DEFAULT 0 CHECK (pooled IN (0, 1)),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (db_service_id, app_service_id)
) STRICT;
CREATE INDEX database_apps_app ON database_apps (app_service_id);
