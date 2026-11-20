# Status

Last prompt done: **Fix 26 — Give the proxy its own system user and socket**.
No commit, push, tag or installation attempted; changes remain on `dev`, as requested.

## Completed
- Run proxy as opsbox-proxy:opsbox-proxy; add a separate root:opsbox-proxy 0660 helper socket at /run/opsbox/priv-proxy.sock, configurable in [priv].
- Match kernel peer UID to the accepting listener before parsing; reject shared UID/path configurations and retain socket origin in tracing; clean up both sockets on shutdown.
- Share idempotent installer/root-helper-startup migration for accounts and encrypted state; refresh the installed proxy unit on binary-only updates from older helpers.
- Keep private proxy state and root sticky parent traversal without sharing the panel group; retain authenticated panel access to the proxy control socket.
- Add socket/UID, service-user, migration and configuration regressions; update decisions/QA/config docs, archive fix 26 and regenerate maps.

## Verification
- Strict TDD failures observed before implementation for service identity, new socket API, migration block and CLI socket routing.
- All 18 configuration tests, four service-unit tests, migration harness and shared-UID refusal pass.
- New two-socket integration test compiles but cannot bind in this sandbox; Linux owner run remains required.
- Requested cargo check --workspace passes. Full priv tests: 228 pass, three existing environment failures (socket denial, stripped setuid, denied uptime sysctl). Full proxy tests: nine pass, four existing socket-denial failures. Cargo stops before integration suites.
- Focused all-target Clippy with warnings denied, formatting and shell syntax pass.
- Final release build passes (1m21s), with the existing rust-objcopy/libLLVM debug-stripping warning. Index regeneration/freshness, shell syntax and whitespace checks pass.
- Exact fresh/reinstall/binary-update, ownership, ACME and cross-socket refusal checks are in docs/QA.md. Rollback to pre-dual-socket binaries needs matching service/state restoration.

## Limits and next prompt
- Per-operation authorization on the proxy helper connection is Fix 27; this change separates identities and listeners.
- Next: Fix 27 — helper operation authorization by caller role.

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
- Fix 27 — Authorize helper operations by caller role.
