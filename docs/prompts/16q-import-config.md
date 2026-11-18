# 16q. Settings: import a configuration on a new server

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-api` (new `import` alongside the 16i export), `crates/opsbox-store`, `web/js/api.js` (`importConfig`, add to `REAL`), `web/js/pages/settings.js`, `docs/API.md`. Depends on 16i (export format `opsbox-config/1`).

Do:
- `POST /api/v1/config/import` (Owner, re-auth, on a server with no services yet, or merge mode later): dry run first that shows what will be created and what needs input (secrets, GitHub connection, DNS), then apply as a task. Never overwrite existing objects in this version.
- Upload in the panel (drop zone), passphrase field when the file has encrypted secrets, then the dry-run summary: will create, needs input (secrets, GitHub connection, DNS records), skipped (already exists). Apply runs as a task with steps; services needing secrets are created but not deployed until their secrets are entered.

Tests: round trip (export, import into an empty test store, export again, equal apart from ids and times); a wrong passphrase fails cleanly; unknown `format` refused; existing objects are never overwritten.

Verify: export from `opsbox-vm`, import on a fresh install (second VM or reinstall), services appear with "needs secret" markers and deploy after secrets are entered. Record in `docs/STATUS.md`.
