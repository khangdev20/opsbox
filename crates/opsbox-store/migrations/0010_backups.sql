-- Phase 5 (prompt 13): local backup destination, generalized backup jobs, and snapshot catalog.

CREATE TABLE backup_destinations (
    id          TEXT PRIMARY KEY NOT NULL,
    provider    TEXT NOT NULL,
    name        TEXT NOT NULL,
    config_json TEXT NOT NULL CHECK (json_valid(config_json)),
    secrets_enc TEXT,
    verified_at INTEGER,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

INSERT INTO backup_destinations (id, provider, name, config_json, secrets_enc, verified_at, created_at, updated_at)
VALUES ('local', 'local', 'Local disk', '{"path":"/var/backups/opsbox"}', NULL, NULL, 0, 0);

CREATE TABLE backup_jobs_new (
    id               TEXT PRIMARY KEY NOT NULL,
    name             TEXT NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN ('database', 'volume', 'path', 'opsbox')),
    what_json        TEXT NOT NULL CHECK (json_valid(what_json)),
    service_id       TEXT UNIQUE REFERENCES services (id) ON DELETE CASCADE,
    template_id      TEXT,
    method           TEXT NOT NULL,
    cron             TEXT NOT NULL,
    retain_daily     INTEGER NOT NULL DEFAULT 14,
    retain_weekly    INTEGER NOT NULL DEFAULT 8,
    destination_id   TEXT NOT NULL DEFAULT 'local' REFERENCES backup_destinations (id),
    enabled          INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    last_run_at      INTEGER,
    last_status      TEXT CHECK (last_status IS NULL OR last_status IN ('ok', 'failed', 'running')),
    last_error       TEXT,
    last_snapshot_id TEXT,
    last_size_bytes  INTEGER,
    last_duration_ms INTEGER,
    next_run_at      INTEGER,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
) STRICT;

INSERT INTO backup_jobs_new (
    id, name, kind, what_json, service_id, template_id, method, cron,
    retain_daily, retain_weekly, destination_id, enabled, created_at, updated_at
)
SELECT
    bj.id,
    COALESCE((SELECT s.name FROM services s WHERE s.id = bj.service_id), bj.service_id),
    CASE WHEN bj.method = 'volume' THEN 'volume' ELSE 'database' END,
    CASE
        WHEN bj.method = 'volume' THEN json_object(
            'serviceId', bj.service_id,
            'serviceName', COALESCE((SELECT s.name FROM services s WHERE s.id = bj.service_id), bj.service_id),
            'volumes', json_array('opsbox-' || COALESCE((SELECT s.name FROM services s WHERE s.id = bj.service_id), bj.service_id) || '-data'),
            'pause', json('false')
        )
        ELSE json_object(
            'serviceId', bj.service_id,
            'serviceName', COALESCE((SELECT s.name FROM services s WHERE s.id = bj.service_id), bj.service_id),
            'templateId', bj.template_id,
            'method', bj.method
        )
    END,
    bj.service_id,
    bj.template_id,
    bj.method,
    bj.cron,
    bj.retain_daily,
    bj.retain_weekly,
    'local',
    bj.enabled,
    bj.created_at,
    bj.updated_at
FROM backup_jobs bj;

DROP TABLE backup_jobs;
ALTER TABLE backup_jobs_new RENAME TO backup_jobs;

CREATE INDEX backup_jobs_enabled_next ON backup_jobs (enabled, next_run_at);

INSERT INTO backup_jobs (
    id, name, kind, what_json, service_id, template_id, method, cron,
    retain_daily, retain_weekly, destination_id, enabled, created_at, updated_at
)
VALUES (
    'opsbox-self',
    'opsbox',
    'opsbox',
    '{"includeDb":true,"includeConfig":true}',
    NULL,
    NULL,
    'opsbox',
    '0 2 * * *',
    14,
    8,
    'local',
    1,
    0,
    0
);

CREATE TABLE backup_snapshots (
    id             TEXT PRIMARY KEY NOT NULL,
    job_id         TEXT REFERENCES backup_jobs (id) ON DELETE SET NULL,
    destination_id TEXT NOT NULL REFERENCES backup_destinations (id) ON DELETE CASCADE,
    restic_id      TEXT NOT NULL,
    short_id       TEXT NOT NULL,
    what           TEXT NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN ('database', 'volume', 'path', 'opsbox')),
    taken_at       INTEGER NOT NULL,
    size_bytes     INTEGER NOT NULL DEFAULT 0,
    check_state    TEXT NOT NULL DEFAULT 'unverified' CHECK (check_state IN ('unverified', 'checksum_ok', 'checksum_failed', 'restore_tested')),
    checked_at     INTEGER,
    tags_json      TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(tags_json)),
    paths_json     TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(paths_json)),
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    UNIQUE (destination_id, restic_id)
) STRICT;

CREATE INDEX backup_snapshots_taken ON backup_snapshots (taken_at DESC);
CREATE INDEX backup_snapshots_job ON backup_snapshots (job_id, taken_at DESC);
