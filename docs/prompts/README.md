# Agent prompts

Small, ordered prompts for building opsbox. Give an agent ONE file at a time, in order. Each prompt is sized for a single session.

When a prompt is fully done, the agent moves it to `archive/`, marks it done below and tags the next `v0.1.0-alpha.N` release (see CONVENTIONS), so every finished prompt is an installable alpha and the lowest-numbered file left in this folder is always the next one.

How to use:
1. Open a new agent in `~/Developer/opsbox`.
2. Paste: `Follow docs/prompts/CONVENTIONS.md, then do docs/prompts/NN-name.md.`
3. When it finishes, check `docs/STATUS.md`, test in `opsbox-vm`, then take the lowest-numbered prompt still in this folder.

If a prompt turns out too big, ask the agent to stop at a clean point, update `docs/STATUS.md`, and continue in a new session with: `Follow docs/prompts/CONVENTIONS.md. Continue the task in docs/STATUS.md.`

| # | Prompt | Phase | Done |
|---|---|---|---|
| 01 | File access race (`01-file-access-race.md`) | 1b | ✓ |
| 02 | Release builds, signing and one-line install (`02-release-builds.md`) | 2 | ✓ |
| 03 | Self-update and rollback (`03-self-update.md`) | 2 | ✓ |
| 03b | Cleanup page performance (`03b-cleanup-performance.md`) | 1 (fix) | ✓ |
| 03c | Proxy port conflicts handled automatically (`03c-proxy-port-conflict.md`) | 1 (fix) | ✓ |
| 03c1 | Landing site: check and commit the new logo (`03c1-site-logo.md`) | site | ✓ |
| 03d | Fix self-update versions and channel (`03d-self-update-versions.md`) | 2 (fix) | ✓ |
| 03e | Self-update: say why nothing is offered (`03e-update-check-reasons.md`) | 2 (fix) | ✓ |
| 03f | Forms keep typed values; helper errors say what happened (`03f-form-state-and-helper-errors.md`) | 1 (fix) | ✓ |
| 04 | Public first run (`04-public-first-run.md`) | 1c | ✓ |
| 05 | Proxy and panel domain (`05-proxy-panel-domain.md`) | 4 (panel first) | ✓ |
| 06 | Watchdog and safe mode (`06-watchdog-safe-mode.md`) | 2 | ✓ |
| 07 | Docker and Containers page (`07-docker-containers.md`) | 3 | ✓ |
| 08 | GitHub connection (`08-github-connection.md`) | 3 | ✓ |
| 09 | Deploy pipeline (`09-deploy-pipeline.md`) | 3 | ✓ |
| 10 | App domains (`10-app-domains.md`) | 4 | ✓ |
| 11 | Templates and Compose (`11-templates-compose.md`) | 3 | ✓ |
| 12 | Databases part A: link to apps (`12-databases-link.md`) | 3 | ✓ |
| 13 | Backups engine (`13-backups-engine.md`) | 5 | ✓ |
| 14 | Backup destinations and restore (`14-backup-destinations-restore.md`) | 5 | ✓ |
| 15 | Alerts (`15-alerts.md`) | 5 | ✓ |
| 15b | Capacity guard: what this server can safely run (`15b-capacity-guard.md`) | 5 | ✓ |
| 16 | Databases part B: management (`16-databases-manage.md`) | 5b | ✓ |
| 16b | UI polish: live-data motion and styled controls (`16b-ui-polish.md`) | release prep | ✓ |
| 16c | Move an existing website behind opsbox (`16c-adopt-existing-web.md`) | 4 (fix) | ✓ |
| 16d | Align the Linux panel with the design (`16d-align-panel-with-design.md`) | release prep | ✓ |
| 16e | Transport security: TLS and cookies (`16e-transport-security.md`) | release prep | ✓ |
| 16f | Notification bell (`16f-notification-bell.md`) | release prep | ✓ |
| 16g | Capacity tab; Overview one-line summary (`16g-capacity-page.md`) | release prep | ✓ |
| 16h | Grow filesystem (`16h-grow-filesystem.md`) | release prep | ✓ |
| 16i | Export configuration (`16i-export-config.md`) | release prep | ✓ |
| 16j | Uninstall opsbox (keep apps and data) (`16j-uninstall.md`) | release prep | |
| 16k | User display names and real 2FA state (`16k-user-display-names.md`) | release prep | |
| 16l | Audit log IP, filters and CSV export (`16l-audit-ip-and-csv.md`) | release prep | |
| 16m | Top processes: container or system (`16m-process-kind.md`) | release prep | |
| 16n | Mock leftovers and capacity test fixture (`16n-mock-leftovers-and-fixture.md`) | release prep | |
| 16o | Secrets in responses: masked, reveal on demand (`16o-secrets-in-responses.md`) | release prep | |
| 16p | Stable live lists (FLIP) everywhere (`16p-stable-live-lists.md`) | release prep | |
| 16q | Import configuration on a new server (`16q-import-config.md`) | release prep | |
| 16r | Default security headers on every response (`16r-security-headers.md`) | release prep | |
| 16s | Responsive panel: phones and tablets (`16s-responsive-panel.md`) | release prep | |
| 17 | Alpha release (`17-alpha-release.md`) | release | |
