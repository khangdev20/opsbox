# Status

Last completed prompt: `docs/prompts/archive/16p-stable-live-lists.md` (stable live lists / FLIP). Not committed / not pushed (per session request). Prior: `16o`.

## What works (16p)
- **`morphChildren` / `patch`**: `data-flip` lists remove leavers immediately; movers use FLIP (`translateY`, `DUR[3]`); `prefers-reduced-motion` skips FLIP and enter; moves further than visible list height are placed with no animation.
- **`data-flip` applied**: Overview top processes + containers (top 5); Containers table (`flip: true`); System services + Capacity reservations; notification popover list in `shell.js`.
- **UI kit**: side-by-side flip vs non-flip lists with Shuffle (`web/js/kit.js`, styles in `app.css`).
- **Tests**: `tools/check-flip-ui.mjs` (source placement always; browser timing/reduced-motion/long-move when Chrome is available). Wired in CI. Flip checks removed from `check-notifications-ui.mjs`.

## Partial / Found, not fixed
- **Shell / delete unavailable this session** — could not run `git mv`, `node tools/check-flip-ui.mjs`, or cargo checks. Full text is in `docs/prompts/archive/16p-stable-live-lists.md`; `docs/prompts/16p-stable-live-lists.md` is a stub. On commit: `git rm docs/prompts/16p-stable-live-lists.md && git add docs/prompts/archive/16p-stable-live-lists.md` (or `git add -A docs/prompts`). Run `node tools/check-flip-ui.mjs` before commit.
- **Not verified on `opsbox-vm`**. Not checked at 390 px.
- Carried over: 16o / … pending commit/release; next prompt `16q`.

## Next
- Run `node tools/check-flip-ui.mjs`, then commit 16p (and pending stack if intended). Do not push unless asked.
- Then `16q-import-config.md`.
- VM check for 16p: `yes > /dev/null` for ~20 s while watching Overview Top processes — rows glide, card height stays fixed. Must fail safely: with reduced motion enabled, rows reorder with no glide/fade.
