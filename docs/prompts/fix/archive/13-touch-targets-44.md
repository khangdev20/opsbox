# Fix 13. Touch targets below 44 px

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 11: coarse-pointer checkboxes and radios are 22 x 22 px (`web/css/app.css` near the `pointer:coarse` block); file selection (`files.js`) and package selection (`system.js`) rows have no larger hit area.

## Steps
1. Test: with `pointer:coarse` emulated, every `input[type=checkbox|radio]` and its label row has a hit area of at least 44 x 44 px.
2. Fix in CSS only: keep the visible box at 22 px but give the label or a pseudo-element a 44 px hit area; make selectable table rows tappable across the row; do not change desktop density.

## Done when
The test passes at 390 px and desktop screenshots are unchanged.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 13`. Commit `fix 13: Touch targets below 44 px`, push `dev`, archive this file.
