# Status

Last completed prompt: `docs/prompts/archive/16-databases-manage.md` (Databases part B, management; tag `v0.1.0-alpha.15`)

## What works
- PostgreSQL management lives in four typed `priv` ops (inspect, manage, pooler, upgrade) that run `psql`/`pg_dumpall` in the container; `serve` only stores sealed values. No new crates, `Cargo.lock` unchanged.
- One database + one least-privilege login per connected app (default on): `app_<app>`, `CONNECT` revoked from `PUBLIC` everywhere. Checked on real PostgreSQL 15/17: app A cannot connect to app B's database, the admin database or `postgres`.
- Database page (`#/databases/<id>`): apps and logins (rotate, remove with typed confirmation, direct/pooled), databases, users, grants, extensions (pgvector, PostGIS, pg_stat_statements, pgcrypto, uuid-ossp, pg_trgm, citext, hstore), biggest tables, connections, slow queries, PgBouncer on/off, version upgrade, data tools.
- Catalog: `postgres-pgvector`, `postgis`, `pgweb`, `adminer`; `postgres` gains 17.2 and preloads `pg_stat_statements`.
- Major upgrade: `priv` takes the backup snapshot first (no snapshot, no upgrade), dumps, starts the new major on a new volume, restores, compares databases/roles/table counts, rolls back on any failure; the old volume is kept.
- Connection strings need a fresh single-use 2FA code (wrong code: 403, counts toward lockout), are audited without the secret and sent `no-store`.
- Data tools open only at `/tools/<service>/` through the signed-in panel session (panel cookies never forwarded, tool cookies scoped, `Origin` checked on writes).
- `web/js/api.js` `REAL` map and `PAGE_USES` updated; apps.js has "Manage database", the own-login choice and "Open tool".
- fmt, `generate-api.py --check`, locked clippy `-D warnings`, locked workspace tests and locked release build pass. New tests: priv scripted-Docker (inspect, provision, rotate, refusals, pooler, upgrade validation), API end-to-end (per-app connect, 2FA reveal + replay + audit, RBAC), gateway, store.

## Partial / Found, not fixed
- Not done: the prompt gained a line after work began ("Capacity: every new or resized database and any memory-adding extension goes through `capacity::admit` from prompt 15b"). 15b is not built (no `capacity` module), so database create/resize, pooler and extension flows do not check capacity yet. Wire `capacity::admit` in when 15b lands.
- Not run against real Docker/Linux: provisioning/inspect/pooler/upgrade container steps, the PgBouncer image, pgvector/PostGIS image tags (PostGIS arm64 support unconfirmed), and the pgweb path-prefix behaviour. SQL itself was run on real PostgreSQL 15 and 17.
- The upgrade's backup step needs restic and a destination, so it is untested end to end here; no disk-space pre-check before the dump.
- Restoring a pgvector/PostGIS backup creates a plain `postgres` instance (restore infers the kind only). Extensions are per-database; a new app database does not inherit them.
- `cargo-deny` is not installed on this Mac: no new crates or licenses were added, so `deny.toml` is unchanged; run it in CI. Earlier notes still apply (pinned cargo-deny 0.18.2 vs CVSS 4, `tools/check-idle-memory.py` probes HTTP).
- Earlier real R2/MinIO/SFTP, Docker DB restore and live alert-channel verification are still pending (prompts 14, 15).

## Next
- Verify in `opsbox-vm` (steps below), then run `docs/prompts/17-alpha-release.md`. Prompt 03c is still independent and staged separately.
- VM checks: add `PostgreSQL`, add two apps, connect both, try the cross-app read (must fail), open Adminer from the Database page, take an upgrade from 16.x to 17.2 with a configured backup destination.
