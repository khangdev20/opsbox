# Status

Last prompt done: **18c — Project index: AGENTS.md, lookup and CI freshness**.
`python3 scripts/gen-index.py` refreshes the maps; `python3 scripts/where.py <symbol>` prints definitions and up to 20 usages.
`python3 scripts/gen-index.py --check` refuses stale maps without writes; CI runs it before Rust setup/builds and explains how to regenerate.

## Completed
- Added the 80-line hand-written `AGENTS.md`: privilege split, read order, fast commands, change-location table, house rules, database-create trace and after-change/pre-push checks. `CLAUDE.md` contains only `@AGENTS.md`.
- Added stdlib Rust/JS lookup with definitions first, literal substrings, qualified impl/module lookup, at most 59 output lines, rg acceleration and a Python re fallback respecting repository ignore rules. Instance-method usages are lexical candidates.
- CI now checks index freshness immediately after checkout and runs both index tooling test scripts.
- Marked 18c done and archived its prompt. No commit, push, throwaway remote branch or release action attempted, as requested. No runtime change or release tag; prior runtime target remains `v0.1.0-alpha.43`, pending CI/release completion.

## Verification
- All 23 Python tests pass: `PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-gen-index.py` (17), then `PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-where.py` (6).
- Repository lookups for a qualified pub fn, struct/enum and JS export work; six CLI measurements, including broad substrings, took 0.067–0.333 seconds. Tests cover line numbers, signatures, bounds, ignores, symlinks, comments/literals, fallback parity, missing names and invalid arguments.
- Regeneration preserves the existing generated maps; `python3 scripts/gen-index.py --check` passes. The exact CI shell step rejected a temporary real route without regeneration, printed “Run `python3 scripts/gen-index.py` and commit the result.” and preserved index bytes/mtimes. Restored source byte-for-byte and rechecked successfully.
- Cold navigation trace used six documentation reads (AGENTS, INDEX, UI, routes, privops, store), without source search: `api.addTemplate` → POST template instances → `templates::create_template_instance` → Store `create_service_with_exposure` → `services` (migration 0006); deploy pipeline → `PrivClient::template_start` → `PrivOp::TemplateStart` in `crates/opsbox-proto/src/privilege.rs` → `templates_compose::start_template_with`.
- `git diff --check` passes. Cargo fmt, Clippy and workspace tests were attempted but Cargo is unavailable. Pull on `dev` was blocked by read-only `.git/FETCH_HEAD`; hosted CI and VM checks remain unrun.

## Found, not fixed
- Navigation maps and lookup are lexical, not Rust/JavaScript type resolution. Arbitrary router expressions fail explicitly; computed settings/table names and dynamically formatted SQL associations remain outside the maps.
- Historical database-create failure still requires QA 03 VM request/journal/kernel evidence; no runtime OOM, timeout or panic cause is claimed.
- Prior Linux/VM, browser smoke tests and pending Git/CI/release work remain owner follow-ups (QA 03/04/09/11/13/17/18/22/24/25/26).

## Next
- No remaining 18-series implementation prompt; continue focused QA from `docs/prompts/qa/README.md`.
- On `opsbox-vm`, run the Python checks, pending Rust checks and installation verification; stale index output must be refused by `--check` without writes.
