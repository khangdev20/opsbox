# 16n. Remove mock leftovers and fix the capacity test fixture

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `web/js/api.js`, `web/js/pages/recovery.js`, `crates/opsbox-api/tests/capacity_http.rs` (or wherever `queued_workloads_block_database_resize_pooler_and_memory_adding_extensions` lives), `docs/DECISIONS.md`.

Do:
- Delete exported mock-only functions that no page uses: `getReclaimable`, `getAudit`, `addDestination` (check with a grep across `web/`), and their mock data if nothing else reads it.
- `simulateFailure` and the mock branch of `recoveryAction`: keep only for `?mode=mock` demos; make sure the real recovery route always sends users to the watchdog page (:8444) and never shows demo buttons in real mode.
- Add a CI check (small Node script) that lists exported `api.js` functions not in `REAL`: it fails when a page uses a function that is mock-only in real mode unless the function is on an explicit allowlist with a reason (demo-only, Desktop-only). Seed the allowlist with what remains after 16h to 16j (for example outgoing SSH keys and "Other servers", which belong to opsbox Desktop).
- Fix the `capacity_http::queued_workloads_block_database_resize_pooler_and_memory_adding_extensions` fixture issue noted in DECISIONS, or rewrite the test so it checks the intended rule without the fragile setup; remove the note when fixed.

Verify: `node` check passes in CI; `cargo test` green including the fixed test; real mode shows no "sample data" badge anywhere except the allowlisted Desktop-only spots. Record in `docs/STATUS.md`.
