# Fix 12. Missing empty-state guidance on Files, System and Backups

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 13: empty lists on Files (empty folder), System (no pending updates, jobs, units filter with no match) and Backups (no snapshots) show a blank area or only a header.

## Steps
1. List each empty case in a short DOM test (render with empty data) and assert a message plus a next action exists, using the shared `empty()` helper.
2. Add the messages in plain language: 'This folder is empty. Upload a file or create one.', 'Everything is up to date.', 'No snapshots yet. Run a backup now.' with the matching button.

## Done when
All empty cases render a message and an action at 390 px and 1440 px.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 12`. Commit `fix 12: Missing empty-state guidance on Files, System and Backups`, push `dev`, archive this file.
