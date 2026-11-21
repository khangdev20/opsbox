# Fix 35. Capacity check returns one long string and repeats the disk check 4 times

Follow `docs/prompts/fix/README.md`. Rust only (`crates/opsbox-api/src/capacity.rs` and the capacity types it uses; find with `rg -n 'allocatable|disk reserve' crates`). Fix 36 draws the new card from this data.

## Problem
A blocked build says: "Needs 1536 MB, 695 MB allocatable (...) · / needs 7168 MB free including 2048 MB disk reserve; 3484 MB free · /var/lib/opsbox needs ... · /var/lib/opsbox/apps needs ... · /var/lib/docker needs ... · Lower the memory limit ... · Stop or remove greenglish-dev-worker-1 ...". All four paths are the same disk, so the same line repeats four times, and reasons, numbers and actions are mixed in one text.

## Steps
1. Test first: a capacity check where `/`, `/var/lib/opsbox`, `.../apps` and `/var/lib/docker` share one device must produce ONE disk check listing all four paths; two different devices produce two checks.
2. Group disk checks by device id (`st_dev`). Each check: `{ kind: "memory"|"disk", need_mb, have_mb, ok, paths: [..], reserve_mb }`.
3. Add to the response `checks: [...]` and `actions: [{ id, label, effect_mb?, target? }]` with ids `lower_limit` (suggested value in MB), `stop_service` (service name, frees MB), `build_in_ci`, `upgrade_instance`, `cleanup_disk`. Keep the existing text field unchanged for old clients, but build it from the grouped checks (the disk sentence appears once).
4. Memory check keeps its breakdown as a list `breakdown: [{label, mb}]` (RAM, OS, opsbox, Docker, workloads, swap credit) instead of text.

## Done when
The test passes, the old text shows the disk once, and the JSON carries `checks`, `actions` and `breakdown`. No UI change in this prompt.

Commit `fix 35: capacity result structured`, push `dev`, archive this file.
