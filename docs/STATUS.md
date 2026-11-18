# Status

Last completed prompt: `docs/prompts/archive/16s-responsive-panel.md` (responsive panel). Not committed / not pushed (per session request). Prior: `16r`.

## What works (16s)
- **Drawer nav** under 900 px: menu button left of brand, backdrop, focus trap, Escape / route / backdrop / swipe-left / Back close; collapse toggle desktop-only. `100dvh` + `env(safe-area-inset-*)`; `viewport-fit=cover` on `index.html` / `kit.html`.
- **Palette** icon-only 641–900 px; hidden under 640. Layout: no page sideways scroll from 320 px, 16 px padding under 700, 44×44 on `pointer:coarse`, 16 px inputs on touch.
- **Tables**: shared `table()` card mode (`data-label`) under 700 px; sticky first column + internal scroll for Files / audit / backups / updates.
- **Dialogs**: form dialogs bottom sheets under 640 px; confirms stay compact. Files editor fullscreen under 900; Terminal `visualViewport` + Esc/Tab/Ctrl/arrows key bar; Overview KPIs 2/row from 520, 1 below.
- **Kit / design**: card table, drawer, sheet demos; `design/project/Mobile.dc.html` board. Check: `node tools/check-responsive-ui.mjs`.

## Partial / Found, not fixed
- Screenshots for `docs/screenshots/16s/` not captured this session (run screenshot tooling or DevTools at 390/768).
- Shell was unavailable this session — could not run `node tools/check-responsive-ui.mjs` or `git mv`. Archive copy is at `docs/prompts/archive/16s-responsive-panel.md`; remove live `docs/prompts/16s-responsive-panel.md` with `git mv` (or `git rm` + add) before commit.
- Wizard sticky bars: configure deploy uses `.wiz-actions`; other multi-step wizards still use normal rows (sheets cover form dialogs).
- Not verified on a real phone / `opsbox-vm` tunnel this session.
- Carried over: pending commit/release for 16o–16s (commit only when asked).

## Next
- Run `node tools/check-responsive-ui.mjs` (and cargo fmt/clippy/test if touching Rust).
- Then commit (only when asked): `prompt 16s: responsive panel`. Do not push unless asked.
- Then `17-alpha-release.md`.
- VM check: open panel at 390 px (or phone) — drawer, card rows, sheet dialog, terminal with keyboard up; no sideways scroll. Must fail safely: open drawer, press Back — drawer closes, stay on page, no logout.
