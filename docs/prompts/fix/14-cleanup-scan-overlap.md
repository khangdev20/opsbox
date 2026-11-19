# Fix 14. Cleanup scans pile up

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 22: opening Cleanup, leaving and reopening starts another `scanCleanup` while the old one runs; two concurrent fresh scans both hit the helper.

## Steps
1. Test: `Promise.all([api.scanCleanup(true), api.scanCleanup(true)])` must produce one request; navigating away cancels the outstanding one.
2. Fix: share one in-flight promise in `api.js` or `cleanup.js`, abort on `dispose`, and ignore a result that arrives after the page was left (`dead` flag already exists).

## Done when
One scan in flight at any time.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 14`. Commit `fix 14: Cleanup scans pile up`, push `dev`, archive this file.
