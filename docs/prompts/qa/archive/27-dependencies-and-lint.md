# QA 27. Dependencies and lint, fast

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. `timeout 120 cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny check` or `cargo audit` if installed (install only if quick).
2. `npm audit --omit=dev` for any JS tooling and a check that `web/vendor/` files match their recorded hashes and licenses.
3. Check `Cargo.lock` is committed and no dependency is yanked or unmaintained.
4. Fix by bumping a patch version only; anything else is 'Found, not fixed'.

## Pass when
Zero advisories that apply, or a short list with severity.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 27: Dependencies and lint, fast`, push `dev`, archive this file.
