# Fix 32. Cleanup page still uses the old narrow width

Follow `docs/prompts/fix/README.md`. Panel only, CSS first. Other pages are fine; touch only Cleanup.

## Problem
After fix 31 every page is wide except Cleanup: it stops at about 880 px with big empty margins, and the "Weekly auto-clean" switch sits outside the right edge of the list.

## Steps
1. Find the cap: `rg -n 'max-width|narrow' web/css/app.css web/js/pages/cleanup.js`. It is probably a page-specific class or inline style on the Cleanup container.
2. Test first: Playwright at 1920 wide on Cleanup: the content area is at least 1400 px and the switch's right edge equals the list's right edge. Overview must stay as it is.
3. Remove the narrow cap from Cleanup only and make the header row use the same container as the list.

## Done when
Cleanup matches the width of Overview at 1920 px, the header switch aligns with the list, and 390/768/1024 are unchanged.

Commit `fix 32: cleanup page width`, push `dev`, archive this file.
