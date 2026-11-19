# QA 12. Keyboard use and visible focus

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. On Overview, Apps, a dialog and the account menu: Tab order is logical, focus is visible in dark and light themes, Escape closes dialogs and menus, focus returns to the opener.
2. Icon-only buttons have an accessible name (`aria-label`); form fields have labels; errors use `role="alert"`.
3. Run an automated check (axe via Playwright) on 5 pages and list the serious or critical items.
4. Fix missing labels and focus styles.

## Pass when
No critical axe items on the five pages.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 12: Keyboard use and visible focus`, push `dev`, archive this file.
