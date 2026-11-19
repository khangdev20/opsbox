# QA 11. Phone widths: no sideways scroll, targets big enough

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Playwright at 360 and 390 px against the real API: for every route assert `scrollWidth <= innerWidth`, the drawer opens and closes, dialogs are bottom sheets, interactive elements are at least 44 px high on touch.
2. Test with the on-screen keyboard emulated on Terminal and a form page.
3. Take screenshots of any failure into `docs/screenshots/qa11/`.
4. Fix CSS-only problems; a table that needs a card layout is 'Found, not fixed'.

## Pass when
No page scrolls sideways at 360 px.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 11: Phone widths: no sideways scroll, targets big enough`, push `dev`, archive this file.
