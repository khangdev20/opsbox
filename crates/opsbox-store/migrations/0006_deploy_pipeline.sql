-- Phase 3 (prompt 09): projects, environments, services, variables, deploys, logs,
-- and service-scoped API tokens (`deploy:<service>`).

CREATE TABLE projects (
    id         TEXT PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    created_at INTEGER NOT NULL
) STRICT;

CREATE TABLE environments (
    id         TEXT PRIMARY KEY NOT NULL,
    project_id TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (project_id, name)
) STRICT;

CREATE TABLE services (
    id               TEXT PRIMARY KEY NOT NULL,
    project_id       TEXT NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    env_id           TEXT NOT NULL REFERENCES environments (id) ON DELETE CASCADE,
    name             TEXT NOT NULL UNIQUE COLLATE NOCASE,
    kind             TEXT NOT NULL DEFAULT 'app' CHECK (kind IN ('app', 'db', 'tool', 'compose')),
    owner_id         TEXT REFERENCES users (id) ON DELETE SET NULL,
    source_kind      TEXT NOT NULL CHECK (source_kind IN ('github', 'deploy_key', 'git_url', 'docker_image')),
    repo             TEXT NOT NULL,
    branch           TEXT NOT NULL,
    build_method     TEXT NOT NULL,
    dockerfile_path  TEXT,
    start_cmd        TEXT,
    port             INTEGER NOT NULL CHECK (port >= 0 AND port <= 65535),
    health_path      TEXT NOT NULL DEFAULT '/',
    domain           TEXT,
    image_ref        TEXT,
    deploy_key_id    TEXT REFERENCES deploy_keys (id) ON DELETE SET NULL,
    auto_deploy      INTEGER NOT NULL DEFAULT 1 CHECK (auto_deploy IN (0, 1)),
    memory_limit_mb  INTEGER,
    cpu_limit_millis INTEGER,
    state            TEXT NOT NULL DEFAULT 'idle' CHECK (state IN ('idle', 'building', 'active', 'failed')),
    active_deploy_id TEXT,
    active_container TEXT,
    active_host_port INTEGER,
    graph_x          INTEGER NOT NULL DEFAULT 0,
    graph_y          INTEGER NOT NULL DEFAULT 0,
    note             TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
) STRICT;
CREATE INDEX services_project_env ON services (project_id, env_id, created_at);
CREATE INDEX services_repo_branch ON services (repo, branch);

CREATE TABLE service_vars (
    id         TEXT PRIMARY KEY NOT NULL,
    service_id TEXT NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    value_enc  TEXT NOT NULL,
    is_secret  INTEGER NOT NULL CHECK (is_secret IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (service_id, key)
) STRICT;
CREATE INDEX service_vars_service ON service_vars (service_id, key);

CREATE TABLE deploys (
    id            TEXT PRIMARY KEY NOT NULL,
    service_id    TEXT NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    commit_sha    TEXT,
    message       TEXT NOT NULL,
    trigger_kind  TEXT NOT NULL CHECK (trigger_kind IN ('manual', 'webhook', 'token', 'rollback')),
    triggered_by  TEXT NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('building', 'live', 'replaced', 'failed')),
    image_ref     TEXT,
    container_id  TEXT,
    error_message TEXT,
    started_at    INTEGER NOT NULL,
    finished_at   INTEGER
) STRICT;
CREATE INDEX deploys_service_started ON deploys (service_id, started_at DESC, id DESC);

CREATE TABLE deploy_logs (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    deploy_id TEXT NOT NULL REFERENCES deploys (id) ON DELETE CASCADE,
    ts        INTEGER NOT NULL,
    line      TEXT NOT NULL
) STRICT;
CREATE INDEX deploy_logs_deploy ON deploy_logs (deploy_id, id ASC);

-- Extend `api_tokens` CHECK constraint to permit per-service scopes `deploy:<service>`.
CREATE TABLE api_tokens_new (
    id           TEXT PRIMARY KEY NOT NULL,
    name         TEXT NOT NULL UNIQUE COLLATE NOCASE,
    token_hash   TEXT NOT NULL UNIQUE,
    scope        TEXT NOT NULL CHECK (
        scope IN ('read', 'deploy', 'full')
        OR (scope LIKE 'deploy:%' AND length(scope) > 7)
    ),
    created_by   TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    last_used_at INTEGER
) STRICT;

INSERT INTO api_tokens_new (id, name, token_hash, scope, created_by, created_at, last_used_at)
SELECT id, name, token_hash, scope, created_by, created_at, last_used_at
FROM api_tokens;

DROP TABLE api_tokens;
ALTER TABLE api_tokens_new RENAME TO api_tokens;
