# Status

Last completed prompt: `docs/prompts/archive/16j-uninstall.md` (uninstall opsbox). Not committed, not tagged (per session request). Prior: `16i` (export configuration), `16h` (grow the filesystem), `16g`, `16f` (tag `v0.1.0-alpha.20`).

## What works (16j)
- **CLI** `sudo opsbox uninstall [--keep-data | --purge] [--yes]`: stops/disables opsbox services and timers (including `opsbox-cleanup` and `opsbox-job-*`), removes units, binary symlink, release slots and the `opsbox` user. Default keeps `/var/lib/opsbox` and `/etc/opsbox`; `--purge` deletes them (and local backups) after typed confirmation unless `--yes`. Managed containers get Docker restart `unless-stopped`. Adopted web receipts under `/etc/opsbox/web-moves` are restored first. Audit export at `/var/lib/opsbox/uninstall-<date>.log` (copy under `/var/log/` when purging).
- **Priv / API**: `GET /api/v1/system/uninstall/plan`, `POST /api/v1/system/uninstall` (interactive admin, password + typed `uninstall`). Exec schedules a detached `systemd-run` unit. Code: `crates/opsbox-priv/src/ops/uninstall.rs`, `crates/opsbox-api/src/uninstall.rs`, `crates/opsbox/src/cmd/uninstall.rs`.
- **UI**: Settings → Export and uninstall shows the plan (removed / kept / domains), confirm modal, then “opsbox is being removed”. `uninstallPlan` and `uninstallExec` are in `REAL`.
- **Docs**: README “Uninstall”, `install.sh` help, `docs/API.md`, `docs/CONFIG.md`, `docs/DECISIONS.md` (16j).
- **Tests**: plan builder (no domains / domains / adopted) and dry-run path removals in `crates/opsbox-priv` (module + `tests/uninstall.rs`).

## Partial / Found, not fixed
- **cargo test not run in this session** (shell was unavailable). Please run: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo test -p opsbox -p opsbox-api -p opsbox-priv` and `python3 tools/generate-api.py`.
- **Archive:** content is in `docs/prompts/archive/16j-uninstall.md`; if `docs/prompts/16j-uninstall.md` is still present, remove it with `git rm docs/prompts/16j-uninstall.md` (or `git mv` if git does not already see the rename).
- **Not verified on `opsbox-vm`** (install → deploy → panel uninstall → container still up, `systemctl list-units 'opsbox*'` empty). Not checked at 390 px.
- Adopted restore from CLI is receipt-based (config + reload), not the full `WebUndo` path through a live proxy Env.
- Carried over: 16i/16h VM checks; 03f / 16e / 03e / 03d pending commit/release; import is 16q.

## Next
- Commit 16j (with the pending stack if intended), push `dev`, wait for CI, tag when appropriate.
- Then `16k-user-display-names.md`.
- VM check for 16j: build and reinstall as usual; deploy one app; Settings → Uninstall (password + type `uninstall`). App container still answers on its port; `systemctl list-units 'opsbox*'` is empty; `/var/lib/opsbox` kept. Must fail safely: wrong password returns 403 and does not schedule uninstall.
