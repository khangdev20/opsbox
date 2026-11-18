# Status

Last completed prompt: `docs/prompts/archive/16f-notification-bell.md` (Notification bell, attention dedup, unread state, popover, overview banner; `tools/check-notifications-ui.mjs` passed). Prior: `16d` (tag `v0.1.0-alpha.20`).

## What works (16f)
- Attention items have stable ids, severity, and `since`; same-subject backup/disk/etc. sources dedupe to one item (higher severity wins).
- Per-user read / read-all / snooze on `/api/v1/attention`; `GET` returns `{ items, unread, critical }`.
- Header notification bell (badge, ring/pop on new unread, critical pulse), popover with mark-read/snooze, shared poll with Overview.
- Overview: large "Needs attention" card removed; thin critical banner when unread critical items exist.
- UI contract: `tools/check-notifications-ui.mjs` passed.

## Verified
- Prompt 16f (Notification bell, attention dedup, unread state, popover, overview banner, `tools/check-notifications-ui.mjs` passed) completed.
- Earlier: `node --check` on `web/js/**/*.js`; capacity/alerts/network UI contracts; 16d Playwright demo pass.

## Partial / Found, not fixed
- Screenshots use demo data (`?mode=mock`); real VM shots need Owner + 2FA session state.
- No display names in the backend; "2FA on" is true for every browser session.
- Earlier open items remain: live DB/R2/MinIO/SFTP/alert-channel checks, pgweb path prefix, Capacity override click-through, idle browser CPU profiling, Caddy/Traefik live cutovers.
- `capacity_http::queued_workloads_block_database_resize_pooler_and_memory_adding_extensions` fixture issue (see DECISIONS) unchanged.

## Next
- `docs/prompts/16e-transport-security.md` (lowest remaining), then `16g-capacity-page.md` and the rest of the 16* release-prep series, then `17-alpha-release.md`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`.
- Check: bell badge/unread, mark-read and snooze; one wrong sign-in code must still be refused.
