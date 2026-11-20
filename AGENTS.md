# Working on opsbox

opsbox is a self-hosted server panel shipped as one Rust binary with an embedded JS UI.
The web process (`serve`) is unprivileged and delegates system actions to the helper.
`priv` runs as root, accepts only typed `PrivOp`s and re-validates every request.

## Read order

Read `AGENTS.md` → `docs/INDEX.md` → the one `docs/index/*.md` map for your task
→ only the source files it points to. Follow another map when crossing a boundary.
Do not read `docs/BACKEND_SPEC.md` in full; open only a relevant named section.
Generated maps omit line numbers; use `python3 scripts/where.py <symbol>` for those.
Lookup is lexical: instance-method usages are name candidates, not type resolution.

## Fast commands

Run from the repository root; replace placeholders with the crate/test/symbol name.

```sh
cargo check -p <crate>
cargo test -p <crate> <name>
cargo fmt --check
cargo clippy -p <crate> --all-targets -- -D warnings
python3 scripts/gen-index.py
python3 scripts/where.py <symbol>
```

Use a full `cargo build --release` once, at the end, when validating a runtime or
embedded UI change for installation. Documentation/tooling alone needs no release build.
Before a push, run `python3 scripts/gen-index.py --check`; stale output must fail.
Run `python3 scripts/test-gen-index.py` and `python3 scripts/test-where.py` for index tooling.

## Where do I change X?

Paths below are starting points; the last column lists maps with their neighbours.

| Change | Files / steps | Neighbour map |
|---|---|---|
| Add an API route | `crates/opsbox-api/src/lib.rs` router; handler in `crates/opsbox-api/src/<area>.rs`; `tools/generate-api.py` for API docs; tests in `crates/opsbox-api/tests/` | `docs/index/routes.md` |
| Add a `PrivOp` | `crates/opsbox-proto/src/privilege.rs` enum and request type in `crates/opsbox-proto/src/<area>.rs`; `crates/opsbox-priv/src/ops/mod.rs` dispatch and `crates/opsbox-priv/src/ops/<area>.rs` handler; `crates/opsbox-priv/src/client.rs` method; caller in `crates/opsbox-api/src/<area>.rs`; test in `crates/opsbox-priv/tests/` and API tests | `docs/index/privops.md`, `docs/index/routes.md` |
| Add a database table/column | New numbered SQL file in `crates/opsbox-store/migrations/`; register in `crates/opsbox-store/src/migrations.rs`; Store method in `crates/opsbox-store/src/<area>.rs` | `docs/index/store.md` |
| Add a setting | `crates/opsbox-store/src/settings.rs` for persisted keys; `crates/opsbox-api/src/tokens.rs` settings response; `crates/opsbox-api/src/system.rs` and `crates/opsbox-priv/src/ops/hostset.rs` for host settings; `web/js/pages/settings.js`, `web/js/api.js`; file config lives in `crates/opsbox/src/config.rs` | `docs/index/store.md`, `docs/index/routes.md`, `docs/index/symbols/opsbox.md` |
| Add a UI page | `web/js/pages/<page>.js`; route in `web/js/main.js`, nav in `web/js/shell.js`; export plus REAL entry in `web/js/api.js`; shared components in `web/js/ui.js`, examples in `web/js/kit.js` and `web/kit.html`; check at 390 px | `docs/index/ui.md` |
| Add an alert kind | `crates/opsbox-alerts/src/lib.rs` rules/state; `crates/opsbox-api/src/alerts.rs` observations; `web/js/pages/alerts.js`; `crates/opsbox-api/tests/alerts_http.rs` | `docs/index/symbols/opsbox-alerts.md`, `docs/index/routes.md` |
| Add a template | `crates/opsbox-deploy/src/templates.rs` catalogue; `crates/opsbox-api/src/templates.rs`; `crates/opsbox-priv/src/ops/templates_compose.rs`; `web/js/pages/templates.js`; `crates/opsbox-api/tests/templates_compose_http.rs` | `docs/index/routes.md`, `docs/index/privops.md`, `docs/index/ui.md` |
| Add a notification | Bell/attention aggregation in `crates/opsbox-api/src/metrics.rs`, routes in `crates/opsbox-api/src/lib.rs`; `web/js/api.js`, `web/js/shell.js`; outbound channels in `crates/opsbox-alerts/src/channels.rs` and `crates/opsbox-api/src/alerts.rs` | `docs/index/routes.md`, `docs/index/ui.md`, `docs/index/symbols/opsbox-alerts.md` |

Database creation uses templates: `api.addTemplate` → `templates::create_template_instance`
→ Store `create_service_with_exposure` (`services`, migration `0006_deploy_pipeline.sql`)
→ `deploy::enqueue_deploy` / `run_deploy_pipeline` → `PrivClient::template_start`
→ `PrivOp::TemplateStart` in `crates/opsbox-proto/src/privilege.rs` (request in
`crates/opsbox-proto/src/deploy.rs`) → `templates_compose::start_template_with`.
See the UI, routes, store and privops maps above; there is no separate database-create op.

## House rules

- Good defaults, one screen per task, safe by default for people new to security.
- Use plain language in UI text: what happened and what to do next.
- No `unsafe` without an unavoidable need and a comment explaining why; workspace forbids it.
- Only typed operations cross the privilege boundary; `serve` never performs system actions.
- Mask secrets in responses; use explicit, authenticated reveal flows when needed.
- UI must work at 390 px: no page overflow, 44 px touch targets, sheets below 640 px.
- Keep commits small; work on `dev`, never touch `main`, never force-push.
- Never commit secrets, signing keys, `.agent-rules.md`, `.cursor/` or personal orchestration files.
- Stay in scope; record unrelated problems under “Found, not fixed” in `docs/STATUS.md`.

## After a change

Run the relevant focused checks, then `python3 scripts/gen-index.py` and include the
regenerated index with the code in the same commit. Before pushing, verify with
`python3 scripts/gen-index.py --check`; CI refuses stale maps. Do not add a git hook.
Follow `docs/prompts/CONVENTIONS.md` for broader checks, status and prompt archival;
explicit session instructions about commits, pushes or releases take precedence.

## Pointers

- `docs/prompts/CONVENTIONS.md`: workflow, verification and VM install commands.
- `docs/STATUS.md`: current progress, limits and next work.
- `docs/DECISIONS.md`: engineering choices and resolved ambiguities.
- `docs/prompts/qa/README.md`: focused QA tasks.
