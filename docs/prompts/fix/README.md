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
| 11 | Apps page can reload repeatedly when the domain request fails (`11-apps-domain-reload-loop.md`) | |
| 12 | Missing empty-state guidance on Files, System and Backups (`12-empty-states-files-system-backups.md`) | |
| 13 | Touch targets below 44 px (`13-touch-targets-44.md`) | |
| 14 | Cleanup scans pile up (`14-cleanup-scan-overlap.md`) | |
| 15 | Cannot change an image service's tag (`15-image-tag-update.md`) | |
| 16 | Deleting the last app leaves its network and directory (`16-project-network-cleanup.md`) | |
| 17 | Docs say /health/ready but the route is /readyz (`17-readyz-doc-mismatch.md`) | |
| 18 | Why did creating a database report that the helper is not running? (`18-db-create-priv-diagnosis.md`) | |
