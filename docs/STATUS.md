# Status

Last completed prompt: `docs/prompts/archive/16o-secrets-in-responses.md` (secrets masked by default, reveal on demand). Not committed / not pushed (per session request). Prior: `16n`.

## What works (16o)
- **Inventory** in `docs/THREAT_MODEL.md` (env vars, DB URLs, API tokens, backup/S3/SFTP, recovery key, alerts, GitHub webhook/private keys, TOTP, Docker env, proxy secret).
- **Masking**: list/detail use `••••••••` / `hasValue` / `hasCredentials` / `hasWebhookSecret`; Docker env uses `••••` + last 4.
- **Reveal**: `POST /auth/reauth`; `…/vars/{key}/reveal`, DB connection-string, backup destination/recovery-key, alerts channel, GitHub webhook secret — Admin/Deployer as appropriate, 5-minute re-auth, `no-store`, reveal rate limit, audited secret name/target.
- **One-time**: API tokens remain hash-only after create; recovery key is re-derived (not stored).
- **Log scrubbing**: `secrets::redact_line` on deploy emit + task logs; unit tests with marker value.
- **UI**: `web/js/secrets.js` (re-auth modal, 30s / nav hide, clipboard copy); Apps Variables Reveal/Copy; Database connection-string flow updated.
- **Tests**: `crates/opsbox-api/src/secrets.rs` unit tests; `tests/secrets_http.rs` (mask, re-auth, audit, rate-limit); task scrub test.

## Partial / Found, not fixed
- **Shell unavailable this session** — could not run `cargo fmt` / `clippy` / `test`, `python3 tools/generate-api.py`, or `git mv`. Please run those before commit; if `generate-api.py --check` fails, re-run it to sync the route table.
- Archive: full text is in `docs/prompts/archive/16o-secrets-in-responses.md`. The file at `docs/prompts/16o-secrets-in-responses.md` is a stub (`git mv` / shell unavailable); on commit run `git rm docs/prompts/16o-secrets-in-responses.md && git add docs/prompts/archive/16o-secrets-in-responses.md`.
- **Not verified on `opsbox-vm`**. Not checked at 390 px.
- Carried over: 16n / … pending commit/release; next prompt `16p`.

## Next
- Run workspace checks, commit 16o (and pending stack if intended), push `dev`.
- Then `16p-stable-live-lists.md`.
- VM check for 16o: reveal a DB connection string — re-auth required, audit row with secret name, value gone after 30 s. Must fail safely: reveal without re-auth → `403`; API token → `403`.
