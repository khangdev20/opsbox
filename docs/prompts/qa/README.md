# QA prompts: small checks, tiny fixes

One file = one check of ONE thing, sized for **20 minutes at most**. Give an agent one file at a time:
`Follow docs/prompts/qa/README.md, then do docs/prompts/qa/NN-name.md.`
Order does not matter much; start with 01 to 08. Run them against the Multipass VM `opsbox-vm` (or the local dev server) only, never a server with real data.

## Rules (these replace the heavy parts of CONVENTIONS for QA prompts)
1. **Time box: 20 minutes, hard.** At 15 minutes stop starting anything new. If the check is not finished, record what was done and what is left and stop. A prompt that hangs is a failed prompt.
2. **No long commands.** Wrap every command in `timeout 120`. Never run a full `cargo build --release` more than once per prompt; use `cargo check -p <crate>` and `cargo test -p <crate> <name>`. CI runs the full suite after the push.
3. **Check first, write the result down**, even when it passes: one row in `docs/QA.md` (create it if missing): `| NN | title | date | PASS / FAIL / PARTIAL | one-line note |`.
4. **Fix only if it is tiny**: at most one or two files and about 30 changed lines, no database migration, no change to an API shape, no new dependency. Add or adjust one test for the fix.
5. **Anything bigger is not fixed here.** Add a section to `docs/QA.md` under "Found, not fixed": what is wrong, the exact file and line, how to reproduce in two lines, and a suggested prompt title. Then stop. Do not start a refactor.
6. Stay in scope. Do not fix unrelated things you notice; list them under "Found, not fixed".
7. If the check needs something you do not have (a real domain, R2 or S3 keys, a Telegram bot, a second machine), write the exact manual steps for the owner into `docs/QA.md` under "Owner checks" and finish. Never invent credentials and never ask for them in chat.
8. Git: branch `dev`, never `main`, never force-push. Commit as `qa NN: <title>` and `git push origin dev`. If code changed and CI passes, tag the next `v0.1.0-alpha.<N+1>` as in CONVENTIONS; if only docs changed, no tag.
9. When done, `git mv` the prompt to `docs/prompts/qa/archive/` and mark it done in the table below. Reply with: result (PASS/FAIL/PARTIAL), what you changed (or "nothing"), and one line on what to look at next.

## Index
| # | Check | Done |
|---|---|---|
| 01 | Fresh install on a clean VM (`01-fresh-install.md`) | x |
| 02 | Update and rollback between two alphas (`02-update-and-rollback.md`) | x |
| 03 | Helper down: messages, banner and no half-made things (`03-priv-helper-failure.md`) | x |
| 04 | serve and priv version match after update (04-version-skew.md) | x |
| 05 | TLS, cookies and security headers (05-transport-and-headers.md) | x |
| 06 | No secret in any API response (`06-secrets-in-responses.md`) | x |
| 07 | Login, lockout, TOTP and session revoke (`07-login-hardening.md`) | x |
| 08 | Roles: deployer and viewer cannot do admin things (`08-permissions.md`) | |
| 09 | Every page: zero console errors, zero failed requests (`09-console-and-network-errors.md`) | |
| 10 | Nothing in http mode still uses sample data (`10-mock-leftovers.md`) | |
| 11 | Phone widths: no sideways scroll, targets big enough (`11-mobile-390.md`) | |
| 12 | Keyboard use and visible focus (`12-keyboard-and-focus.md`) | |
| 13 | Empty and error states on every page (`13-empty-and-error-states.md`) | |
| 14 | Capacity guard on the 1 GB VM (`14-capacity-1gb.md`) | |
| 15 | Ports 80/443 already taken (`15-proxy-port-conflict.md`) | |
| 16 | Containers: start, stop, restart, logs, remove (`16-containers-actions.md`) | |
| 17 | Deploy a Docker image app end to end (`17-deploy-image-app.md`) | |
| 18 | Local backup and restore round trip (`18-backup-and-restore-local.md`) | |
| 19 | Alerts fire and notify (`19-alerts-fire.md`) | |
| 20 | Terminal behaves like a real shell (`20-terminal-session.md`) | |
| 21 | Files: traversal, symlinks and big files (`21-files-safety.md`) | |
| 22 | Cleanup page speed (`22-cleanup-speed.md`) | |
| 23 | Audit log integrity (`23-audit-chain.md`) | |
| 24 | Export, uninstall, reinstall, import (`24-export-import-uninstall.md`) | |
| 25 | Firewall cannot lock the admin out (`25-firewall-lockout.md`) | |
| 26 | Watchdog and safe mode (`26-watchdog-safe-mode.md`) | |
| 27 | Dependencies and lint, fast (`27-dependencies-and-lint.md`) | |
| 28 | README, install page and links (`28-docs-and-links.md`) | x |
| 29 | Table layout: padding, scrollbar, columns and alignment (`29-table-layout.md`) | |
| 30 | List rows: push the action to the right edge (`30-list-row-actions.md`) | |
| 31 | Notification list: stray commas, stale alerts, restarting timestamps (`31-notification-list.md`) | |
