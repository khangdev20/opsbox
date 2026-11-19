# 18c. Project index for agents, part 3: AGENTS.md, `where` lookup and a freshness check in CI

Follow `docs/prompts/CONVENTIONS.md`: branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done, commit as `prompt NN: <title>` and `git push origin dev`. **Time box 20 minutes.** Requires 18a and 18b. No release tag unless `ci.yml` change needs a fix release (it should not).

## 1. `AGENTS.md` at the repo root (under 120 lines, hand-written)
The first file any coding agent reads. Contents:
- What opsbox is in three lines and the privilege split (web process is unprivileged; `priv` is root and accepts only typed `PrivOp`s).
- **Read order**: `AGENTS.md` -> `docs/INDEX.md` -> the one `docs/index/*.md` file for your task -> only the source files those point to. Do not read `docs/BACKEND_SPEC.md` in full.
- **Commands** that finish fast: `cargo check -p <crate>`, `cargo test -p <crate> <name>`, `cargo fmt --check`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `python3 scripts/gen-index.py`, `python3 scripts/where.py <symbol>`. Say when a full `cargo build --release` is justified (once, at the end).
- **Where do I change X?** a table: add an API route; add a `PrivOp` (proto, priv handler, client method, API caller, test); add a database table or column (migration, store method); add a setting; add a UI page (page file, route, `api.js` entry, nav, kit page, mobile check at 390 px); add an alert kind; add a template; add a notification. Each row names the files by path and says which index file lists the neighbours.
- **House rules** in one list: product principles (good defaults, one screen per task, safe by default), plain language in UI text, no `unsafe` without a comment, typed ops only across the privilege boundary, secrets masked in responses, UI usable at 390 px, small commits, never touch `main`, never force-push, never commit secrets or signing keys.
- **After a change**: run `python3 scripts/gen-index.py` and commit the regenerated index with the code.
- Pointers: `docs/prompts/CONVENTIONS.md`, `docs/STATUS.md`, `docs/DECISIONS.md`, `docs/prompts/qa/README.md`.
- `CLAUDE.md` containing only `@AGENTS.md` so Claude Code loads the same file.
- Do **not** commit `.agent-rules.md`, `.cursor/` or any personal orchestration file; if they are untracked, leave them alone.

## 2. `scripts/where.py`
`python3 scripts/where.py <name>` prints where a symbol is defined and used: definitions first (`fn`/`struct`/`enum`/`trait`/`const`/JS `function`/`export`) as `path:line  kind  signature`, then at most 20 call sites. Substring and `Type::method` forms work; output is plain text, under 60 lines, runs in under 1 second (use `rg` when present, fall back to Python `re`). This is where line numbers live, so the committed index never needs them.

## 3. CI freshness check
- Add a job step to `.github/workflows/ci.yml`: `python3 scripts/gen-index.py --check`, placed before the Rust build so it fails in seconds. The failure message says: "Run `python3 scripts/gen-index.py` and commit the result."
- Add the same check to the pre-push advice in `AGENTS.md`; do not add a git hook.

## Checks
- Cold read test: start from `AGENTS.md` only and answer without searching: where is the `PrivOp` for creating a database defined, how does the panel call it, and which table does it write? Time it; it should take a handful of file reads.
- `where.py` finds a `pub fn`, a struct and a JS function correctly.
- CI step passes on the pushed commit and fails when you push a throwaway branch that edits a route without regenerating (delete the branch afterwards, do not touch `main`).

## Done when
`AGENTS.md`, `CLAUDE.md`, `scripts/where.py` exist, CI runs the check, and `docs/STATUS.md` explains the three commands in two lines.
