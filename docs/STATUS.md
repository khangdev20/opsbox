# Status

Last prompt completed: **Fix 28 — Verify updates inside the root helper**.
No commit, push, tag or installation attempted; changes remain on `dev`, as requested.

## Completed
- Require signed-archive SHA-256/minisign proof in SelfUpdateInstall; update both panel and CLI callers together.
- Read the descriptor-pinned xfer archive once, verify with the embedded public key, then extract the requested version/target's regular binary.
- Execute selftest and install from one helper-owned 0700 temporary copy; close its writable fd before execution and clean it up automatically.
- Preserve slot state, schema checks, restart scheduling and rollback.
- Add marker-script refusals and private-copy/rollback regressions; update decisions/QA, archive fix 28 and regenerate navigation maps.

## Verification
- TDD: replaying the old validation/selftest sequence failed both marker tests because the untrusted scripts ran.
- All three focused helper regressions and all 21 opsbox-update tests pass, including staging replacement, permissions, installation and rollback.
- cargo check --workspace passes, including both update callers.
- Full helper suite: 236 pass, three existing environment failures (socket bind denied, stripped setuid, denied uptime sysctl).
- Full API suite: 72 pass, four TLS socket-bind denials. Both full suites stop before integration tests; Linux owner steps are in docs/QA.md.
- Formatting and all-target Clippy for changed crates with warnings denied pass; whitespace and generated-index freshness checks pass.
- Release build passes (1m26s), with the existing rust-objcopy/libLLVM debug-stripping warning.

## Limits and next prompt
- Live Linux signed updates through panel/CLI, previous-alpha upgrade, and systemd health-check rollback remain owner work.
- Next: Fix 29 — derive audit actors from peer credentials.

## Found, not fixed
- Existing recursive delete/Trash policy checks only selected roots, not protected descendants; deleting an ancestor such as `/etc/systemd/system` can bypass descendant protection. This broader ancestor-deletion gap needs a separate fix; do not try it on an installed host.
- This environment strips setuid bits from temporary files, preventing the existing mode-display test from passing; rerun the complete file tests on Linux.
- Local rust-objcopy cannot load libLLVM.dylib for debug stripping; release compilation emits the pre-existing warning.
- Prior Fix 19/20 helper lifecycle, Fix 21 Safe Mode filtering/TLS isolation, and Fix 22 Linux activation/credential-consumer checks remain owner follow-ups.
- Existing installed job files retain their old modes until separately reviewed/recreated (Fix 22 secures newly written files).
- Historical database-create failure still needs QA 03 VM request/journal/kernel evidence; no OOM, timeout or panic cause is claimed.
- Prior Fix 24 live installation/upgrade, root ownership and managed-directory checks remain owner follow-ups.
- Prior CI/release work remains pending; prior runtime target remains `v0.1.0-alpha.43`.
- Navigation maps/lookup remain lexical rather than Rust/JavaScript type resolution.


## Next
- Fix 29 — Derive audit actors from peer credentials.
