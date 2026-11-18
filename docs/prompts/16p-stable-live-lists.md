# 16p. Stable live lists (FLIP) everywhere

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `web/js/ui.js` (`patch`, `morphChildren`), `web/js/pages/overview.js`, `web/js/pages/containers.js`, `web/js/pages/system.js`, the notification popover from 16f, `web/js/kit.js`, `web/css/app.css`.

## Context and work

Problem: the keyed list patch from 16b let a row that dropped out of a ranked list fade out in place while new rows were inserted, so "Top processes" (5 rows) grew to 6 to 8 rows for 200 ms and then shrank.

Already done on 2026-11-19 (keep it, test it, do not revert):
- `morphChildren` in `ui.js` supports `data-flip` on a list container: rows that leave are removed at once, rows that move animate from their old position with FLIP (`Element.animate`, `translateY`, `--dur-3`), new rows use the normal enter animation.
- Overview wraps the process rows in `<div class="pr-list" data-flip>`.

To do:
- Apply `data-flip` to every other list with a fixed number of rows or a ranking: Overview containers (top 5), the notification popover list (if 16f is done), any "top N" in System, and the containers table when sorting changes.
- Respect reduced motion (no FLIP, no enter animation), and skip FLIP for rows that move more than the visible list height (just place them).
- Tests (in the existing web test setup, or a small Playwright check under `web/tests/`): patching a 5-row `data-flip` list into a different 5-row order keeps the container height constant and the row count at 5 at 0, 60, 120 and 300 ms; a non-flip list still fades leaving rows.
- Add both cases to the UI kit page with a "shuffle" button.

## Verify on `opsbox-vm`
- Run a CPU load (`yes > /dev/null` for 20 s) while watching Top processes: rows reorder by gliding, the card never changes height.
Record the results in `docs/STATUS.md`.
