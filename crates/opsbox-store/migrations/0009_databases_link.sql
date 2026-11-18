-- Prompt 12: Databases part A (link to apps, reference tracking, port exposure, credential rotation).

ALTER TABLE services ADD COLUMN public_port INTEGER NOT NULL DEFAULT 0 CHECK (public_port IN (0, 1));
ALTER TABLE services ADD COLUMN credentials_rotated_at INTEGER;

ALTER TABLE service_vars ADD COLUMN ref_expr TEXT;

CREATE TABLE service_var_refs (
    id                 TEXT PRIMARY KEY,
    service_id         TEXT NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    var_key            TEXT NOT NULL,
    target_service_id  TEXT NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    target_key         TEXT NOT NULL,
    stale              INTEGER NOT NULL DEFAULT 0 CHECK (stale IN (0, 1)),
    ref_seq            INTEGER NOT NULL DEFAULT 1,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    UNIQUE(service_id, var_key, target_service_id, target_key)
);

CREATE INDEX idx_service_var_refs_service ON service_var_refs(service_id);
CREATE INDEX idx_service_var_refs_target  ON service_var_refs(target_service_id);
