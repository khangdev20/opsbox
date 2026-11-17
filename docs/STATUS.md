# Status

Last completed prompt: `docs/prompts/archive/07-docker-containers.md` (Docker and Containers page)

## What works
- `crates/opsbox-docker` + `opsbox-priv` (`ops/docker.rs`, `ops/cleanup.rs`, `ops/facts.rs`, `ops/doctor.rs`): pure-Rust `bollard` Unix-socket Docker Engine client isolated inside `priv`, covering status/install (signed apt/dnf repos with GPG fingerprint verification or distro packages, never convenience scripts), container list/inspect (secret env vars masked)/start/stop/restart/remove, log snapshot and live follow stream, container stats, `DockerExec` primitive, images/volumes/networks listing, restarting-container facts, and `opsbox-docker`-backed Cleanup scanners & pruning.
- `crates/opsbox-metrics`: polls running container stats every 2 s (backing off to 15 s when Docker is missing/stopped), attaches top running containers to `Snapshot`, and emits `"containers"` Attention alerts (`#/containers`) when containers are restarting.
- `crates/opsbox-api` (`docker.rs`, `metrics.rs`, `session.rs`): `/api/v1/docker/{status,install,containers,containers/{id},containers/{id}/{action},containers/{id}/logs,images,volumes,networks}` and `WS /api/v1/stream/containers/{id}/logs`, container stats on `/api/v1/metrics`, audit logging, strict ref/action validation, 501 vs 503 missing/stopped daemon mapping, and read-only `deploy` token scope.
- `web/js`: Containers page wired live (`api.js` `REAL` map updated, sample-data badge removed) with Install Docker / Start Docker cards, Containers/Images/Volumes/Networks tabs, unmanaged container display/control, confirmation dialogs on `stop`/`restart`/`delete`, inspect modal, live streaming logs, and running container stats + restarting alerts on Overview.

## Partial / Found, not fixed
- Linux-only runtime/CI checks (`tools/check-idle-memory.py`, `musl` static build, `cargo-deny`, real `/var/run/docker.sock` Engine daemon, and real apt/dnf signed Docker installation) cannot run natively on macOS and remain for Linux CI / `opsbox-vm`.

## Next
- Verify missing-Docker install, Start Docker, managed/unmanaged containers, live WS logs, Inspect, Cleanup Docker scanners, and Overview container stats/attention in `opsbox-vm`.
- Next prompt: `docs/prompts/08-github-connection.md`.
