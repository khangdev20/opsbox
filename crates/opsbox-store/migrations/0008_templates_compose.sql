-- Phase 3 (prompt 11): service templates metadata, backup job configs, and Compose stacks.

CREATE TABLE compose_stacks (
    id            TEXT PRIMARY KEY NOT NULL,
    project_id    TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    env_id        TEXT NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    name          TEXT NOT NULL UNIQUE COLLATE NOCASE,
    owner_id      TEXT REFERENCES users (id) ON DELETE SET NULL,
    compose_yaml  TEXT NOT NULL,
    project_name  TEXT NOT NULL,
    service_names TEXT NOT NULL CHECK (json_valid(service_names)),
    depends_edges TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(depends_edges)),
    confirmed     INTEGER NOT NULL DEFAULT 0 CHECK (confirmed IN (0, 1)),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
) STRICT;
CREATE INDEX compose_stacks_project_env ON compose_stacks (project_id, env_id, created_at);

ALTER TABLE services ADD COLUMN compose_stack_id TEXT REFERENCES compose_stacks (id) ON DELETE CASCADE;
ALTER TABLE services ADD COLUMN template_id TEXT;
ALTER TABLE services ADD COLUMN internal_only INTEGER NOT NULL DEFAULT 0 CHECK (internal_only IN (0, 1));
CREATE INDEX services_compose_stack ON services (compose_stack_id);

CREATE TABLE backup_jobs (
    id            TEXT PRIMARY KEY NOT NULL,
    service_id    TEXT NOT NULL UNIQUE REFERENCES services (id) ON DELETE CASCADE,
    template_id   TEXT,
    method        TEXT NOT NULL,
    cron          TEXT NOT NULL,
    retain_daily  INTEGER NOT NULL DEFAULT 14,
    retain_weekly INTEGER NOT NULL DEFAULT 8,
    enabled       INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
) STRICT;
