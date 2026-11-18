# 15b. Capacity guard: what this server can safely run

Files: new module `crates/opsbox-metrics/src/capacity.rs` (or a small `opsbox-capacity` crate if cleaner), `crates/opsbox-priv/src/ops/deploy.rs`, `ops/templates_compose.rs`, `ops/backup.rs`, the databases code from prompt 12, `crates/opsbox-api` (new `capacity.rs`, and the deploy, templates, databases and backups handlers), `install.sh`, `web/js/pages/overview.js` plus the deploy, template, database and backup dialogs. Spec: add a new section 7.19 "Capacity guard" to `docs/BACKEND_SPEC.md` describing what you build.

Problem: opsbox targets small, cheap instances (512 MB to 4 GB RAM). Today nothing stops a user from deploying more apps, databases or on-server builds than the box can hold. The kernel OOM killer then takes out a random process (possibly sshd, Postgres or opsbox itself) and the server becomes unreachable, which defeats "never SSH again". Goal: opsbox knows what the server has, keeps a safety buffer for the OS and itself, shows which features fit, and refuses (by default) any new long-running workload that would eat into the buffer.

## 1. Host profile
Detected at startup and refreshed every 10 minutes (cheap reads only, no `du`):
- RAM total and available (`/proc/meminfo`), swap total and free, zram present.
- CPU: online cores, cgroup v2 CPU quota if opsbox itself runs in a limited VM/container.
- Disk: total and free for `/` and for the opsbox data dir and Docker data root (`statvfs`).
- Docker present and its data root; cgroup v2 available (needed to enforce limits).
- Live pressure: PSI `/proc/pressure/memory` and `/proc/pressure/cpu` (`some avg60`); fall back to available memory if PSI is missing.

## 2. Reserves (the buffer that is never handed out)
Single source of truth: `capacity/requirements.rs`, one table with comments, so numbers can be tuned in one place.
- OS reserve, memory: `max(192 MB, 10% of RAM)`. CPU: 0.25 core.
- opsbox reserve, memory: 128 MB (serve, priv, proxy, watchdog plus margin; check the real peak RSS on `opsbox-vm` and adjust). CPU: 0.1 core.
- Docker engine reserve, when Docker is installed: 150 MB.
- Disk reserve: `max(2 GB, 10% of disk)` kept free on each relevant filesystem.
- Swap counts for at most 50% of its size toward memory capacity, and never for databases.

`allocatable_memory = RAM + swap credit - OS reserve - opsbox reserve - Docker reserve - sum(reservations of running workloads)`.
CPU is compressible: allow up to 2x overcommit of allocatable cores. Memory is not overcommitted.

## 3. Feature requirements
In the same table. Each entry has a minimum and a recommended value. Below the minimum it is blocked; between the two it is allowed with a "tight" warning.

| Feature | Minimum | Recommended |
|---|---|---|
| Install opsbox (panel, files, terminal, services, cleanup, metrics, firewall) | 512 MB RAM total, 2 GB free disk | 1 GB RAM |
| Docker engine and containers | 768 MB RAM total | 1 GB |
| App container, default reservation if the user sets no limit | 128 MB | 256 MB |
| Build on server (deploy pipeline) | 1.5 GB allocatable at build time (swap may count), 5 GB free disk | 2 GB |
| Postgres instance | 256 MB | 512 MB |
| Other templates (Redis, etc.) | Taken from the template manifest: add `min_memory_mb` and `recommended_memory_mb` to the manifest format; a template without them defaults to 128/256 MB | |
| Backup run (restic) | 256 MB headroom when the run starts | |

