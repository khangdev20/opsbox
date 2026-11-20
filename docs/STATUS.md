# Status

Last prompt completed: **Fix 30 — Honest alpha security documentation (docs only)**.
No commit, push, tag or installation attempted; changes remain on `dev`, as requested.

## Completed
- Add Alpha status to THREAT_MODEL.md and SECURITY.md: panel UID compromise remains host-root compromise; fixes 26/27 restrict the separate proxy UID/socket/operations.
- Align README/site text on trusted deployers (fix 19), sensitive Full tokens with root-equivalent exclusions (fix 20), and the separate, opt-in Safe Mode source allowlist (fix 21).
- Keep private GitHub advisory reporting; document latest-alpha-only support and distinguish current source from installed releases.
- Record the three-line decision, update QA/fix index and archive Fix 30. No runtime, layout or style changes.

## Verification
- Manual source/document review covers helper caller authorization, peer credentials, Compose restrictions, token scope/denials and watchdog defaults.
- Generated-index regeneration makes no changes; `timeout 120 python3 scripts/gen-index.py --check` passes.
- `timeout 120 git diff --check` passes; misleading privilege/automatic-lockdown claims removed. No runtime tests or release build needed for prose-only changes.

## Limits and next prompt
- Complete Linux helper tests, installed-service journal checks and prior fixes' live owner checks remain; exact steps are in docs/QA.md.
- Fix prompts 01–30 are complete in the working tree; pending Linux owner checks and prior CI/release work are next.

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
- Run the pending Linux owner checks in docs/QA.md before the public alpha post; no new numbered fix prompt is queued.
