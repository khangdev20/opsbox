# Conventions for every prompt

You are working in the opsbox repository (`~/Developer/opsbox`), branch `dev`.

Before starting:
- Read `docs/STATUS.md` (create it if missing) and only the files and spec sections the prompt names. Do not read `docs/BACKEND_SPEC.md` in full; jump to the sections listed.
- `git checkout dev && git pull`.

While working:
- Stay inside the prompt's scope. If you find something else broken, note it in `docs/STATUS.md` under "Found, not fixed" instead of fixing it.
- Privilege split is strict: `serve` never touches the system; every system action is a typed `PrivOp` re-validated in `priv` (spec section 2, 7.2).
- Current, maintained crates only. Record substitutions and resolved ambiguities in `docs/DECISIONS.md` (one short entry each).
- No `unsafe` unless unavoidable, with a comment saying why.
- UI: reuse existing styles and components in `web/`. Keep plain language for users (say what happened and what to do, not internals). In `web/js/api.js`, move a function from mock to real by adding it to the `REAL` map; remove the "sample data" badge from a section once it is real.
- Product principles: good defaults over options, one screen per task, safe by default for people who do not know security.

Finishing:
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass; CI stays green.
- Commit in small logical steps; push to `origin/dev`. Never force-push. Never touch `main`.
- Rewrite `docs/STATUS.md` (keep it under 60 lines): last prompt done, what works, what is partial, "Found, not fixed", next prompt number.
- Archive the prompt you just completed: `git mv docs/prompts/NN-name.md docs/prompts/archive/` (create `archive/` if missing; if the file is not tracked yet, `git add docs/prompts` first) and mark it done in the table in `docs/prompts/README.md`. Only archive when the task is fully done; if it is partial, leave it in place and say so in `docs/STATUS.md`.
- If `docs/STATUS.md` says an earlier prompt is done but its file is still in `docs/prompts/`, archive that one too in the same commit.
- Final step, always: commit everything from this prompt in one last commit (code, tests, docs, `docs/STATUS.md`, the archive move, and `docs/prompts/` itself if it is untracked) with the message `prompt NN: <title>`, then `git push origin dev`. Run `git status` afterwards; it must be clean. Never end a session with uncommitted or unpushed work. If you stop early, still commit and push what builds and passes tests, with `prompt NN (partial): <what is done>`.
- End your reply with exact steps to verify in the Multipass VM `opsbox-vm`: repo mounted at `/home/ubuntu/opsbox`, build with `CARGO_TARGET_DIR=~/target cargo build --release`, reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`, UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>` (or the public URL once prompt 04 is done). Include one check that must be refused or fail safely.
