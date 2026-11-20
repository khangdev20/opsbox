# Status

Last prompt done: **Fix 21 — Safe Mode exposure and TLS key isolation**.
No commit, push, tag or installation attempted, as requested; changes remain on `dev`.

## Completed
- Add exact-IP `[watchdog] allow_ips` filtering immediately after accept, before TLS; empty lists retain open recovery and local-only binding still follows the panel.
- Use a separate root-owned 0600 watchdog TLS key under `/etc/opsbox/watchdog`; never chown it to opsbox or fall back to panel/data-directory credentials.
- Show allowlist state and watchdog certificate SHA-256 in status and installer output; root status can use a fresh private snapshot when its source is excluded.
- Add admission, key, config, status and live-connection regressions; document configuration and owner checks, record decision, mark fix 21 done and archive its prompt.

## Verification
- Test-first key, config and status regressions reproduced the old behavior; source-filter test initially found the admission function missing.
- 24 socket-free watchdog tests, 17 config tests, eight socket-free status tests and the local-only config-edit test pass.
- Three watchdog socket tests are blocked by sandbox PermissionDenied (including the new close-before-TLS integration test); live root ownership, network filtering and installation remain owner checks in docs/QA.md.
- Targeted all-target Clippy with warnings denied, formatting, shell syntax, whitespace and regenerated-index checks pass.
- `timeout 120 cargo build --release` passed in 1m39s; local rust-objcopy emitted a nonfatal missing-libLLVM warning.

## Found, not fixed
- Local rust-objcopy cannot load libLLVM.dylib for debug stripping; release compilation still exits successfully.
- Prior Fix 19 live-helper lifecycle and Fix 20 successful Admin file/helper lifecycle remain owner follow-ups.
- Historical database-create failure still needs QA 03 VM request/journal/kernel evidence; no OOM, timeout or panic cause is claimed.
- Prior Linux/VM and pending CI/release work remain owner follow-ups; prior runtime target remains `v0.1.0-alpha.43`.
- Navigation maps/lookup remain lexical rather than Rust/JavaScript type resolution.

## Next
- Fix 22 — Root helper umask and unit modes.
