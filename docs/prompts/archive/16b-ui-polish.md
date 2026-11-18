# 16b. UI polish: motion for live data, native controls brought into the design system

Files: `web/css/app.css`, `web/js/ui.js`, `web/js/pages/*.js` (mainly `overview.js`, `containers.js`, `alerts.js`, `cleanup.js`, `apps.js`, `system.js`, `backups.js`, `settings.js`, `terminal.js`), `web/index.html`. Reference: the design canvas (`design/`, tokens in `design/gen.py`) and the existing tokens at the top of `app.css`.

Goal: the panel feels alive and finished. Values that update in real time change smoothly instead of jumping, and no control looks like a browser default. No new framework, no build step, no animation library: plain CSS and small vanilla helpers in `ui.js`. Behaviour and API calls stay as they are; this is presentation only.

## 0. Audit first
- Walk every page in both themes on `opsbox-vm` (and at 390 px width). List in `docs/STATUS.md`: each native control that still shows browser styling (`select`, checkbox, radio, `textarea`, `input[type=number|file|date|range|search]`, `dialog`, `progress`, scrollbars, autofill), each place that updates live (polling, `waitTask`, streams), and each visible "jump" (numbers, bars, rows, status chips, layout shift when data arrives).
- Take before screenshots of Overview, Containers, Cleanup, Alerts, a deploy in progress, and a form dialog; save under `docs/screenshots/16b/before/` (small PNGs).

## 1. Motion tokens (one place)
Add to `:root` in `app.css`:
- `--dur-1: 120ms` (hover, press), `--dur-2: 200ms` (state change), `--dur-3: 320ms` (enter, expand), `--ease-out: cubic-bezier(.2,.8,.2,1)`, `--ease-in-out: cubic-bezier(.4,0,.2,1)`.
- Use them everywhere instead of literal durations. Animate only `opacity`, `transform`, `background-color`, `border-color`, `color`, `box-shadow`; never width, height, top or left on large elements (for bars use `transform: scaleX()` with `transform-origin: left`).
- One global rule: under `prefers-reduced-motion: reduce`, durations drop to 0 and number tweens jump straight to the final value. Merge the existing reduced-motion rules into it.

## 2. Live data (the main part)
Helpers in `ui.js`, each small and tested by hand in the kit page (section 4):
- `tweenNumber(el, to, { format, dur })`: animates a displayed number (CPU %, memory, disk, network rate, counts, bytes freed) with `requestAnimationFrame`, using the existing `bytes()` and percent formatters. Skip the tween when the change is under 0.5% or the tab is hidden. Keep `font-variant-numeric: tabular-nums` so digits do not wobble.
- `setBar(el, ratio)`: meter and usage bars move with `scaleX` and `--dur-3`; colour changes (ok, warn, danger thresholds) cross-fade with `--dur-2`.
- `flash(el, kind)`: a short background highlight (`--accent` at about 12% fading to transparent over 900 ms) on a value or row that changed, for example a container restarting, a new alert, a deploy step finishing. Never flash on the first render.
- Status chips (`st()`): colour and label cross-fade; "running/building/pending" states get a subtle pulsing dot (reuse `topo-pulse`), "ok" is static.
- Lists that update (containers, alerts, deploy history, tasks): keyed diffing so rows are updated in place, not re-rendered. New rows fade and slide in 4 px (`--dur-3`), removed rows fade out before removal, and changed rows use `flash`. Keep scroll position and focus.
- Charts and sparklines (Overview metrics): append the new point and shift smoothly instead of redrawing with a jump; keep the y-axis scale stable (only rescale when the new max exceeds it by 20%) so lines do not bounce.
- Long tasks (`waitTask`: deploys, cleanup runs, backups, updates): a progress row with an indeterminate shimmer while there is no percent, a determinate bar when there is, step list items that tick from pending to running to done with the dot animation, and a final `toast`. The page never goes blank while a task runs.
- Loading: skeleton blocks (shimmer, respects reduced motion) sized like the real content for first loads, so nothing shifts when data arrives. Later refreshes keep the old data visible with a small "updating" dot in the section header; no spinners covering content.
- A small "live" indicator in the status bar: a steady dot when polling works, amber "reconnecting" after a failed poll, with the last-updated time on hover.
- Polling and animations pause when `document.hidden` and resume with one immediate refresh.
- Accessibility: live regions stay polite and announce only meaningful changes (task finished, alert fired), never every metric tick.

## 3. Controls that fit the design system
Style once in `app.css` (component classes, applied by the existing helpers), then replace ad hoc markup in pages:
- Buttons (`.btn` variants: default, primary, danger, link, icon-only): consistent height (36 px, 44 px touch target), hover, `:active` press (scale .98), focus ring, `disabled`, and a loading state (`btn(..., { loading })` or `setBusy(button, true)`) that keeps the width and shows an inline spinner. Use it on every button that triggers an API call so double submits are impossible.
- Text inputs, number, search, textarea: same height, radius, border (`--ctl`), background (`--input`), hover and focus (border to `--accent` plus a soft ring), invalid state with a message line, `::placeholder` colour, autofill colours fixed for both themes, number spin buttons hidden with stepper buttons where steps matter (memory limits).
- Select: `appearance: none` with a custom chevron icon, same metrics as inputs; long option lists stay native for accessibility.
- Checkbox and radio: custom box and dot with a check-draw animation (`--dur-2`), using the real input underneath (visually styled, not replaced by divs). Align with labels; whole label clickable.
- Switch (`sw()`): knob slides with `--ease-out`, track colour cross-fades, and a pending state while the API call runs (revert on failure with a toast).
- Segmented control and tabs: an active indicator that slides between items instead of jumping.
- `dialog` (`confirmDialog`, `formDialog`, `keepOrUndoDialog`): backdrop fade and blur, panel scale from .97 to 1, close on Escape, focus trapped and returned to the trigger, the same field styles as pages.
- Toasts: stack, auto-dismiss with a thin progress line, pause on hover, kinds ok, warn, error.
- File inputs (backup restore, uploads): a drop zone with drag-over state instead of the native button.
- Scrollbars (logs, terminal side panels, tables): thin, themed (`scrollbar-color` plus the WebKit fallback).
- Tooltips (`.tip`): fade in with a small delay, never on touch.
- Tables: row hover, sticky header on long tables, numeric columns right aligned.
- Empty states (`.empty`): icon, one line, one action, consistent across pages.

## 4. UI kit page
`web/kit.html` (served only when the panel runs in dev mode or with `?kit` for logged-in Owners; not linked in the nav): every component in every state (default, hover via class, focus, disabled, invalid, loading), both themes side by side, plus a "simulate live data" button that drives tween, flash, list diffing, chart append and a fake task. Use it to review and to catch regressions.

## Rules
- Contrast: text and control borders meet WCAG AA in both themes; focus is always visible.
- Performance on the 1 GB `opsbox-vm` in a normal laptop browser: Overview idle CPU stays low (no continuous animation except the pulse dots, which use CSS only), no layout shift when data arrives (check with the Performance panel), and the CSS stays one file, kept tidy by section.
- No markup copy from other products; follow the design canvas.
- Keep all existing `data-act` and `bindActions` wiring; do not change API calls.

## Done when
- No browser default control visible on any page in either theme (audit list all checked off in `docs/STATUS.md`).
- Every live value from the audit tweens or flashes as specified, and reduced motion turns it all off.
- Every API button shows a loading state and cannot be double submitted.
- After screenshots in `docs/screenshots/16b/after/` next to the before ones.

Out of scope: new pages or features, layout redesign, changing the logo or brand colours, a component framework.