## 4. Admission control
One function, `capacity::admit(request) -> Fit { Ok | Tight { reasons } | Blocked { reasons, suggestions } }`, used before anything that adds a long-running workload or a heavy job:
- New app deploy, scaling or raising its memory limit, compose or template install, database create or resize: reserve the workload's memory limit (or the default). Every workload opsbox starts must now get a cgroup memory limit (today `memory_limit_mb` is optional and is skipped below 64 MB). If the user sets none, use the default reservation, so reservations are real limits, not wishes.
- Image pull: requires image size plus disk reserve.
- Build on server: requires the build requirement at that moment. If blocked, suggest building in GitHub Actions and pulling the image (prompt 09 path), or adding swap.
- Backup run (scheduled): if blocked, delay it by up to 30 minutes and retry; after that, run it at lowest priority (`nice`, `ionice`) and raise an alert (prompt 15).
- Also block, regardless of reservations, while memory pressure is high (PSI `some avg60` > 20% for 2 minutes, or available memory below the OS reserve). The message says the server is under memory pressure right now.

Blocked results return HTTP 409 with a structured body: what was asked, what is free, which reserve it would break, and suggestions (lower the limit, stop or remove a named workload that uses the most, add swap, upgrade the instance). Reasons carry numbers, for example "needs 512 MB, 310 MB allocatable (1 GB RAM, 192 MB OS, 128 MB opsbox, 150 MB Docker, 256 MB used by api, 0 MB swap)".

Override: an Owner can pass `override: true` with a typed confirmation in the UI ("I understand this server may run out of memory"). It is audited, and the workload is marked "over capacity" in the UI until the server has room again. API tokens and Deployer roles can never override.

## 5. Swap helper
On servers with 2 GB RAM or less and no swap, offer "Add a swap file" (1 GB, or 2 GB if disk free > 20 GB). It is a typed `PrivOp`: fallocate, chmod 600, mkswap, swapon, fstab entry, `vm.swappiness=10`. Show it in the Capacity card and as a suggestion in Blocked results. Never automatic.

## 6. Installer and status
- Preflight in `install.sh`: print the profile in one line (`1 vCPU, 961 MB RAM, no swap, 18 GB free`) and the tier (`small: Docker apps OK, builds on server not recommended`). Below the install minimum: refuse with a clear message; `--force` installs anyway and marks the server as below minimum.
- `opsbox status` adds a `capacity` line: allocatable memory, reserved, pressure.
- `opsbox capacity` (CLI) prints the full table: each feature with ok, tight or blocked and the reason. Useful for agents and for bug reports.

## 7. UI
- Overview: a "Capacity" card with a stacked bar (OS reserve, opsbox, Docker, each workload, free), pressure state, and the swap suggestion when relevant.
- Each dialog that creates a workload (deploy, template, database, memory limit edit) calls `GET /capacity/check?...` as the user types the limit and shows "fits" / "tight" / "doesn't fit" with the reason before submit. A blocked submit shows the same reasons and suggestions; the override checkbox is only shown to Owners.
- Features below minimum are shown disabled with a tooltip ("Build on server needs 1.5 GB free memory; this server has 610 MB. Build in GitHub Actions instead."), not hidden.
- Workloads marked over capacity get a badge on their row.

## API
`GET /capacity` (profile, reserves, reservations, allocatable, pressure, feature table), `GET /capacity/check` (dry run of `admit`), plus 409 bodies from the existing create and update endpoints. Document all in `docs/API.md`.

## Tests
- `admit` table tests: 512 MB, 1 GB, 2 GB and 4 GB profiles with and without swap and Docker; each feature lands in the expected ok, tight or blocked state.
- Reservations add up across existing workloads; removing a workload frees its reservation.
- Pressure gate blocks even when reservations fit (fixture PSI file).
- Every workload created through the API gets a memory limit (deploy, template, database).
- Override: Owner with confirmation succeeds and is audited; Deployer and API token get 403.
- Backup delay and retry when blocked.
- Installer refuses below minimum without `--force` (shell test with fake `/proc/meminfo` via an env override for tests only).

## Verify on `opsbox-vm` (1 GB)
`opsbox capacity` shows a 1 GB profile with builds on server blocked. Deploy one app with 256 MB: fits. Try a Postgres with 512 MB: blocked with numbers and the swap suggestion. Add swap from the panel, then retry the 512 MB Postgres: still blocked (swap does not count for databases). Try 384 MB: tight or fits as expected. Record the numbers in `docs/STATUS.md`.

Out of scope: auto-scaling, moving workloads to other servers (that is opsbox Desktop), per-process CPU pinning.
