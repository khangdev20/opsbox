# Roadmap

## Done: UI on mock data (13 + 5 screens)
Overview, Apps (graph, deploys, rollback, variables), New service (GitHub, templates, Compose), Containers, System (updates, services, logs, jobs, firewall), Backups, Alerts, Cleanup, Terminal, Files (Monaco), Security, Access (SSH keys, Linux users), Settings, Login, Setup, Safe mode. Dark and light themes.

## Decided: the web panel is the product
opsbox = agent + web panel on one instance, on by default, for small users who want plug-and-play. opsbox Desktop (many instances, apps across instances) is a separate, later product. See docs/ARCHITECTURE.md.

## Done: backend phase 0, skeleton
Cargo workspace, `opsbox-proto`, SQLite store with migrations and a hash-chained audit log, the `priv` socket server with peer-credential checks, the axum server with the embedded UI, `status`, install script and systemd units, CI. At that milestone only `GET /api/v1/host` was real; see `docs/DECISIONS.md` and `docs/CONFIG.md`.

## Implemented: backend Phase 1, own the server
Auth/setup/TOTP, sessions/lockout/audit, real metrics, system updates and systemd D-Bus, journal/jobs/firewall/emergency SSH/reboot, files/Monaco, PTY/xterm, cleanup, security, access and settings/scoped tokens. Hardening includes rate/body/deadline/stream limits, doctor, generated route documentation and an idle-memory CI budget. See [PHASE1_STATUS.md](PHASE1_STATUS.md) for evidence and remaining validation.

## Next phases
1b. Close Phase 1 safety gaps: descriptor-relative file access (no check-then-open race), console notice through priv, wait for systemd job results, firewalld rules with a source.
1c. Public first-run: panel on the public address :8443 with a self-signed certificate until claimed, setup token gate, `--local-only` install flag (see ARCHITECTURE.md "Reaching the panel without SSH").
2. Watchdog, safe mode, signed self-update/rollback.
3. Containers, GitHub deployments, templates and Compose.
   - Includes linking databases to apps (see "Databases" below, part A).
   - Deployer file access and API tokens scoped per service (deferred from Phase 1).
4. Proxy, panel domains and TLS (panel domain first: it is how small users secure the panel).
5. Backups and alerts.
5b. Databases (see below, part B).

MVP = phases 1 to 5b. After that, as a separate product:
- opsbox Desktop: many instances, apps and microservices across instances (on an existing orchestrator), WireGuard, pairing, network rules.

## Databases (decided 2026-11-17, not started)
Goal: create and configure databases from the UI and wire them into apps without copying credentials. Users never need to SSH in or paste passwords.

**Part A, in Phase 3**
- "+ Database": Postgres (then Redis, MySQL) with version, volume, generated credentials, private-network only, daily backup on by default.
- Reference variables instead of copied values: an app's variable is stored as `DATABASE_URL = ${{ postgres-main.DATABASE_URL }}` and resolved when the container starts. Also `PGHOST`, `PGPORT`, `PGUSER`, `PGPASSWORD`, `PGDATABASE`, `REDIS_URL`, `MYSQL_URL`.
- "Connect database" in an app's Variables tab; the Apps graph draws the link automatically.
- Credential rotation in one place; apps that use it are offered a redeploy.
- Connection string uses the internal name (`postgres-main:5432`); the database port is never published to the internet unless the user explicitly asks, with a warning.

**Part B, Phase 5b (after backups exist)**
- One database and one least-privilege user per connected app (default on).
- Databases and users list, grants, extensions (pgvector, postgis), connection pooler (PgBouncer).
- Table sizes, connection count, slow queries.
- Major version upgrade with a snapshot first.
- Show connection string behind re-entering 2FA (audited); temporary tunnel for TablePlus/DBeaver through the desktop app.
- Data tools: first a one-click pgweb/Adminer template behind opsbox auth; later a small built-in SQL console (Monaco, read-only by default, writes audited).

**Out of scope**
Supabase-style backend services (end-user auth, auto-generated REST, realtime, storage). Users who want them can run self-hosted Supabase as a Compose template.

Design still needed: "Connect database" in the Variables tab, database detail page (connected apps, users, extensions, sizes, slow queries, SQL console entry).
Spec still needed: reference variables, rotation and per-app users in docs/BACKEND_SPEC.md (sections 7.5, 7.6).

## Before trusting it with company data
Independent security review, SECURITY.md, threat model, signed releases.
