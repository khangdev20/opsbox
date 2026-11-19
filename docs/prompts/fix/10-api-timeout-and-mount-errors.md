# Fix 10. Requests never time out and a failed System tab stays on the skeleton

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 09 and 13: `web/js/api.js` awaits `fetch` with no deadline, so a hung request waits forever; `web/js/main.js` calls `page.mount(host)` without awaiting it, so a rejected async mount (System tab pane request) bypasses the router's error handling and leaves the loading skeleton.

## Steps
1. Tests: (a) a fetch that never resolves rejects after the deadline with a readable 'The server did not answer in time' error; (b) a page whose `mount` rejects shows the standard error view with Retry.
2. Fix: `AbortController` with a 30 second default (longer for the endpoints that are known slow: updates check, cleanup scan, backups), one place in `api.js`; `await` the mount in `main.js` inside the existing try/catch; in `system.js` catch pane failures and show the error state, not the skeleton.

## Done when
Both tests pass; with the server paused the page shows an error and a Retry button within the deadline.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 10`. Commit `fix 10: Requests never time out and a failed System tab stays on the skeleton`, push `dev`, archive this file.
