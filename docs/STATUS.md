# Status

Last prompt done: **Fix 22 — Root helper umask and unit modes**.
No commit, push, tag or installation attempted, as requested; changes remain on `dev`.

## Completed
- Set `UMask=0077` in `packaging/systemd/opsbox-priv.service`.
- Write JobAdd service/timer files atomically with explicit 0600 before content, using the existing private tempfile writer.
- Audit helper file writes; use private atomic writes for deployment credentials, uninstall audit exports and adoption receipts; preserve intended public/group modes elsewhere.
- Add permission, packaging and uninstall regressions; record the three-line decision and audit/VM checks in docs/QA.md, regenerate the index and archive fix 22.

## Verification
- Test-first job regression observed both files at 0644; service regression found no UMask. Both pass; job creation is checked under child-process umask 000 without changing the test runner's umask.
- Uninstall regressions first reproduced a public audit export and stale temporary symlink overwrite; both pass after private atomic replacement.
- Seven jobs tests, eight uninstall tests and four deploy unit tests pass.
- Targeted cargo check, all-target Clippy with warnings denied, formatting, whitespace and regenerated-index freshness checks pass.
- `timeout 120 cargo build --release` passed in 1m24s with the pre-existing rust-objcopy/libLLVM warning; Linux activation and real credential-consumer checks remain owner checks in docs/QA.md.

## Found, not fixed
- Existing installed job files retain their old modes until separately reviewed/recreated; this fix secures newly written files.
- Local rust-objcopy cannot load libLLVM.dylib for debug stripping; release compilation emits the pre-existing warning.
- Prior Fix 19/20 live-helper lifecycle and Fix 21 live Safe Mode filtering/TLS isolation checks remain owner follow-ups.
- Historical database-create failure still needs QA 03 VM request/journal/kernel evidence; no OOM, timeout or panic cause is claimed.
- Prior Linux/VM and pending CI/release work remain owner follow-ups; prior runtime target remains `v0.1.0-alpha.43`.
- Navigation maps/lookup remain lexical rather than Rust/JavaScript type resolution.

## Next
- Fix 23 — Prevent file-manager modification of /etc/opsbox/config.toml.
