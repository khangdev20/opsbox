# 16o. Secrets in API responses: masked by default, reveal on demand

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: every API module that returns secrets (`crates/opsbox-api/src/deploy.rs`, `databases.rs`, `backups.rs`, `tokens.rs`, `github.rs`, `alerts.rs`, settings), task and deploy log writers, `web/js/*` where secrets are shown, `docs/API.md`, `docs/THREAT_MODEL.md`. Builds on 16e (transport, cookies, headers).

Context: DevTools shows decrypted JSON, which is expected; the risk is secrets returned where they are not needed, cached, or written to logs.

## Do
- Inventory every field that is a secret: environment variable values, database passwords and connection URLs, API tokens, backup repository passwords and recovery keys, S3 and SFTP credentials, webhook URLs with tokens, GitHub tokens, TOTP secrets after enrolment, alert channel credentials. Write the list in `docs/THREAT_MODEL.md`.
- List and detail endpoints return them masked (`••••` plus the last 4 characters at most, or a `has_value: true` flag). Full values only from a dedicated reveal endpoint per secret: Owner or the role that owns it, recent re-authentication (password or 2FA within 5 minutes), `no-store`, rate limited, audited ("revealed DATABASE_URL of app web").
- Secrets that never need to be shown again (API tokens, recovery keys) are shown once at creation and only their hash is stored.
- Logs (tracing, audit, task output, deploy logs) never contain secret values; add a redaction layer for known secret values in task output (deploy builds can echo env vars).
- The UI shows masked values with a "Reveal" button that calls the reveal endpoint, hides the value again after 30 s or on navigation, and a "Copy" that does not render the value.

## Tests
- Secret field test: serialise every response type that contains a secret field and assert it is masked, using a marker value; grep test that the marker never appears in logs or task output during a deploy test.
- Reveal endpoint: requires recent re-auth, is audited, is rate limited.

## Verify on `opsbox-vm`
- Reveal a database password: requires re-auth, appears in the audit log, disappears after 30 s.
Record the results in `docs/STATUS.md`.

Out of scope: application-level encryption of JSON.
