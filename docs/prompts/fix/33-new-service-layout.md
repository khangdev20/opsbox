# Fix 33. New service page: not full width, ragged Deploy buttons

Follow `docs/prompts/fix/README.md`. Panel only, CSS and markup first. Same width fix as 31 and 32, plus the row alignment left from QA 30.

## Problem
1. At 2576 px the New service content stops at about 1000 css px with big empty margins; the sidebar and header are full width and other pages are wide.
2. Each Deploy button sits right after its row's text, so the buttons form a ragged column; rows are cramped against the separator lines.
3. Rows detected as "nothing to build" still offer an enabled Deploy.

## Steps
1. Files: `web/js/pages/newservice.js`, `web/css/app.css` (`rg -n 'max-width|narrow|list-row' ...`). Remove the narrow cap on this page only, the same way fix 32 did for Cleanup.
2. Test first (Playwright): at 1920 the content area is at least 1400 px; at 1440 and 390 every Deploy button has the same right edge as the search box; rows have at least 12 px vertical padding; at 390 the button goes under the text at 44 px height, no overflow.
3. Make each row a grid: text `1fr`, action auto and right aligned; long names truncate with ellipsis and `title` holds the full name.
4. For "nothing to build" rows show a secondary "Choose build" button (or disable with a short reason), same size so the column stays straight.

## Done when
New service is as wide as Overview at 1920 px, the Deploy column is straight at 1440 and 390 px, and "nothing to build" rows no longer look deployable. Mark QA 30 as fixed.

Commit `fix 33: new service layout`, push `dev`, archive this file.
