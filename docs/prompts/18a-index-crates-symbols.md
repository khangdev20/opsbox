# 18a. Project index for agents, part 1: generator, crate map and symbol map

Follow `docs/prompts/CONVENTIONS.md`: branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, commit as `prompt NN: <title>` and `git push origin dev`. **Time box 25 minutes**; if you pass 20 minutes, commit what works and note the rest in `docs/STATUS.md`. No release tag: scripts and docs only.

## Why
The repo is about 120k lines of Rust in 18 crates plus a vanilla-JS panel. Agents lose most of their budget searching before they change anything. Give them a small, always-correct map: they read it first, then open only the files it points to. The map is **generated**, never hand-written, so it cannot rot.

## Rules for the generator
- One script, `scripts/gen-index.py`, **Python 3 standard library only**, no network, runs in under 10 seconds, output is deterministic (sorted, no timestamps, no commit hash, **no line numbers**: they churn on every edit; line numbers come from `scripts/where.py` in 18c).
- `python3 scripts/gen-index.py` writes the files; `python3 scripts/gen-index.py --check` writes nothing, prints a unified diff of what is stale and exits 1 (used by CI in 18c).
- Parse with regex and light scanning, not a Rust parser. Skip `target/`, `tests/` bodies, `node_modules`, `web/vendor/`, generated files and anything in `.gitignore`.
- Never write secrets, tokens or absolute home paths into the output.

## Output of this prompt
1. `docs/INDEX.md` (target under 250 lines, hand-written header plus generated block between `<!-- gen:start -->` and `<!-- gen:end -->` markers; the script only rewrites the block):
   - Layout: top-level folders and one line each (`crates/`, `web/`, `site/`, `design/`, `docs/`, `scripts/`, `packaging/`, `tools/`).
   - **Crate table**: name, one-line purpose (first `//!` line of `lib.rs` or `main.rs`, else `description` in its `Cargo.toml`, else `TODO: add a //! line`), internal dependencies (opsbox crates only), non-test LOC, entry file.
   - **Dependency graph** as indented text (who depends on whom), so an agent knows what a change can break.
   - **Runtime roles**: `serve`, `priv`, `proxy`, `watchdog`, and which crate each is started from.
2. `docs/index/symbols/<crate>.md`, one file per crate: for each `pub` item (`fn`, `struct`, `enum`, `trait`, `type`, `const`, `mod`) one line `path/to/file.rs: kind Name  summary`, where summary is the first line of its `///` comment or empty. Group by file, sorted. Skip `pub(crate)` and private items. Cap each file at 400 lines; if a crate exceeds it, keep the top-level `lib.rs` re-exports and the items that have a doc comment, and add a `(truncated, N more)` line.
3. Add `//!` one-liners where a crate is missing one (the generator prints a TODO for each). Do this only for crates you open anyway; do not rewrite code.

## Checks
- Run it twice: second run produces no diff. Change one file and confirm `--check` fails, then regenerate.
- Open `docs/INDEX.md` as an agent would: from it alone, name the crate that handles backups, the one that owns the Docker client, and the one that runs as root. Fix the purposes until the answer is obvious.
- Size: whole `docs/index/` plus `INDEX.md` under 500 KB; `INDEX.md` alone under 15 KB.

## Done when
`docs/INDEX.md` and `docs/index/symbols/*.md` exist, regenerate identically, and `--check` passes on a clean tree. Record the command in `docs/STATUS.md`.
