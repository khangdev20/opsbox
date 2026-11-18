# Status

Last completed prompt: `docs/prompts/archive/03d-self-update-versions.md` (self-update versions and channel). Not committed, not tagged yet. Prior: `03c1` (site only), `16f` (tag `v0.1.0-alpha.20`).

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
- Commit, push, wait for CI, tag `v0.1.0-alpha.21`, then run the VM verification above. `03e-update-check-reasons.md` is next in the update series.
- Then `16e-transport-security.md`, `16g-capacity-page.md` and the rest of the 16* series, then `17-alpha-release.md`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`.
