# Status

Last prompt done: **18b — Project index: routes, privileged operations, database and UI maps**.
Tooling/documentation only; no release tag. Prior runtime target remains `v0.1.0-alpha.43`, pending CI/release completion.

## Completed
- Extended the Python 3 standard-library generator with `routes.md`, `privops.md`, `store.md` and `ui.md`; linked them from `docs/INDEX.md` alongside the 18a crate/symbol maps.
- Routes include nested prefixes, methods, WebSocket upgrades, signature extractors, token refusals, capacity patterns, API call links and informational API.md drift.
- PrivOp rows include typed/inline requests, grouped dispatch, streaming interception, client delegation/timeouts and variants without dedicated wiring.
- Store reconstruction uses in-memory SQLite migrations, including ALTERs, table rebuilds, explicit/implicit indexes, creation migrations, Store methods/delegation and literal settings reads.
- UI maps cover actual page references, router hashes, global/vendor CSS, exported API functions/auth methods, normalized endpoints, mock backlog and shared helpers. Mode-independent local utilities are REAL with no HTTP endpoint.
- Marked 18b done and archived its prompt. No commit, push or release action attempted, as requested.

## Acceptance example — add a field to the database create form
Read only the generated index files for this trace; no repository source search was used.
1. `ui.md` Pages: `web/js/pages/templates.js`, exported `view`, hash `#/apps/new/templates` (optional trailing slash), CSS `web/css/app.css`, calls `api.addTemplate`.
2. `ui.md` API layer: `addTemplate` in `web/js/api.js` sends `POST /api/v1/templates/{id}/instances`.
3. `routes.md` templates: this URL reaches `templates::create_template_instance` in `crates/opsbox-api/src/templates.rs`, with `Deployer` and `Client` extractors; API call links name `CreateTemplateInstanceBody` and `Store::create_service_with_exposure`.
4. Follow the same call links through `deploy::enqueue_deploy` to `deploy::run_deploy_pipeline` in `crates/opsbox-api/src/deploy.rs`, which calls `PrivClient::template_start`.
5. `privops.md`: `template_start` sends `PrivOp::TemplateStart(crate::deploy::TemplateStartRequest)`, with a 300-second timeout; helper handler `templates_compose::start_template_with` is in `crates/opsbox-priv/src/ops/templates_compose.rs`.
6. `store.md`: `create_service_with_exposure` in `crates/opsbox-store/src/deploy.rs` touches `services`, created by `crates/opsbox-store/migrations/0006_deploy_pipeline.sql`; the row lists current column types/indexes. A persisted field needs a new migration and this Store path as well as the form, API body mapping and typed helper request when relevant.

## Verification
- All 17 tests pass: `PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-gen-index.py`; new regressions cover each map, nesting, auth/WS, grouped/stream dispatch, timeouts, SQL ALTER/rebuild/rename, settings, JS templates/regex, CSS and freshness.
- Two repository generations preserved bytes and mtimes, taking 1.067 and 1.082 seconds; `python3 scripts/gen-index.py --check` passes.
- A temporary real router entry made `--check` exit 1 without writing; regenerating passed, then the entry was removed, regenerated and checked successfully. Source restored exactly.
- `docs/index/` totals 323,393 bytes; `docs/INDEX.md` is 7,919 bytes. Generated output has no source line numbers.
- Source spot checks: routes readyz/template-create/upload/metrics-stream/tools-gateway; PrivOps Ping/TemplateStart/BackupRun/PtyOpen/ProxyNetwork; store settings/users/services/database_settings/backup_jobs; UI templates/database/files/terminal/system plus addTemplate/scanCleanup/runBackup/streamDeployLog/auth.login API rows. Corrected temporary rebuild-table leakage and vendor CSS paths.
- `git diff --check` passes. Cargo fmt, Clippy and workspace tests were attempted but Cargo is unavailable. Git pull on dev was blocked by read-only `.git/FETCH_HEAD`.

## Found, not fixed
- Maps are lexical navigation, not Rust/JavaScript type resolution: method-name links can be candidates; arbitrary router expressions fail explicitly. Computed settings keys and table names are omitted; metrics_* SQL selected by formatting has no literal table-to-method association. API drift compares explicit method/path mentions, leaving prose shorthand unexpanded.
- Historical database-create failure still requires QA 03 VM request/journal/kernel evidence; no runtime OOM, timeout or panic cause is claimed.
- Prior Linux/VM, browser smoke tests and pending Git/CI/release work remain owner follow-ups (QA 03/04/09/11/13/17/18/22/24/25/26).

## Next
- **18c**: agent instructions, lookup and CI freshness checking.
- Run the Python checks and pending Rust/VM verification on `opsbox-vm`; stale index output must be refused by `--check` without writes.
