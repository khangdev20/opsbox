# Status

Last completed prompt: `docs/prompts/archive/16b-ui-polish.md` (tag `v0.1.0-alpha.18`).

## What works (16b)
- Motion tokens (`--dur-1..3`, `--ease-*`) in `app.css`; one reduced-motion block zeroes durations and `tweenNumber` jumps. Only opacity/transform/colour/shadow animate; bars use `scaleX`.
- `ui.js`: `tweenNumber`, `num`, `setBar`/`bar`/`segBar`, `flash`, keyed `patch` (rows update in place, new rows slide in, removed rows fade, changed rows flash, focus/scroll/typed input kept), `poll` + `refreshing` (pause when hidden, "updating" dot), `announce` (polite), `trackTask` rows (from `waitTask`), `createChart` (smooth append, stable y axis), skeletons, `empty`, `setBusy`/`withBusy`.
- Controls styled once: buttons (busy state on every `data-act` button), inputs, stepper, select, checkbox/radio, switch (pending/revert), sliding tabs, dialogs (focus trap/restore, Esc), stacked toasts, drop zone, scrollbars, `.tip`, tables, empty states. Status bar shows live / reconnecting.
- `web/kit.html` + `js/kit.js`: all components and states, dark and light side by side, "Simulate live data". Served by debug builds or to a signed-in Owner only (`ui.rs`, unit-tested).

## Audit checklist (mock panel, headless Chromium, dark + light, 1440 and 390 px; no console errors, no horizontal overflow on 18 routes)
- [x] Native controls restyled: select, checkbox, radio, textarea, number (spin hidden, stepper), file (drop zone), date/time, range, search, dialog, scrollbars, autofill (dark and light). `progress` is not used anywhere.
- [x] Live places: Overview metrics/tables/capacity/network (stream + 5 s poll), status bar, Containers poll, Alerts 15 s poll, Apps poll and deploy status, Cleanup scan, System tabs/journal, `waitTask` (updates, cleanup, backups, swap, Docker install), Files list.
- [x] Jumps fixed: numbers and bars (tween / scaleX), rows and chips (keyed patch, cross-fade), chart redraw, first-load blank page (skeleton after 140 ms), switch/tab snaps, dialog pop-in.
- [x] Before/after screenshots: `docs/screenshots/16b/{before,after}/` (Overview, Containers, Cleanup, Alerts, deploy in progress, form dialog; dark, mock data).

## Verified
- `node --check` on every `web/js/*.js` and `web/js/pages/*.js`; `node tools/check-{capacity,alerts,network}-ui.mjs`.
- `cargo fmt --all --check`; `python3 tools/generate-api.py --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked`; `cargo build --release --locked`.
- Browser (Playwright on the mock panel): rows/charts patched in place across polls, focus restored after dialog Esc, switch pending then revert on failure, double click ignored, reduced-motion run has no errors.

## Partial / Found, not fixed
- Not run on `opsbox-vm`: http-mode parts (real task rows from the server, stream "reconnecting", release `kit` gate for a real Owner cookie) are covered by code and the `ui.rs` unit test only.
- Idle CPU was not profiled in a browser Performance panel; the only infinite animations are CSS (pulse dots, skeleton, indeterminate bar).
- `html` templates drop `false`, so any attribute written as `"${bool}"` renders empty; fixed for tabs/switch/tiles, others may remain. `.topo-edge` flow still animates `background-position` (topology only).
- Earlier items still open: live DB/R2/MinIO/SFTP/alert-channel checks, pgweb path prefix, Capacity override click-through. `design/*` working-tree edits are untouched and unstaged.

## Next
- Prompt `17` alpha release; finish earlier live integration checks.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target cargo build --release`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Check Overview numbers glide, a Cleanup run shows a task row, and `/kit.html` is 404 when signed out or as a Deployer (and loads for the Owner).
