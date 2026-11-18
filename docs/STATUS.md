# Status

Last completed prompt: `docs/prompts/archive/16n-mock-leftovers-and-fixture.md` (mock leftovers + capacity fixture). Not committed, not tagged (per session request). Prior: `16m` (process kind).

## What works (16n)
- **Mock cleanup**: Removed unused `getReclaimable` and `addDestination` from `web/js/api.js`. Kept `getAudit` (used by Security, already in `REAL`). `simulateFailure` / `recoveryAction` refuse in http mode; recovery page already shows only watchdog :8444 in real mode (demo buttons under `?mode=mock` only).
- **CI**: `tools/check-mock-api.mjs` fails on page → mock-only `api.js` calls unless allowlisted (demo Safe Mode; Desktop outgoing keys / mock terminal hosts). Wired in `.github/workflows/ci.yml`.
- **Capacity fixture**: `queued_workloads_block_database_resize_pooler_and_memory_adding_extensions` uses one Building app at 450 MB on a 1 GB profile so pooler/resize/extensions get structured capacity 409s; removed fragile Building-Postgres setup and the DECISIONS note.

## Partial / Found, not fixed
- **cargo fmt / clippy / test not run this session** (shell was unavailable for most of the session). Please run: `node tools/check-mock-api.mjs` and `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo test -p opsbox-api --test capacity_http queued_workloads_block_database_resize_pooler_and_memory_adding_extensions` plus full workspace check before commit.
- **Archive**: full text is in `docs/prompts/archive/16n-mock-leftovers-and-fixture.md`. The file at `docs/prompts/16n-mock-leftovers-and-fixture.md` is a stub (`git rm` / shell `git mv` unavailable this session); on commit run `git rm docs/prompts/16n-mock-leftovers-and-fixture.md && git add docs/prompts/archive/16n-mock-leftovers-and-fixture.md`.
- **Not verified on `opsbox-vm`**. Not checked at 390 px.
- Carried over: 16m / 16l / … pending commit/release; next prompt `16o`.

## Next
- Commit 16n (with the pending stack if intended), push `dev`, wait for CI, tag when appropriate.
- Then `16o-secrets-in-responses.md`.
- VM check for 16n: open panel in real mode — Recovery shows Open Safe Mode (:8444), no Simulate failure; no unexpected “sample data” badge. Must fail safely: `?mode=http` calling simulateFailure/recoveryAction returns an error (no demo mutation).
