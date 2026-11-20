# Fix prompts: one small bug, one test, one commit

Each file fixes ONE problem found by the QA runs (`docs/QA.md`, section "Found, not fixed") in **20 minutes at most**. Give an agent one file at a time:
`Follow docs/prompts/fix/README.md, then do docs/prompts/fix/NN-name.md.`
Order is by importance for the public alpha: 01 to 06 first.

## Rules
1. **Time box 20 minutes, hard.** At 15 minutes stop adding anything. If the fix is not finished, commit nothing half-done: write what you learned (root cause, files, a suggested split) under the matching entry in `docs/QA.md` and stop.
2. **Test first.** Reproduce the problem with a failing test (Rust test in the owning crate, or a Node/DOM check for the panel) before changing code. If a test truly cannot be written, say why in the commit message and show the manual reproduction.
3. **Smallest fix.** Change only what the prompt names. No refactors, no new dependencies, no API shape changes unless the prompt says so. Do not touch unrelated findings; list them under "Found, not fixed".
4. **Fast checks only.** `timeout 120` on every command. `cargo check -p <crate>`, `cargo test -p <crate> <name>`, `cargo fmt --check`, `cargo clippy -p <crate> -- -D warnings`. The full suite runs in CI after the push.
5. If a check needs a live VM, a real domain or credentials you do not have, do the code and unit-test part, then add exact steps for the owner under "Owner checks" in `docs/QA.md`.
6. After the fix: update the matching QA entry to `Fixed in fix NN` and, if the routes, `PrivOp`s or UI map changed, run `python3 scripts/gen-index.py` (once prompt 18 exists).
7. Git: branch `dev`, never `main`, never force-push. Commit `fix NN: <title>`, `git push origin dev`. When CI passes, tag the next `v0.1.0-alpha.<N+1>` as in CONVENTIONS (docs-only changes get no tag). Move the prompt to `docs/prompts/fix/archive/` and mark it done below.
8. Reply with: what was wrong (one line), what you changed, the test that proves it, and anything left for the owner.

