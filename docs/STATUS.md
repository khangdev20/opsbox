# Status

Last completed prompt: `docs/prompts/archive/03e-update-check-reasons.md` (say why nothing is offered). Not committed, not tagged yet; 03d is also still uncommitted. Prior: `03c1` (site only), `16f` (tag `v0.1.0-alpha.20`).

## What works (03e)
- `opsbox-update`: `ReleaseChecker` (injectable fetcher and clock) returns a `CheckResult` with `reason`: `no_releases_on_channel`, `up_to_date` (newest older or equal), `network_error` (+ `detail`), `rate_limited` (+ `retry_at`), `dev_build`; `None` when an update is offered. A failed lookup is a result, not an error.
- GitHub: `User-Agent` sent; the release list is cached for 1 hour (shared by both channels); a 403/429 with rate-limit headers backs off until `X-RateLimit-Reset`/`Retry-After` (1 min to 1 h clamp), and a 403 without them is a plain network error. Dev builds never ask GitHub.
- API: `/updates/self` adds `reason`, `reasonDetail`, `retryAt`, `checkedChannel` (with `checkedAt`). `POST /updates/check` ("Check now") bypasses the cache, not a backoff. A failed check keeps an offer found earlier on the same channel. Changing the channel re-evaluates from the cache. The daily loop retries failed lookups hourly. `apply` with no version and the CLI `opsbox update` give the reason instead of "up to date".
- Settings: each reason has its own text (`web/js/update-status.js`, e.g. "No stable release yet; switch to beta to get alpha releases"), plus "last checked HH:MM · <channel> channel". Tests: 13 new Rust tests (unit and `tests/update_http.rs`) and `tools/check-update-status.mjs` (added to CI).
- Verified here: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test -p opsbox-update -p opsbox-api`, `node tools/check-update-status.mjs`, `generate-api.py --check`. Not run here: full workspace tests, 390 px browser check, the `opsbox-vm` steps below.
- To verify on `opsbox-vm`: set the channel to stable (see "No stable release yet"), back to beta (newest alpha or "up to date" with the last checked time), block outbound HTTPS briefly (see the network error text), press Check now twice within an hour (second one still asks GitHub). Not done yet.

## What works (03d)
- New crate `opsbox-version`: `build.rs` picks the version. `OPSBOX_BUILD_VERSION` (the tag without `v`) wins; otherwise `0.1.0-dev+<sha>`. `opsbox --version`, `status`, `/healthz`, `/updates/self`, priv/proxy/watchdog `Ping` and slot names all use it.
- Release workflow: `tools/stamp-version.sh <tag>` sets the workspace version and `Cargo.lock`, builds with `OPSBOX_BUILD_VERSION`, and fails if `opsbox --version` is not `opsbox <version>` (x86_64 run; aarch64 string check). CI repeats this with `v9.9.9-alpha.1` and restores the files.
- `select_release`: a newer alpha is seen on beta; stable with only pre-releases finds nothing; an old plain `0.1.0` build is offered the newest `0.1.0-alpha.N` once (`migration`, with a note); dev builds get nothing. An unchosen channel still migrates old builds (`select_release_default_channel`).
- Channel order: panel setting, then `[update] channel` in config, then default (beta for a pre-release, else stable). `/updates/self` returns `channelSource`, `devBuild`, `migration`, `migrationNote`; Settings shows the source and the note.
- `install.sh --channel beta` (or a pre-release `--version`) writes `[update] channel = "beta"`; stable writes nothing. Invalid `--channel` is refused.
- Manual path kept: `sudo opsbox update --version v0.1.0-alpha.<N>` skips the comparison (CLI and API `apply`). `docs/RELEASING.md` and `docs/CONFIG.md` updated.

## Verified
- `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -D warnings`, `cargo test --workspace --locked` (607 passed), `tools/generate-api.py --check`, `shellcheck` on `install.sh` and the new tools, `tools/check-installer-channel.sh`.
- Local stamping dry run with `v9.9.9-alpha.1`: `opsbox --version` printed `opsbox 9.9.9-alpha.1`; an unstamped build printed `opsbox 0.1.0-dev+<sha>`.

## Partial / Found, not fixed
- **Not yet verified on real servers.** The prompt's VM checks (`opsbox-vm`, `almanac-sensor` at alpha.13 offered `v0.1.0-alpha.21` via migration, update from the panel, then `alpha.22` normal path, rollback) need the tag and a release; record them here once run.
- `opsbox update` (CLI) does not read the panel's channel setting (it is in the database); it uses `--channel`, then config, then the default.
- Settings mock data in `web/js/api.js` still says `current: '0.1.0'` (sample mode only).
- Earlier open items remain: screenshots use demo data, live DB/R2/MinIO/SFTP/alert-channel checks, pgweb path prefix, Capacity override click-through, idle browser CPU profiling, Caddy/Traefik live cutovers, the `capacity_http` queued-workloads fixture issue (see DECISIONS).

## Next
- Commit, push, wait for CI, tag `v0.1.0-alpha.21`, then run the VM verification for 03d and 03e.
- Then `16e-transport-security.md`, `16g-capacity-page.md` and the rest of the 16* series, then `17-alpha-release.md`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`.
