# Status

Last completed prompt: `docs/prompts/archive/16q-import-config.md` (import configuration). Not committed / not pushed (per session request). Prior: `16p`.

## What works (16q)
- **`POST /api/v1/config/import/preview`**: parses `opsbox-config/1`, validates format, decrypts sealed secrets with passphrase (clean `403` on wrong passphrase). Returns `{willCreate, needsInput, skipped, secretsDecrypted}`. Never writes.
- **`POST /api/v1/config/import/apply`**: Admin/Owner, re-auth via password; `202 {task}` kind `config.import`. Creates services (no deploy when secrets missing), templates, compose projects, databases, backup destinations/jobs, alert rules/channels, firewall rules. Existing objects skipped, never overwritten.
- **Store**: `create_compose_stack_if_absent` for non-overwrite compose import.
- **UI** (Settings → Export, import and uninstall): drop zone, passphrase field, Preview summary, Apply with task progress. `importPreview` / `importConfig` in `REAL`.
- **Tests**: `import_http.rs` — round-trip export→import empty→re-export (normalized equality), wrong passphrase, unknown format, non-overwrite.
- **Docs**: `docs/API.md`, `BACKEND_SPEC` 7.8.3, `DECISIONS` 16q.

## Partial / Found, not fixed
- **Shell unavailable this session** — could not run `cargo test`, `cargo fmt`/`clippy`, or `git mv`. Before commit: run those checks; `git rm docs/prompts/16q-import-config.md && git add docs/prompts/archive/16q-import-config.md` (stub left in place because delete/`git mv` were blocked).
- **16p** still noted as uncommitted in prior STATUS; include if committing the stack.
- **Not verified on `opsbox-vm`**. Not checked at 390 px.
- Carried over: pending commit/release for 16o–16q.

## Next
- Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (esp. `import_http`).
- Then commit (only when asked): `prompt 16q: import configuration`. Do not push unless asked.
- Then `16r-security-headers.md`.
- VM check: export from `opsbox-vm`, import on a fresh install; services show needs-secret and deploy after secrets. Must fail safely: wrong passphrase → clear error, no objects created.