## Index
| # | Fix | Done |
|---|---|---|
| 01 | The one-line install command returns 404 ([01-install-link-404.md](archive/01-install-link-404.md)) | Fixed in fix 01 |
| 02 | Demo values leak into the real panel ([02-demo-data-in-real-panel.md](archive/02-demo-data-in-real-panel.md)) | Fixed in fix 02 |
| 03 | Removing the panel's firewall allowance can lock the admin out ([03-firewall-panel-port-guard.md](archive/03-firewall-panel-port-guard.md)) | Fixed in fix 03 |
| 04 | Pending firewall rollback can be lost ([04-firewall-rollback-state.md](archive/04-firewall-rollback-state.md)) | Fixed in fix 04 |
| 05 | Export leaks inline Compose credentials ([05-export-compose-secrets.md](archive/05-export-compose-secrets.md)) | Fixed in fix 05 |
| 06 | Import accepts invalid documents and can hide key failures ([06-import-schema-and-key-errors.md](archive/06-import-schema-and-key-errors.md)) | Fixed in fix 06 |
| 07 | Database restore can write before a late corruption is found ([07-restore-before-corruption.md](archive/07-restore-before-corruption.md)) | Fixed in fix 07 |
| 08 | Rapid service crashes can evade safe mode ([08-watchdog-rapid-crashes.md](archive/08-watchdog-rapid-crashes.md)) | Fixed in fix 08 |
| 09 | Fallback emergency SSH rule never expires ([09-nft-emergency-ssh-expiry.md](archive/09-nft-emergency-ssh-expiry.md)) | Fixed in fix 09 |
| 10 | Requests never time out and a failed System tab stays on the skeleton ([10-api-timeout-and-mount-errors.md](archive/10-api-timeout-and-mount-errors.md)) | Fixed in fix 10 |
| 11 | Apps page can reload repeatedly when the domain request fails ([11-apps-domain-reload-loop.md](archive/11-apps-domain-reload-loop.md)) | Fixed in fix 11 |
| 12 | Missing empty-state guidance on Files, System and Backups ([12-empty-states-files-system-backups.md](archive/12-empty-states-files-system-backups.md)) | Fixed in fix 12 |
| 13 | Touch targets below 44 px ([13-touch-targets-44.md](archive/13-touch-targets-44.md)) | Fixed in fix 13 |
| 14 | Cleanup scans pile up ([14-cleanup-scan-overlap.md](archive/14-cleanup-scan-overlap.md)) | Fixed in fix 14 |
| 15 | Cannot change an image service's tag ([15-image-tag-update.md](archive/15-image-tag-update.md)) | Fixed in fix 15 |
| 16 | Deleting the last app leaves its network and directory ([16-project-network-cleanup.md](archive/16-project-network-cleanup.md)) | Fixed in fix 16 |
| 17 | Docs say /health/ready but the route is /readyz ([17-readyz-doc-mismatch.md](archive/17-readyz-doc-mismatch.md)) | Fixed in fix 17 |
| 18 | Why did creating a database report that the helper is not running? ([18-db-create-priv-diagnosis.md](archive/18-db-create-priv-diagnosis.md)) | Fixed in fix 18 (source audit; VM diagnosis pending) |
| 19 | SECURITY: Deployer can reach root through Compose ([19-compose-elevated-admin-only.md](archive/19-compose-elevated-admin-only.md)) | Fixed in fix 19 |
| 20 | SECURITY: Full API tokens reach root-equivalent endpoints ([20-token-deny-root-endpoints.md](archive/20-token-deny-root-endpoints.md)) | Fixed in fix 20 |
| 21 | SECURITY: Safe Mode (root) listens on the public interface by default ([21-safe-mode-exposure.md](archive/21-safe-mode-exposure.md)) | Fixed in fix 21 (live owner checks pending) |
| 22 | SECURITY: Root helper creates files with a permissive umask ([22-priv-umask-and-unit-modes.md](archive/22-priv-umask-and-unit-modes.md)) | Fixed in fix 22 (live owner checks pending) |
| 23 | SECURITY: The file manager can rewrite /etc/opsbox/config.toml ([23-config-toml-no-modify.md](archive/23-config-toml-no-modify.md)) | Fixed in fix 23 (live owner checks pending) |
| 24 | SECURITY: apps/, xfer/, trash/ are created at runtime inside a group-writable dir ([24-install-root-owned-dirs.md](archive/24-install-root-owned-dirs.md)) | Fixed in fix 24 (live owner checks pending) |
| 25 | SECURITY: The internet-facing proxy can write the database ([25-proxy-readwrite-paths.md](archive/25-proxy-readwrite-paths.md)) | Fixed in fix 25 (live owner checks pending) |
| 26 | SECURITY: Give the proxy its own system user ([26-proxy-own-uid.md](archive/26-proxy-own-uid.md)) | Fixed in fix 26 (live owner checks pending) |
| 27 | SECURITY: The helper authorises by UID only, not by operation ([27-priv-op-allowlist-by-uid.md](archive/27-priv-op-allowlist-by-uid.md)) | Fixed in fix 27 (live owner checks pending) |
| 28 | SECURITY: The root helper runs a staged binary without checking its signature ([28-priv-verify-update.md](archive/28-priv-verify-update.md)) | Fixed in fix 28 (live owner checks pending) |
| 29 | SECURITY: Audit log trusts the caller's own actor string ([29-audit-actor-from-peercred.md](archive/29-audit-actor-from-peercred.md)) | Fixed in fix 29 (live journal owner check pending) |
| 30 | SECURITY: Make the security docs say what is true ([30-threat-model-honest-alpha.md](archive/30-threat-model-honest-alpha.md)) | Fixed in fix 30 (docs only) |
| 31 | Wide screen layout ([31-wide-screen-layout.md](archive/31-wide-screen-layout.md)) | Fixed in fix 31 |
| 32 | Cleanup page width ([32-cleanup-page-width.md](archive/32-cleanup-page-width.md)) | Fixed in fix 32 |

## Security batch (19-30): do these BEFORE the public alpha post
Order: 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30. Items 19 to 21 are the must-haves; 22 to 25 are quick hardening; 26 to 28 are the structural root-boundary work (26 before 27); 30 is docs and can run any time. Prompts 01 to 18 can wait until these are done.
| 33 | New service layout (`33-new-service-layout.md`) | |
