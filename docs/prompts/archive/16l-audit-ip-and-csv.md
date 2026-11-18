# 16l. Audit log: IP column, filters and CSV export

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-store/src/audit.rs` (`AuditEntry` already has `ip`), `crates/opsbox-api/src/security.rs` (audit endpoints), `web/js/api.js`, `web/js/pages/security.js`, `docs/API.md`.

Today: the Security page's audit table has no IP column and no "Export CSV" (16d STATUS: backend gap), although `AuditEntry.ip` is stored.

Do:
- `GET /api/v1/audit` returns `ip` and supports filters: actor, action prefix, result, from and to dates, and a cursor for paging (keep the default page size).
- `GET /api/v1/audit/export.csv` (Owner): same filters, streamed, `Content-Disposition` with host and date, `no-store`, columns `seq,ts,actor,actor_kind,ip,action,target,result,detail,hash`; CSV escaping per RFC 4180; formulas neutralised (prefix `'` for cells starting with `= + - @`).
- Include a "verify chain" result for the exported range (first and last hash, ok or the first broken `seq`).
- UI: IP column (with "local" for loopback and "panel" for the proxy), filter controls above the table, "Export CSV" button using the current filters.

Tests: CSV escaping and formula neutralising; filters; streaming a large range without loading all rows; permissions (Owner only for export).

Verify: sign in from two IPs (host and VM), filter by actor, export, open in a spreadsheet. Record in `docs/STATUS.md`.
