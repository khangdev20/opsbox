# Status

Last completed prompt: `docs/prompts/archive/15b-capacity-guard.md` (tag `v0.1.0-alpha.17`).

## What works
- Shared `opsbox-capacity` policy (`admit` / reserves / feature table). Host reads and swap live only in `priv` (`Capacity`, `CapacityImageSize`, `CapacityAddSwap`).
- Workload create/resize/deploy/compose/template/database pooler/extension/upgrade, Docker start/restart, image pull and backups go through one admission guard. Defaults become real cgroup memory limits; builds use a 1536 MB BuildKit driver; restic runs low-priority with a 256 MB scope when systemd is present.
- Owner-only typed override (`admin` session), Deployer/token 403, audit + over-capacity badge. Missing cgroup v2 cannot be overridden. Scheduled backups retry 30 minutes then run and alert.
- Installer prints profile/tier, refuses below 512 MB / 2 GB free without `--force`. `opsbox status` shows allocatable/pressure; `opsbox capacity` prints the feature table. Overview Capacity card and deploy/template/database dialogs dry-run `/capacity/check`.

## Verified
- `cargo fmt --all --check`; `python3 tools/generate-api.py --check`; `shellcheck --shell=sh --severity=warning install.sh`.
- `bash tools/check-capacity-install.sh`; `node tools/check-capacity-ui.mjs`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked`; `cargo build --release --locked`.
- Mock Docker now handles `POST .../update` used to enforce memory on start/restart.
- `opsbox-vm` (Ubuntu 24.04 arm64, ~3901 MB RAM): release build needed temporary 3 GB swap + `CARGO_BUILD_JOBS=1` (full LTO SIGKILL otherwise); temp swap removed afterward. Installer: `2 vCPU, 3901 MB RAM … standard`. With conflict-test `web` capped to 128 MB: `capacity 3105 MB allocatable, 796 MB reserved, pressure normal`; App/Database/Backup ok; Build blocked on disk headroom (needs ~8 GB free incl. reserve, ~5.5 GB free).
- 1 GB policy simulation: empty allocatable 554 MB; build blocked; app 256 MB ok; after that app, Postgres 512 MB blocked (`298 MB allocatable`, swap suggestion); after 1 GB swap, DB allocatable stays 298 MB so 512/384 still blocked (swap never counts for databases). Installer fake meminfo: 256 MB refuses; 1 GB tier `small: Docker apps OK, builds on server not recommended`.

## Partial / Found, not fixed
- Conflict-test `web` left at 128 MB after capacity checks (`docker update --memory 0` did not clear the limit). Unlimited containers still reserve all RAM by design.
- Browser click-through of Capacity card/override remains manual. Earlier live DB/R2/MinIO/SFTP/alert-channel and pgweb path-prefix checks still pending. cargo-deny not installed on this Mac; CI retains license/advisory and musl builds.
- Concurrent `design/*` working-tree edits are untouched and unstaged.

## Next
- Prompt `17` alpha release; finish earlier live integration checks.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target CARGO_BUILD_JOBS=1 cargo build --release` (add temp swap if LTO is killed); `sudo sh install.sh --dev --binary ~/target/release/opsbox`; `sudo opsbox capacity`; `sudo opsbox status`.
- Open panel via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`. A Deployer override or API-token override must be refused (403).
