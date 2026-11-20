# Status

Last prompt done: **Fix 20 — Full API tokens must not be root**.
No commit, push, tag or installation attempted, as requested; changes remain on `dev`.

## Completed
- Refuse tokens on system jobs, file mutations, package/Docker installation, reboot/units, firewall/SSH/host/swap changes, automatic/self updates, security fixes, listener exposure, Compose submission and snapshot restore.
- Refuse session-only requests before capacity helper calls; token redeploys cannot reuse elevated Compose authorization, including previously confirmed stacks.
- Retain ordinary service deployment and interactive Admin authorization; Full scope now says “everything except root-equivalent actions” in UI/API docs.
- Added HTTP regressions and a live-router inventory check for explicit token decisions on all 138 mutating routes; recorded decision and QA, marked fix 20 done and archived its prompt.

## Verification
- Strict TDD: root-action HTTP requests initially reached helper/tasks; elevated Full-token Compose redeploy initially returned 503 instead of 403. Both regressions now pass.
- All four Compose role tests and the mutating-route policy test pass. The bind-free root-action test verifies 403, no file/staging changes, Admin settings success and Admin job authorization reaching the helper.
- Targeted all-target Clippy with `-D warnings`, formatting and regenerated index checks pass.
- Existing helper-backed token/file tests cannot start because this sandbox refuses Unix socket binds; successful Admin file/helper lifecycle remains an owner check in docs/QA.md.

## Found, not fixed
- Prior Fix 19 live-helper lifecycle and VM/browser checks remain owner follow-ups.
- Historical database-create failure still needs QA 03 VM request/journal/kernel evidence; no OOM, timeout or panic cause is claimed.
- Prior Linux/VM and pending CI/release work remain owner follow-ups; prior runtime target remains `v0.1.0-alpha.43`.
- Navigation maps/lookup remain lexical rather than Rust/JavaScript type resolution.

## Next
- Fix 21 — Safe Mode listens on the public interface by default.
