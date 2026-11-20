# Status

Last prompt completed: **Fix 29 — Helper audit identity from peer credentials**.
No commit, push, tag or installation attempted; changes remain on `dev`, as requested.

## Completed
- Add kernel peer_uid and optional pid to all four priv_request spans: ordinary requests, PTY, Docker log follow and deploy builds.
- Label the caller-supplied actor as claimed_actor; preserve PrivRequest and authorization behavior.
- Add socket-pair regression coverage for all four paths with alice and absent actor; compare structured fields with real kernel credentials.
- Document advisory actor/authoritative kernel identity, record the three-line decision, update QA and archive Fix 29.

## Verification
- Strict TDD: the new test first failed on missing peer_uid with the old actor=alice/uid span; all eight cases now pass.
- Full helper suite: 237 pass, three existing environment failures (socket bind denied, stripped setuid, denied uptime sysctl); Cargo stops before integration suites.
- All-target helper Clippy with warnings denied and generated-index regeneration/freshness checks pass.
- Formatting and whitespace checks pass. Release build passes (1m22s), with the existing rust-objcopy/libLLVM debug-stripping warning.

## Limits and next prompt
- Complete Linux helper tests and installed-service journal checks remain owner work; exact steps are in docs/QA.md.
- Next: Fix 30 — Make the security docs say what is true.

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
- Fix 30 — Make the security docs say what is true.
