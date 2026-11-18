# Status

Last completed prompt: `docs/prompts/archive/16g-capacity-page.md` (Capacity tab; Overview one-line summary). Not committed, not tagged (per session request). Prior: `16f` (tag `v0.1.0-alpha.20`), plus unreleased `03f` / `16e` / `03e` / `03d` work still on `dev`.

## What works (16g)
- **System → Capacity** (`#/system/capacity`): tabs are Updates, Services, Capacity, Logs, Jobs, Firewall. Summary tiles (allocatable, reserved, pressure + swap action), memory map (taller stacked bar + legend chips with hover highlight; workload chips deep-link), feature fit table (friendly labels, Fits/Tight/Blocked badges, Needs, Why, actions), reservations table (limit, live usage from containers when available, usage bar, sortable, Edit limit).
- **Overview**: bulky Capacity card removed. Memory KPI footer shows `N MB free to allocate · Capacity →` (warning tint for blocked builds/apps/DBs or high pressure). One `getCapacity` fetch per poll shared with that footer.
- **Shared**: `FEATURE_LABELS`, `featureLabel`, `fitStatus` / `fitBadge`, `capacityFooter`, `capacityPage` in `web/js/capacity.js`. Command palette: "Capacity", "Add swap". Kit shows memory map, feature table (all three states), and footer normal/warning. `tools/check-capacity-ui.mjs` covers labels, badges, footer and page markup.
- Route: `main.js` accepts `/system/<tab>`; tab switches use `history.replaceState` so the page is not remounted.

## Partial / Found, not fixed (16g)
- **Capacity → notification bell** (pressure high / allocatable below default app size → 16f warning; critical after 10 min): not wired; needs attention items from the API. No Overview capacity banner (correct per prompt).
- **Design board** for Capacity in the System family and a 16d table note: not added.
- **PSI sparkline** (10 min): skipped — metrics history has mem %, not PSI samples.
- **Not verified on `opsbox-vm`**: no before/after screenshots in `docs/screenshots/16g/`. Run the checks below.
- Shell/delete were unavailable in the implementing session: the prompt was **copied** to `docs/prompts/archive/16g-capacity-page.md` but the original `docs/prompts/16g-capacity-page.md` may still be present. Before commit run: `git mv docs/prompts/16g-capacity-page.md docs/prompts/archive/16g-capacity-page.md` (or remove the original if the archive copy is already there). Also run `node tools/check-capacity-ui.mjs`.

## Earlier open items (unchanged)
- 03f / 16e / 03e / 03d still pending commit/release; TLS VM checks; helper-banner and form-state work from 03f.
- Database dialogs still close before the request (not converted to `run`); capacity reserve RSS check; streaming priv call logging; unit-file OOMScoreAdjust only after reinstall.

## Next
- Commit 16g (and any pending 03f/16e stack if intended), push `dev`, wait for CI, tag when appropriate.
- Then `16h-grow-filesystem.md`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`.
