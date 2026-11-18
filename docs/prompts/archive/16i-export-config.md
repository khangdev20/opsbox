# 16i. Settings: export configuration

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-api/src/settings` or new `export.rs`, `crates/opsbox-store`, `web/js/api.js` (`exportConfig`, new `importConfig`, add to `REAL`), `web/js/pages/settings.js` ("Export and uninstall"), `docs/API.md`, `docs/BACKEND_SPEC.md` (new subsection).

Today: "Export configuration" is a mock that returns demo JSON; it is disabled in real mode.

Do:
- `GET /api/v1/config/export` (Owner, re-auth): a versioned JSON document (`"format": "opsbox-config/1"`) with services (source repo, branch, build settings, ports, domains, memory limits, health checks), templates and compose projects, databases (engine, version, size, extensions, which apps they link to; no data), backup jobs and destination metadata, alert rules and channels, proxy and panel settings, firewall rules opsbox manages, users and roles (no password hashes, no TOTP secrets).
- Secrets: excluded by default and listed as "needs re-entry". Option "include secrets, encrypted with a passphrase" (age or the existing crypto helper; passphrase entered in the dialog, never stored).
- Audited both ways; the file name includes host and date.

Tests: the document validates against a JSON schema checked into `docs/`; secrets never present without the passphrase option; a wrong passphrase fails to decrypt cleanly (test helper); every service field the API stores is either exported or listed as deliberately excluded.

Verify: export from `opsbox-vm`, inspect the file (no secrets without the passphrase option), the audit log shows the export. Import is prompt 16q. Record in `docs/STATUS.md`.
