# QA 10. Nothing in http mode still uses sample data

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. In `web/js/api.js` list every exported function that is not in `REAL`. For each, decide: needed only in demo mode, or leftover.
2. In http mode, `grep -rn "sample data\|Demo shell\|demo" web/js` for badges and text that can appear on a real server.
3. Load every page against the real API and confirm no 'sample data' badge or fake value shows (host names like srv-sgn-01, fake users, fake audit rows).
4. Remove leftovers that are one-liners; list the rest.

## Pass when
A real server never shows demo content.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 10: Nothing in http mode still uses sample data`, push `dev`, archive this file.
