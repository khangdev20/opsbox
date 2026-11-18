# 16s. Make the panel work well on phones and tablets

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `web/css/app.css` (shell and sidebar rules, `@media` blocks), `web/js/shell.js` (header, nav, drawer), `web/js/ui.js` (`table`, dialogs), every page in `web/js/pages/`, `web/kit.html`, `design/` (add mobile boards). Backend unchanged.

Problem: the panel is built for desktop. Below 900 px the sidebar simply turns into a wrapping row of every nav link, which pushes the content far down; tables, dialogs, the terminal and the file manager were not designed for narrow screens. People will open the panel from a phone to check a server or restart an app, so this must be good, not just "not broken".

## 1. Navigation on small screens
- Up to 900 px: the sidebar becomes a **drawer** opened by a menu button in the top bar (left of the brand). It slides in from the left over a dimmed backdrop, closes on backdrop tap, Escape, route change and swipe-left. Focus is trapped while open and returns to the button on close. No wrapping nav row any more.
- The collapse toggle and the 72 px collapsed mode apply only above 900 px.
- Keep the top bar to: menu button, brand, notification bell (16f), account menu. The command palette button is already hidden under 640 px; give it an icon-only button from 641 to 900 px.
- Use `100dvh` (not `100vh`) for the drawer and sheets, and `env(safe-area-inset-*)` padding at the top, bottom and sides so nothing hides under the notch or home bar. Add `viewport-fit=cover` to the viewport meta in `web/index.html` and `web/kit.html`.

## 2. Layout rules
- No horizontal scroll on the page at any width from 320 px up. Content padding 16 px under 700 px.
- Grids collapse to one column (Overview already does at 1100 px; check every other page: Apps, Containers, Databases, Backups, Alerts, Settings, Security, System tabs, Capacity if it exists).
- Page headers: title and actions stack; actions wrap or move into an overflow menu. Tabs (System) scroll horizontally with a visible fade at the edge instead of wrapping.
- Touch targets at least 44 x 44 px for buttons, icon actions, row actions, checkboxes and nav items on `(pointer:coarse)`. Do not change desktop density.
- Inputs use 16 px font on touch devices so iOS does not zoom on focus.

## 3. Tables
- Default for data tables under 700 px: each row becomes a **card** (first column as the title, the rest as label/value pairs, row actions in a menu at the top right). Implement once in the shared `table` helper with an opt-in `data-label` per column, so pages do not each hand-roll it.
- Wide technical tables (processes, audit log, backups list, file listings) may instead scroll horizontally inside their own container, with the first column sticky. Pick per table; never let the page itself scroll sideways.
- Keep `data-flip` and keyed morphing working in card mode (16p).

## 4. Dialogs, menus and popovers
- Dialogs under 640 px become **bottom sheets** (full width, max 90 dvh, rounded top corners, internal scroll, sticky action bar at the bottom, drag handle optional). Confirmation dialogs stay compact.
- Dropdown menus and the account menu stay anchored but never overflow the viewport; the notification panel already becomes a top sheet at 640 px, keep it consistent.
- Toasts sit above the bottom safe area and never cover primary actions.

## 5. Specific screens
- **Terminal**: usable with the on-screen keyboard (resize on `visualViewport`, keep the prompt visible), plus a small key bar for Esc, Tab, Ctrl, arrows.
- **Files**: list view with a path breadcrumb that scrolls; editor takes the full screen; no side-by-side panes under 900 px.
- **Overview**: KPI tiles 2 per row from 520 px, 1 per row below; charts keep a readable height, tooltips work on tap.
- **Deploy, database, backup wizards**: one step per screen, Next/Back in the sticky action bar.
- **Login, setup, enroll, invite**: already split-page; verify the QR code and TOTP input fit at 320 px.

## 5b. Design
Add a "Mobile" board to `design/` (Overview, an Apps list in card mode, the drawer open, a bottom-sheet dialog) in the same style, and a short note in 16d's table that the panel now follows it.

## Tests and checks
- Playwright (or the existing browser test setup) loads each page at **360, 390, 768 and 1024 px** using the mock API and asserts: `document.documentElement.scrollWidth <= innerWidth`, the drawer opens and closes with the button, Escape and a route change, dialogs are sheets under 640 px, and no interactive element on a touch viewport is smaller than 44 px. Keep these tests in the repo and runnable with one command.
- Screenshots of Overview, Apps, Containers, System, Terminal, one dialog and the drawer at 390 px and 768 px in `docs/screenshots/16s/`.
- Kit page shows the card-mode table, bottom sheet and drawer.
- Reduced motion: drawer and sheets respect `prefers-reduced-motion` (no slide).

## Verify on `opsbox-vm`
Open the panel from a real phone (or Chrome DevTools device mode at 390 px) over the SSH tunnel or public URL: open the drawer, switch pages, restart a container from a card row, open a dialog as a sheet, use the terminal with the keyboard up. Nothing scrolls sideways. One check that must fail safely: open the drawer, then press Back; the drawer closes and the app stays on the current page without logging out.
