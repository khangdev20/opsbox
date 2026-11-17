# 16. Databases part B: management (Phase 5b)

Read `docs/ROADMAP.md` "Databases" part B. Builds on prompts 12-14.

Do:
- One database and one least-privilege user per connected app (default on).
- Database page: databases and users, grants, extensions (pgvector, postgis), table sizes, connection count, slow queries (pg_stat_statements).
- PgBouncer as an option.
- Major version upgrade with a snapshot first.
- Show connection string behind re-entering 2FA (audited).
- Data tools: one-click pgweb or Adminer template behind opsbox auth. (Built-in SQL console is a later prompt.)

Verify: two apps on one Postgres each get their own user and cannot read each other's tables.
