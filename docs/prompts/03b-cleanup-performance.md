# 03b. Cleanup page performance

Files: `crates/opsbox-priv/src/ops/cleanup.rs`, `crates/opsbox-api/src/cleanup.rs`, `web/js/pages/cleanup.js`, `web/js/api.js` (`scanCleanup`, `runCleanup`). Spec: 7.11.

Problem: the Cleanup page is very slow to open. Findings from a code review:
1. `system_bytes()` runs `du -sxk /usr` on every scan, only to draw the grey "System" segment of the disk bar.
2. `scan()` runs every scanner one after another (journal, package cache, release slots, /tmp, core dumps, then Docker), so the wait is the sum of all of them.
3. Docker: `docker info`, then `docker system df` (slow: sizes volumes and build cache; 120 s timeout), then `docker images --filter dangling`; a run calls `docker system df` twice more.
4. `stale_files()` walks `/tmp` and `/var/tmp` recursively with no depth or entry limit and a `stat` per entry.
5. The UI awaits the whole scan before rendering anything (blank page), has no cache, and rescans after every run and on every visit.

Step 1, measure first: add per-scanner timing (tracing span with duration) and `opsbox cleanup scan --timings` (CLI, runs the priv scan and prints each scanner and its duration). Record the before numbers on `opsbox-vm` in `docs/STATUS.md`.

Step 2, backend:
- Remove `du /usr` from the scan path. Compute "system" bytes in a background job at most once a day with low IO priority (`ionice -c3`, `nice`), store it in priv's state dir, and return the cached value (or none, and the UI omits that segment).
- Run scanners concurrently (`std::thread::scope`), each with its own deadline (3 s for local scanners). A scanner that misses its deadline is reported as `pending` with no size instead of holding up the others; it keeps running and its result goes into the cache.
- Cache the scan result in priv for 60 s; any cleanup run, or a `?fresh=1` request, invalidates it.
- Docker: run `docker info` once and reuse the result; get dangling images and build cache sizes without `docker system df` where possible (`docker image ls --filter dangling=true --format '{{.Size}}'`, `docker builder du` or the Engine API if prompt 07 has landed); compute volume sizes only when the user expands the volumes item (separate on-demand call). For "freed" after a run, use what the prune command reports instead of re-running `system df`.
- Bound the /tmp walk: max depth 8, max 50 000 entries per root, use `DirEntry::metadata` rather than a second `symlink_metadata` on a new path. If the limit is hit, report the item with "at least X" wording.

Step 3, API and UI:
- `GET /cleanup/scan` returns within about 300 ms: cached or finished items plus `pending: true` for ones still running. The UI polls (or uses the task stream) until none are pending.
- `cleanup.js`: render the page shell immediately (header, auto-clean switch, disk bar from the latest metrics snapshot, skeleton rows), then fill each item as it arrives. Show "scanned 12 s ago · Rescan" from the cache age.
- After a run: update the cleaned items in place from the run result (freed bytes, errors), then refresh in the background; no blocking full rescan.

Targets on `opsbox-vm` (no Docker): shell visible under 100 ms, first items under 500 ms, full scan under 2 s cold and under 200 ms when cached. With Docker installed: full scan under 5 s cold. Put the before and after numbers in `docs/STATUS.md`.

Tests: a scanner that sleeps past its deadline is reported pending and does not delay the others; cache is invalidated by a run; the /tmp walk stops at its limits; parse tests for any new Docker output formats.

Out of scope: new scanners, UI redesign, Docker Engine API integration beyond what prompt 07 already provides.
