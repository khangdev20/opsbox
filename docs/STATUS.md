# Status

Last completed prompt: `docs/prompts/archive/16l-audit-ip-and-csv.md` (audit IP column, filters, CSV export). Not committed, not tagged (per session request). Prior: `16k` (display names + 2FA).

## What works (16l)
- **Store**: `AuditFilter` / `list_audit` (newest-first, cursor via `before_seq`, default page 50) and `for_each_audit` (oldest-first walk, no collect). `verify_audit_range`, `csv_cell` / `audit_csv_row` (RFC 4180 + `'` formula neutralise).
- **API**: `GET /api/v1/audit` (Admin) returns `ip` + filters (`actor`, `action` prefix, `result`, `from`/`to`, `cursor`). `GET /api/v1/audit/export.csv` streams CSV (`no-store`, `Content-Disposition: opsbox-audit-<host>-<date>.csv`), trailing `# chain:` verify. `GET /security` audit rows include `ip`. Trusted proxy without XFF is labelled `panel` in `Transport::client_ip_text`.
- **UI**: Security audit table has IP (local / panel / address), filter row, Export CSV using active filters (`getAudit`, `exportAuditCsv` in REAL).
- **Docs / tests**: `docs/API.md`, `docs/DECISIONS.md`; store unit tests for CSV/filters/paging; `security_http` integration for filters, pagination, large export, admin-only.

## Partial / Found, not fixed
- **cargo fmt / clippy / test not run** (shell unavailable this session). Please run: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.
- **Archive move**: content is in `docs/prompts/archive/16l-audit-ip-and-csv.md`; if `docs/prompts/16l-audit-ip-and-csv.md` still appears in git, `git rm` it and `git add` the archive path (shell `git mv` was unavailable).
- **Not verified on `opsbox-vm`**. Not checked at 390 px.
- Carried over: 16k / 16j / 03f / 16e / 03e / 03d pending commit/release; import is 16q.

## Next
- Commit 16l (with the pending stack if intended), push `dev`, wait for CI, tag when appropriate.
- Then `16m-process-kind.md`.
- VM check for 16l: sign in from host (tunnel → local) and VM (real IP or panel), filter by actor, Export CSV, open in a spreadsheet; `# chain: ok` at the end. Must fail safely: a viewer `GET /api/v1/audit/export.csv` returns 403.
