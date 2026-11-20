# Status

Last prompt done: **Fix 25 — Internet-facing proxy can write the database**.
No commit, push, tag or installation attempted, as requested; changes remain on `dev`.

## Completed
- Limit proxy ReadWritePaths to /var/lib/opsbox/proxy under ProtectSystem=strict; hide xfer and the existing opsbox.db with InaccessiblePaths.
- Confirm proxy persistence needs only routes, adopted listeners, ACME accounts and encrypted certificates under its own state directory.
- Retain existing installer directory creation/unit copying; no duplicate unit definition needs changing.
- Add three unit-file regressions, update decisions/QA/fix table and archive fix 25; regenerate the index.

## Verification
- Strict TDD: all three unit-file tests failed before the service change, then passed; they check database ancestry, writable proxy state, strict protection and both hidden paths.
- Requested full proxy tests: nine library tests pass; four existing socket-bind tests fail with sandbox Operation not permitted, stopping Cargo before integration tests. All nine socket-free library tests pass separately.
- Focused all-target Clippy with warnings denied, formatting, whitespace, index regeneration and freshness checks pass; generated maps are unchanged.
- `timeout 120 cargo build --release` passed in 1m22s with the pre-existing rust-objcopy/libLLVM debug-stripping warning.
- Linux service namespace enforcement, fresh installation and real certificate issue/renew/reload remain owner checks in docs/QA.md.

## Limits and next prompt
- Database exclusion permits a missing file so fresh proxy startup precedes panel SQLite creation. Parent stays read-only; restart the proxy after creation to hide the new database from reads too.
- Next: Fix 26 — give the proxy its own system user, then Fix 27 helper operation authorization. This mount policy does not isolate the shared UID or helper socket.

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
- Fix 25 — Restrict the internet-facing proxy's writable paths.
