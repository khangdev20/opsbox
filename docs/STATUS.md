# Status

Last completed prompt: `docs/prompts/archive/16d-align-panel-with-design.md` (tag `v0.1.0-alpha.20`).

## What works (16d, presentation only; no API changes)
- Logo (mark + "Ops"/"Box" wordmark) in top bar, sign-in, setup and safe mode; PNG favicons and apple-touch-icon (`web/img/`).
- Sidebar groups (Overview, Deploy, Server, Protect) with dividers when collapsed; Collapse is a bottom row; state stays in `localStorage`.
- Account menu (initials pill, `role="menu"`, arrows/Home/End, Escape, outside click, route change). Access, Settings and API tokens live there; every route still reachable.
- Overview: 6-column grid, 24px gaps, 300px chart with current/avg/peak, Top processes (live badge, working CPU/Memory sort, "Heaviest right now", "this" badge), Containers / Reclaimable / Recent activity from the existing real calls.
- Select, checkbox, radio as in `design/gen.py`; shared `iconAct` row actions (Apps, Backups); inline styles in pages replaced by classes; `web/kit.html` shows logo, menu, sidebar, process rows, icon actions.
- Tools: `design/preview.py` (boards to `docs/screenshots/design/`), `tools/screenshot-panel.py` (before/after in `docs/screenshots/16d/`).

## Board -> module -> differences (all other differences closed)
| Board | Module | Status |
|---|---|---|
| Header, Nav, Status | shell.js | aligned (logo, palette, theme, menu, groups, collapse, status bar) |
| Login, Setup, Recovery | login/setup/recovery.js | aligned layout, logo, form-err, theme button; design marketing copy, install snippet and 5-step setup not adopted (panel flow has 3 steps); recovery gained design header |
| Main, MainLight | overview.js | aligned; process type ("container"/"system") not in API, so only PID shown |
| Apps, AppsLight, AppDomains | apps.js | tokens + icon actions aligned; topology stays a free graph (no "Services" card, build log opens inline below table): deliberate |
| NewService, Templates, Configure | newservice/templates/configure.js | shared tokens, form-err, spacing utilities |
| Containers | containers.js | panel redesign kept (exception in prompt) |
| System, Terminal, Files, Cleanup | system/terminal/files/cleanup.js | shared tokens only; no per-page layout differences found |
| Backups, BackupDest | backups/backupdest.js | icon actions on schedules, snapshots, destinations |
| Alerts, Security, Access, Settings | alerts/security/access/settings.js | tokens aligned; Security keeps panel layout (checks as rows, audit beside users), audit has no IP column and no "Export CSV" (backend gap) |
| none | database/enroll/invite.js | same components and spacing |

## Verified
- `node --check` on every `web/js/**/*.js`; capacity, alerts and network UI contract scripts pass.
- Browser run (Playwright, demo mode): account menu keys/Escape/outside/route/sign-out, sort switch, collapse persistence, no console errors on 22 screens x 2 themes.
- Rust checks: see the final commit message log; fmt, API generator, clippy, tests and release build run before tagging.

## Partial / Found, not fixed
- Screenshots use the built-in demo data (`?mode=mock`): `opsbox-vm` needs Owner credentials and a 2FA code that automation does not have. Run `tools/screenshot-panel.py --base https://...:8443 --storage state.json` against a signed-in session for real data.
- No display names in the backend: the menu shows the sign-in name; "2FA on" is true for every browser session (sign-in requires a code).
- `design/` files were not touched (owner edits); `design/project/assets/` logos copied to `web/img/`.
- Earlier open items remain: live DB/R2/MinIO/SFTP/alert-channel checks, pgweb path prefix, Capacity override click-through, idle browser CPU profiling, Caddy/Traefik live cutovers.
- `capacity_http::queued_workloads_block_database_resize_pooler_and_memory_adding_extensions` fixture issue (see DECISIONS) is unchanged.

## Next
- `docs/prompts/16e-transport-and-response-security.md`, then `17-alpha-release.md`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`.
- Check: sidebar groups, account menu with arrow keys, Overview sort switch, one wrong sign-in code must be refused with the form error.
