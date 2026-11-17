# Status

Last completed prompt: `docs/prompts/archive/03b-cleanup-performance.md` (Cleanup page performance)

## Current state
- Per-scanner `tracing` timings and `opsbox cleanup scan --timings` CLI command added (`opsbox-priv`, `opsbox`).
- Synchronous `du -sxk /usr` removed from scan path; system bytes computed at most once per 24h via low-IO background `nice -n 19 ionice -c3 du -sxk /usr`, cached in `/var/lib/opsbox/cleanup-system-bytes.json`.
- Scanners run concurrently with per-scanner deadline; `GET /api/v1/cleanup/scan` returns cached/completed items + `pending: true` placeholders (~250ms budget), caches for 60s, invalidates on `cleanup_run` or `?fresh=1`.
- Docker scanning optimized (`docker info` once, `docker image ls --filter dangling=true`, `docker builder du --format '{{json .}}'`, `docker ps -a -s --filter status=exited --filter status=created`), unused volume count without `docker system df`, on-demand `GET /api/v1/cleanup/volumes`, prune parses `"Total reclaimed space:"`.
- `/tmp` and `/var/tmp` walk bounded to depth 8 / 50,000 entries per root using `DirEntry::metadata()`.
- Cleanup UI renders immediately with skeleton rows, polls while `pending`, shows `"scanned X ago · Rescan"`, omits System bar segment when `system === 0`, adds `"Calculate size"` for `docker-volumes`, updates cleaned items in place.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `python3 tools/generate-api.py --check` pass locally.

## Next
- Verify `sudo opsbox cleanup scan --timings` and the Cleanup UI on the real Linux instance (`almanac-sensor`).
- Next prompt: `docs/prompts/05-proxy-panel-domain.md`.
