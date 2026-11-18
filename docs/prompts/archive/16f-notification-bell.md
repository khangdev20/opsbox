# 16f. Notification bell in the header

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-api/src/metrics.rs` (`get_attention`), `crates/opsbox-metrics` (`Attention`), `crates/opsbox-api/src/alerts.rs`, `crates/opsbox-store` (read state), `web/js/shell.js` (header), `web/js/pages/overview.js`, `web/js/ui.js` (`patch`, `morphChildren`), `web/js/api.js`, `web/css/app.css`, `web/kit.html` / `web/js/kit.js`, `docs/API.md`. Design: the header and Overview boards in `design/` (add the bell to the Header board in `design/gen.py` too so design and panel stay aligned).

## Notification bell instead of the "Needs attention" card

Today `GET /api/v1/attention` feeds a large "Needs attention · N" card on Overview only. Problems seen on a real server: the card pushes the charts down, it is invisible on every other page, and the same problem shows twice ("Scheduled backup missed for opsbox (daily 02:00)" from backups and "Backup opsbox is overdue" from alerts).

### Backend
- Give every attention item a stable `id` (kind plus subject, for example `backup:opsbox`, `cpu:host`, `deploy:web`, `cert:shop.example.com`, `update:0.2.0`), a `severity` (`critical` or `warning`; info items such as "update available" are `info`), a `since` timestamp, and the existing text and link.
- Deduplicate by subject: when an alert and a built-in check describe the same thing (backup missed and backup overdue, disk alert and disk check), return one item, keep the higher severity, and link to the most useful page (the alert if one fired, else the feature page). Write a table of which sources map to which id in `docs/API.md`.
- Read state per user: `POST /api/v1/attention/{id}/read`, `POST /api/v1/attention/read-all`, `POST /api/v1/attention/{id}/snooze` (`{"minutes": 60}`, max 7 days). An item that changes (new `since` or higher severity) becomes unread again. Snooze never hides `critical` items from the count, only from the list order (they stay on top, dimmed).
- `GET /api/v1/attention` returns `{ items: [...], unread, critical }`. Keep the per-viewer filtering that exists today (service scopes, deployer ownership).

### Header bell
- An icon button in the header between search and the theme toggle, on every page: bell icon, `aria-label="Notifications, N unread"`.
- Badge with the unread count (hidden at 0, "9+" above 9): warning colour, danger colour when any unread item is critical.
- Motion, only when something new arrives (the unread count goes up or a new critical item appears): the bell rings once (short rotate wiggle, about 600 ms) and the badge pops once. A critical item adds a slow soft pulse on the badge until it is read. Never a continuous shake. All of it off under `prefers-reduced-motion`. Polite announcement for new critical items only.
- Click opens a popover (`role="dialog"`, labelled, focus moves in, Escape and outside click close, focus returns to the bell): header "Notifications" with "Mark all read"; items newest first with critical on top, each with severity icon, text, relative time, the action link ("View backups →"), and a small menu for "Mark read" and "Snooze 1 h / 24 h"; empty state "All clear". Unread items have a dot and a slightly stronger background. Keyboard: arrow keys move between items.
- Polls with the existing `poll()` helper (pauses when the tab is hidden), shares one request with Overview so the API is not called twice.
- Mobile: the popover becomes a sheet from the top, full width.

### Overview
- Remove the "Needs attention" card.
- When there are unread critical items, show one thin banner line at the top of Overview: icon, the most important item's text and its action link, plus "and N more" opening the bell popover. Nothing for warnings only.

## Verify on `opsbox-vm`
- Trigger a missed backup and a high CPU alert: the bell shows 2 (not 3), rings once, the popover lists both with working links; mark one read, the count drops; snooze the other, it leaves the count and comes back after the snooze.
- A critical item (fill the disk past the alert threshold in a test file): red badge with pulse and the Overview banner; resolving it removes both.
Record the results and a short screen recording note in `docs/STATUS.md`.

Out of scope: email or push delivery (alerts already cover channels), a notification history page.
