# 16g. Capacity gets its own screen; Overview keeps a one-line summary

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `web/js/capacity.js` (`capacityCard`, `addSwap`), `web/js/pages/overview.js`, `web/js/pages/system.js` (tabs), `web/js/main.js` or wherever routes are registered, `web/js/shell.js` (command palette entries), `web/css/app.css`, `web/kit.html`. Backend unchanged unless noted: `GET /api/v1/capacity` already returns `allocatable`, `features[]` (with `fit.status`, `fit.reasons`, `minimum_mb`, `recommended_mb`, `disk_mb`) and reservations. Design: add the screen to `design/` (System board family) in the same style, and a matching note in prompt 16d's table.

Problem: the Capacity card from 15b sits on Overview as a full-width block: one sentence of totals, a stacked bar, a long single-line legend that wraps, then eight plain lines like `install: tight`, `build: blocked`. It takes more space than the charts, and the raw feature keys and missing reasons make it hard to read.

## 1. Where it lives
- A new tab **Capacity** on the System page (`#/system/capacity`; tabs become Updates, Services, Capacity, Logs, Jobs, Firewall). No new sidebar item. Deep links from anywhere (`#/system/capacity`) open that tab.
- Command palette entries: "Capacity", "Add swap".

## 2. Overview: one line instead of the card
- Remove the Capacity card from Overview.
- Add the summary to the Memory KPI tile footer: `471 MB free to allocate · Capacity →` (link to the tab). When any feature the user is likely to use is `blocked` or memory pressure is high, the footer turns warning colour with the reason in a few words ("builds blocked", "memory pressure high").
- Memory pressure high, or allocatable below the smallest app default, raises a notification (prompt 16f bell, severity warning; critical when pressure stays high 10 min). No banner on Overview for capacity.

## 3. The Capacity tab
Top to bottom:
1. **Summary row** of three tiles: Allocatable memory (big number, `of 956 MB RAM`), Reserved (OS + opsbox + Docker + workloads), Memory pressure (normal / elevated / high, from PSI, with a tiny 10 min sparkline if the metrics history already has it). Plus swap: `2 GB swap, counts as 914 MB for apps, never for databases`, with the "Add a swap file" action when the 15b rules offer it.
2. **Memory map**: the stacked bar, full width and taller (12 px), segments with 2 px gaps and rounded ends; below it a wrapping legend of chips, one per segment (colour dot, name, MB). Workload chips link to their app or container. Hover or focus on a chip highlights its segment and the reverse. Fixed colours: OS and opsbox muted greys, Docker neutral, workloads cycle the chart palette, free is the track colour.
3. **What this server can run**: a table, one row per feature, with a human label and description instead of the raw key:
   - install → "opsbox itself", docker → "Docker engine", app → "New app (default 128 MB)", build → "Build on this server", database → "New database", template → "Templates", backup → "Backup runs", image_pull → "Image pulls".
   - Columns: Feature, Status badge (Fits / Tight / Blocked, colours ok / warn / danger, with icon, not colour only), Needs (`min 768 MB · rec 1 GB`, plus disk when non-zero), Why (the first reason from `fit.reasons`, full list on expand), and an action where one exists: "Build in GitHub Actions" for blocked builds (link to the deploy settings), "Add swap", "See what uses memory" (scrolls to the reservations table).
4. **Reservations**: table of workloads with their memory limit (reservation), actual usage now, and a small bar of usage within the limit; sortable; "Edit limit" opens the existing `editMemory` dialog with the live fit check. Workloads marked over capacity show the badge from 15b.
5. A short explainer in muted text at the end: how reserves are computed (OS, opsbox, Docker) and that swap does not count for databases, with a link to the docs section.

Use the existing helpers (`section`, `table`, `st`, `segBar`, `legend`, `patch` with keyed rows) and the 16b motion rules: bars tween, rows update in place, the reservations table uses `data-flip` when sorted (16f).

## 4. Shared pieces
- Move the feature label map and status mapping into `capacity.js` so the deploy, database and template dialogs (which already show fit results) use the same labels and badges.
- `capacityCard` is removed or reduced to the Overview footer helper; no dead code left.

## Tests and checks
- Unit (or kit) checks: every feature key from the API has a label (unknown keys fall back to a title-cased key, never crash); status mapping covers ok, tight, blocked.
- Overview no longer requests capacity twice; the Memory tile footer and the tab share one fetch per poll.
- Kit page shows the memory map, the feature table in all three states, and the Overview footer in normal and warning states.

## Verify on `opsbox-vm` (1 GB)
Overview shows the one-line footer and no Capacity card. The Capacity tab shows the memory map with the three greenglish containers as chips, builds and databases as Blocked with readable reasons and actions, Add swap works and updates the numbers without a reload. Before and after screenshots of Overview in `docs/screenshots/16g/`; results in `docs/STATUS.md`.

Out of scope: changing the capacity rules or numbers (15b), CPU reservations UI beyond showing the allocatable millicores in the summary.
