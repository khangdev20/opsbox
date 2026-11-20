# Fix 34. Settings page is not full width

Follow `docs/prompts/fix/README.md`. Panel only, CSS first. Same idea as fixes 32 and 33.

## Problem
Settings stops at the old narrow width (about 1000 css px at 2576 px) with big empty margins, while Overview and the other pages are wide.

## Steps
1. Find the cap: `rg -n 'max-width|narrow' web/css/app.css web/js/pages/settings.js`. Use the same container that Overview and the New service Templates tab use.
2. Test first (Playwright): at 1920 the Settings content area is at least 1400 px; at 390, 768 and 1024 nothing overflows and the layout is unchanged.
3. Use the room, without changing any setting or text: on wide screens lay the sections (cards) out in two columns; a section with a long form keeps its fields in a sensible width inside its card (inputs may cap near 560 px, left aligned). Sections stay one column below 1100 px.
4. Header actions (save, switches) align to the right edge of their card or the page content.

## Done when
Settings fills the area beside the sidebar at 1920 px, uses two columns on wide screens, and 390/768/1024 are unchanged.

Commit `fix 34: settings width`, push `dev`, archive this file.
