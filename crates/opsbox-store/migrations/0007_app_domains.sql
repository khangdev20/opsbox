-- Phase 4 (prompt 10): multiple custom domains per service, DNS state, redirects,
-- Force HTTPS, and HSTS policy.

CREATE TABLE service_domains (
    id             TEXT PRIMARY KEY NOT NULL,
    service_id     TEXT NOT NULL REFERENCES services (id) ON DELETE CASCADE,
    domain         TEXT NOT NULL UNIQUE COLLATE NOCASE,
    is_primary     INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    redirect_to    TEXT,
    force_https    INTEGER NOT NULL DEFAULT 1 CHECK (force_https IN (0, 1)),
    hsts           INTEGER NOT NULL DEFAULT 1 CHECK (hsts IN (0, 1)),
    dns_status     TEXT NOT NULL DEFAULT 'pending' CHECK (dns_status IN ('pending', 'ok')),
    dns_checked_at INTEGER,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
) STRICT;

CREATE INDEX service_domains_service
    ON service_domains (service_id, is_primary DESC, created_at ASC);

-- Backfill single domains created under prompt 09 so existing services retain
-- routing and appear in the Domains & SSL tab.
INSERT OR IGNORE INTO service_domains (
    id, service_id, domain, is_primary, redirect_to, force_https, hsts,
    dns_status, dns_checked_at, created_at, updated_at
)
SELECT
    id || '-dom', id, lower(trim(domain)), 1, NULL, 1, 1,
    'pending', NULL, created_at, updated_at
FROM services
WHERE domain IS NOT NULL AND trim(domain) != '';
