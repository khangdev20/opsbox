# Status

Last prompt completed: **Fix 27 — Helper operation authorization by caller**.
No commit, push, tag or installation attempted; changes remain on `dev`, as requested.

## Completed
- Carry Caller::Serve/Proxy from Fix 26's UID-checked listeners into each connection.
- Exhaustively match PrivOp with no catch-all: Proxy gets only PortHolders and SecretSeal/SecretOpen for proxy: contexts. Every new variant requires an explicit decision.
- Check policy before blocking dispatch and PTY/log/build streams; preserve request IDs and return typed Forbidden on every denied request.
- Emit structured priv_op_forbidden warnings with kernel UID/PID, role and operation, without payloads or the caller-supplied actor.
- Keep Serve authorized for now; record future API admin proof for root-grade operations in ROADMAP.
- Add five regressions plus filesystem-listener coverage; update decisions/QA, archive fix 27 and regenerate navigation maps.

## Verification
- Tests written first failed on missing policy APIs; replaying UID-only admission caused all three policy/response tests and the real-connection regression to fail assertions.
- Five new tests pass, including kernel credentials/framing through a socket pair, all streaming denials, no scratch write, successful allowlisted operations after refusals, and denial-log fields/redaction.
- Full requested helper suite: 233 pass, three existing environment failures (socket bind denied, stripped setuid, denied uptime sysctl). Cargo stops before integration suites.
- Explicit proxy socket integration run: shared-UID refusal passes; both listener tests cannot bind in this sandbox. Linux owner checks are in docs/QA.md.
- Formatting, all-target helper Clippy with warnings denied and whitespace checks pass.
- Release build passes (1m22s), with the existing rust-objcopy/libLLVM debug-stripping warning. Generated index regenerated; freshness check passes.

## Limits and next prompt
- Serve retains its full typed operation set; root-grade operations need future API admin proof. Audit actor provenance remains Fix 29.
- Live Linux listener, journal, ACME and panel regression checks remain owner work.
- Next: Fix 28 — verify staged update signatures inside the root helper.

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
- Fix 28 — Verify staged update signatures in the root helper.
