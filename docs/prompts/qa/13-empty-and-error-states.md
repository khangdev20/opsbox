# QA 13. Empty and error states on every page

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. With a fresh install (no apps, no databases, no backups) open every page: each shows a clear empty state with the next action, not a blank area or a spinner forever.
2. Then stop `opsbox-priv` and reload each page: each shows a readable error or the banner, never raw JSON or `undefined`.
3. Throttle the network to slow 3G in DevTools: loading states appear, nothing jumps.

## Pass when
No blank, infinite spinner or raw error on any page.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 13: Empty and error states on every page`, push `dev`, archive this file.
