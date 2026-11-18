# 16j. Uninstall opsbox (keep apps and data)

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox/src/cmd/` (new `uninstall.rs`), `crates/opsbox-priv` (op), `crates/opsbox-api` (endpoint), `install.sh` (document the reverse), `web/js/api.js`, `web/js/pages/settings.js`, `docs/` (README section "Uninstall").

Today: the "Uninstall opsbox" button only shows "The demo does not uninstall anything" and is disabled in real mode. There is no CLI path either.

Do:
- CLI `sudo opsbox uninstall [--keep-data | --purge] [--yes]`: stops and disables `opsbox`, `opsbox-priv`, `opsbox-proxy`, `opsbox-watchdog`, removes their units, the binary and the release slots, timers (`opsbox-cleanup.timer`, backup timers), and the `opsbox` system user. Default `--keep-data`: leaves `/var/lib/opsbox` (database, audit log, backups config) and `/etc/opsbox` in place and prints where they are. `--purge` removes them after a typed confirmation.
- Apps keep running: containers opsbox started are switched to Docker restart policy `unless-stopped` so they survive reboots without opsbox. Ports 80/443: if the opsbox proxy served domains, print the list and warn that those domains stop working until another proxy takes over; if 16c moved an old web server behind opsbox, undo that move first (restore its ports) so the old sites keep working.
- Panel: Settings > Uninstall shows exactly this plan (what is removed, what stays, which domains stop), needs re-auth and typing `uninstall`, then starts the uninstall from `priv` in a detached transient unit (`systemd-run`) so it can stop the panel itself; the page shows a final "opsbox is being removed" screen.
- Audit entry written and exported to `/var/lib/opsbox/uninstall-<date>.log` before removal.

Tests: the plan builder for each case (no proxy domains, domains, adopted web server); a dry run on a temp root that checks every path removed or kept.

Verify on `opsbox-vm`: install, deploy one app, uninstall from the panel; the app container still answers on its port, `systemctl list-units 'opsbox*'` is empty, data dir kept; reinstall picks up the kept data (or document that it does not). Record in `docs/STATUS.md`.
