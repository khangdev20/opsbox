# Status

Last prompt done: **Fix 24 — Root-owned apps/, xfer/ and trash/ directories**.
No commit, push, tag or installation attempted, as requested; changes remain on `dev`.

## Completed
- Finish the started managed-directory hardening; retain the portable `Mode::from_raw_mode(mode as _)` conversion.
- Install/migrate apps as root:opsbox 0750, xfer as root:opsbox 0770 and trash as root:root 0700; refuse directory symlinks and protect legacy migration against parent renames.
- Prepare configured directories at root-helper startup after binary upgrades and before self-update activation; update downloading requires an existing real xfer directory.
- Use checked no-follow directory descriptors for helper roots/children, uploads, downloads, Trash moves/purges and staged-update verification; API opens/validates xfer without creating or chmodding it.
- Preserve service-group access under UMask=0077 and stage root-owned downloads with group-readable 0640 permissions.
- Update decisions, QA evidence/owner checks and the fix table; archive fix 24 and regenerate the index.

## Verification
- API symlink regression failed before the change by altering the target mode, then passed; five managed helper regressions pass, including unchanged symlink targets and fresh/migration preparation.
- Required two-crate cargo check and both requested focused test commands pass; all 30 other file tests pass when skipping the known environment-sensitive mode-display test.
- Installer scratch harness passes fresh install, legacy-mode migration and all three symlink refusals; actual shell control flow runs with recorded owner/group arguments and real mkdir/chmod.
- Focused all-target Clippy, formatting, shell syntax and whitespace checks pass; generated maps refreshed and freshness check passes.
- `timeout 120 cargo build --release` passed in 1m29s with the pre-existing rust-objcopy/libLLVM warning. Linux installation/upgrade and real root chown/systemd remain owner checks in docs/QA.md.

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
- Fix 25 — Restrict the internet-facing proxy's writable paths.
