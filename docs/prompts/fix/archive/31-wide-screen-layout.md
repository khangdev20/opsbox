# Fix 31. Use the width of large screens

Follow `docs/prompts/fix/README.md`. Panel only, CSS first.

## Problem
At 1800 px wide the page content stops at about 1100 px: roughly 230 px of empty space on each side of Overview, while charts and tables feel cramped. Find the cap (`rg -n 'max-width' web/css/app.css`, probably on the page container).

## Steps
1. Test first: Playwright at 1440, 1920 and 2560 wide. Assert the page container is at least 1400 px at 1920 and no wider than 1800 px at 2560. Also assert no horizontal scroll at 390, 768 and 1024.
2. Replace the fixed cap with a fluid one: `width: min(100%, 1680px)`, side padding `clamp(16px, 2.5vw, 40px)`, centred.
3. Keep readable widths for text-heavy pages (Settings forms, Security text, dialogs): cap those inner blocks at about 880 px, left aligned, so lines do not run too long.
4. Let grids use the room: stat cards stay 4 across, chart and Top processes keep 2 columns but grow; tables stretch. Do not add new columns or change data.

## Done when
At 1920 px Overview fills the area beside the sidebar with about 40 px margins, forms stay narrow, and 390/768/1024 look unchanged.

Commit `fix 31: wide screen layout`, push `dev`, archive this file.
