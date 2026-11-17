# Agent prompts

Small, ordered prompts for building opsbox. Give an agent ONE file at a time, in order. Each prompt is sized for a single session.

When a prompt is fully done, the agent moves it to `archive/` and marks it done below, so the lowest-numbered file left in this folder is always the next one.

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
| 04 | Public first run (`04-public-first-run.md`) | 1c | ✓ |
| 05 | Proxy and panel domain (`05-proxy-panel-domain.md`) | 4 (panel first) | ✓ |
| 06 | Watchdog and safe mode (`06-watchdog-safe-mode.md`) | 2 | ✓ |
| 07 | Docker and Containers page (`07-docker-containers.md`) | 3 | ✓ |
| 08 | GitHub connection (`08-github-connection.md`) | 3 | ✓ |
| 09 | Deploy pipeline (`09-deploy-pipeline.md`) | 3 | ✓ |
| 10 | App domains (`10-app-domains.md`) | 4 | |
| 11 | Templates and Compose (`11-templates-compose.md`) | 3 | |
| 12 | Databases part A: link to apps (`12-databases-link.md`) | 3 | |
| 13 | Backups engine (`13-backups-engine.md`) | 5 | |
| 14 | Backup destinations and restore (`14-backup-destinations-restore.md`) | 5 | |
| 15 | Alerts (`15-alerts.md`) | 5 | |
| 16 | Databases part B: management (`16-databases-manage.md`) | 5b | |
| 17 | Alpha release (`17-alpha-release.md`) | release | |
