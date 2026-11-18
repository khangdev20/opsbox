# 16m. Top processes: container or system

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-metrics` (process sampler), `crates/opsbox-api/src/metrics.rs`, `web/js/pages/overview.js` (`procRow`), `docs/API.md`.

Today: Top processes shows only the PID; the design shows "container · pid 1204" or "system · pid 887" and, for containers, which one (16d STATUS: process type not in API).

Do:
- For each sampled process, read `/proc/<pid>/cgroup` (cgroup v2): a Docker scope (`docker-<id>.scope` or `/docker/<id>`) means container; map the id to the container name and opsbox service (cached from the Docker list, refreshed with it). `system.slice/<unit>.service` means system with the unit name. Otherwise `user`.
- API process entries gain `kind` (`container`, `system`, `user`), `container` `{ id, name, service? }` and `unit`.
- UI subline: `container web-app · pid 1204`, `system nginx.service · pid 887`; the row links to the container or the service page.
- Cost: cgroup reads only for the top N after sorting, cached by PID until it exits.

Tests: cgroup fixtures for Docker (systemd and cgroupfs drivers), systemd services, user sessions, and cgroup v1 fallback (kind `unknown`).

Verify on `opsbox-vm`: the greenglish containers show their names; dockerd shows `system docker.service`. Record in `docs/STATUS.md`.
