# 12. Databases part A: link to apps (Phase 3)

Read `docs/ROADMAP.md` "Databases" part A. Builds on prompts 09 and 11.

Do:
- Reference variables: an app variable stored as `${{ postgres-main.DATABASE_URL }}`, resolved when the container starts. Also `PGHOST`, `PGPORT`, `PGUSER`, `PGPASSWORD`, `PGDATABASE`, `REDIS_URL`, `MYSQL_URL`.
- "Connect database" in the app's Variables tab; values shown as references with a link icon, never the secret.
- Apps graph draws app-to-database edges from references.
- Rotate credentials on the database page; apps that reference it are offered a redeploy.
- Database ports never published to the internet unless the user explicitly asks, with a warning.
- Add the two missing design pieces to `web/` using existing styles (Connect database, database detail with connected apps).

Verify: create Postgres, connect it to the sample app, redeploy, confirm the app reads `DATABASE_URL`; rotate and redeploy.
