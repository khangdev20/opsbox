# Status

Last completed prompt: `docs/prompts/archive/16m-process-kind.md` (top process kind from cgroup). Not committed, not tagged (per session request). Prior: `16l` (audit IP/filters/CSV).

## What works (16m)
- **Metrics**: After top-N sort, `opsbox-metrics` reads `/proc/<pid>/cgroup` (cgroup v2), classifies `container` / `system` / `user` / `unknown`, maps Docker ids via the refreshed container list (`name`, `service?`), and caches by PID until exit. `ProcView` carries `kind`, `container`, `unit`.
- **API**: `GET /metrics` and `/stream/metrics` process entries include `kind` plus optional `container` / `unit` (`docs/API.md`).
- **UI**: Overview `procRow` subline `container <name> · pid …` / `system <unit> · pid …`; row links to `#/apps?s=…` when `service` is set, else `#/containers`.
- **Tests / docs**: Unit fixtures in `proc_kind` (Docker systemd + cgroupfs, system, user, v1 unknown, cache); API view unit tests; metrics HTTP asserts `kind`. `docs/DECISIONS.md` 16m entry.

## Partial / Found, not fixed
- **cargo fmt / clippy / test not run this session** (shell unavailable). Please run: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`.
- **Archive**: full text is in `docs/prompts/archive/16m-process-kind.md`. The file at `docs/prompts/16m-process-kind.md` is a stub (shell/`git mv` unavailable); on commit run `git rm docs/prompts/16m-process-kind.md && git add docs/prompts/archive/16m-process-kind.md`.
- **Not verified on `opsbox-vm`**. Not checked at 390 px.
- Carried over: 16l / 16k / 16j / … pending commit/release; next prompt `16n`.

## Next
- Commit 16m (with the pending stack if intended), push `dev`, wait for CI, tag when appropriate.
- Then `16n-mock-leftovers-and-fixture.md`.
- VM check for 16m: Overview top processes show greenglish container names; `dockerd` shows `system docker.service`. Must fail safely: a process with unreadable/missing cgroup still appears with `kind: unknown` (or `user`) and no crash.
