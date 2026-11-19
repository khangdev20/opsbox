# Fix 11. Apps page can reload repeatedly when the domain request fails

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 13: in `web/js/pages/apps.js` (around the domain loading code) a failed domain request triggers a re-render that requests again, in a loop.

## Steps
1. Reproduce with a stub where the domain endpoint returns 500: count requests over 3 seconds; expect at most one retry per user action.
2. Fix: remember the failure per service id, show an inline error with a Retry button, and only refetch on that click or on the next poll interval.

## Done when
At most one request per interval while failing; the error and Retry are visible.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 11`. Commit `fix 11: Apps page can reload repeatedly when the domain request fails`, push `dev`, archive this file.
