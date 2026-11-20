# Status

Last prompt done: **Fix 23 — Protect opsbox configuration and service units**.
No commit, push, tag or installation attempted, as requested; changes remain on `dev`.

## Completed
- Protect `/etc/opsbox` and descendants plus `/etc/systemd/system/opsbox*`, including new units and drop-ins, from file-manager mutations.
- Check normalized paths and symlink expansion before mutations; retain descriptor/inode protection and explain that opsbox configuration/service state is protected.
- Cover write, chmod, permanent delete, move to Trash and upload replacement with two test-first regressions; ordinary reads retain their existing rules.
- Record the three-line decision, QA evidence and Linux owner checks; regenerate the index, update the fix table and archive fix 23.

## Verification
- Both new regression tests failed before implementation and now pass. All 28 other file checks pass when skipping the unchanged mode-display test.
- Full file run: 28 pass, one environment failure in `mode_strings_look_like_ls`; scratch chmod 04755 is reduced to 0755 here, independently reproduced with Python.
- `cargo check -p opsbox-priv`, all-target Clippy with warnings denied, formatting, whitespace and regenerated-index freshness checks pass.
- `timeout 120 cargo build --release` passed in 1m25s with the pre-existing rust-objcopy/libLLVM warning; Linux live-panel checks remain owner checks in docs/QA.md.

## Found, not fixed
- Existing recursive delete/Trash policy checks only selected roots, not protected descendants; deleting an ancestor such as `/etc/systemd/system` can bypass descendant protection. This broader ancestor-deletion gap needs a separate fix; do not try it on an installed host.
- This environment strips setuid bits from temporary files, preventing the existing mode-display test from passing; rerun the complete file tests on Linux.
- Local rust-objcopy cannot load libLLVM.dylib for debug stripping; release compilation emits the pre-existing warning.
- Prior Fix 19/20 helper lifecycle, Fix 21 Safe Mode filtering/TLS isolation, and Fix 22 Linux activation/credential-consumer checks remain owner follow-ups.
- Existing installed job files retain their old modes until separately reviewed/recreated (Fix 22 secures newly written files).
- Historical database-create failure still needs QA 03 VM request/journal/kernel evidence; no OOM, timeout or panic cause is claimed.
- Prior CI/release work remains pending; prior runtime target remains `v0.1.0-alpha.43`.
- Navigation maps/lookup remain lexical rather than Rust/JavaScript type resolution.

## Next
- Fix 24 — Install root-owned apps/, xfer/ and trash/ directories.
