# 03c. Proxy port conflicts handled automatically

Files: `crates/opsbox-proxy` (listener startup), `crates/opsbox/src/cmd/proxy.rs`, `crates/opsbox/src/config.rs` (`[proxy]`), `crates/opsbox-watchdog/src/policy.rs`, `crates/opsbox/src/cmd/status.rs`, `install.sh`, `crates/opsbox-api` and `web/js/pages/` (Overview or Settings > Network). Spec: 7.18, 6.3.

Problem seen on the first real install (Ubuntu 26.04 x86_64, alpha.13): a Docker container already published ports 80 and 443 (`docker-proxy`). `opsbox proxy` exited with "Address in use", systemd restarted it every 2 s, the watchdog counted 3 failures and entered the safe mode alert state, and the installer ended with "did not become healthy" pointing only at `opsbox` and `opsbox-priv` logs. The user had to SSH in and edit config by hand. Goal: an install or update on a busy server finishes cleanly, the panel explains the situation, and the user fixes it from the UI; no SSH, no manual config.

## 1. Proxy: never crash-loop on a busy port
- On startup, try to bind the configured ports. If a bind fails with EADDRINUSE (or EACCES), do not exit. Enter a `port_conflict` state:
  - Bind the fallback ports instead (`fallback_http`, `fallback_https` in `[proxy]`, defaults `8080` and `8445`; if those are busy too, pick the next free port in 8080–8099 / 8445–8464). Serve normally there, so app routes keep working for testing through `http://IP:8080`.
  - Keep the control socket up, so `status` and the API report "running, conflict" instead of "DOWN".
  - Retry the primary ports every 30 s. As soon as they are free, bind them, drop the fallback listeners after in-flight requests finish, and leave `port_conflict`. No restart, no user action.
- Identify the holder for each busy port: read `/proc/net/tcp{,6}` for the listening inode, map it to a PID through `/proc/*/fd` (done in priv; proxy asks priv over the existing socket or priv pushes it), then the process name. If the process is `docker-proxy`, find the container with `docker ps --filter publish=80` (name, image, id). Report: `{port, pid, process, container?: {id, name, image}}`.
- A real crash (panic, bad config) still exits non-zero as today.

## 2. Watchdog: a port conflict is not a failure
- The watchdog reads the proxy status. `port_conflict` is a warning: no restart counting, no safe mode, no rollback. It shows as an alert (prompt 15 delivers it as a notification when that lands).
- If an update leads straight to `port_conflict` while the previous version had the ports, that is still not a reason to roll back (the conflict comes from the host).

## 3. Installer and `opsbox update`
- Before enabling units, check 80 and 443 with `ss -ltnpH` (fallback: `/proc/net/tcp`). If busy, print one clear line per port, for example: `port 80 is used by docker-proxy (container "web", image nginx:1.27); the opsbox proxy will use :8080 until it is free`. Continue the install.
- The health wait treats proxy `port_conflict` as healthy-with-warning: exit 0, print the warning and where to fix it (`panel > Settings > Network`).
- On any real failure, the error points at the units that actually failed (`journalctl -u opsbox-proxy -n 50`, etc.), not a fixed list.
- `opsbox status` prints `proxy  conflict  :80 used by docker-proxy (web), serving on :8080`.

## 4. Panel: fix it in one click
A banner on Overview and a card in Settings > Network when `port_conflict` is active. It names the holder and offers:
- **Let opsbox take ports 80/443** (only when the holder is a Docker container): confirm dialog naming the container, then priv stops it and sets its restart policy to `no` (`docker update --restart=no`), so it does not come back on reboot. Record the old restart policy in the audit log and offer **Undo** (start it again, restore the policy, proxy goes back to fallback). Never remove the container or its volumes.
- **Keep it and use other ports**: saves the current fallback ports as the configured ports (`listen_http`/`listen_https`), stops retrying, and explains that automatic HTTPS for domains needs 80/443.
- If the holder is not a container (nginx, apache, caddy as a system service): show the service name (from the PID's cgroup, `systemd` unit) and offer **Stop and disable <unit>** with the same confirm, audit and undo. Unknown process: show PID and name only, no action button.
- All actions are typed `PrivOp`s with tests; no shell strings built from container or unit names.

## Tests
- Proxy with the primary port pre-bound: does not exit, serves on fallback, reports the holder, moves to the primary port within one retry after it is released.
- Watchdog policy: `port_conflict` never increments failure counts or enters safe mode; real exits still do.
- `/proc/net/tcp` parsing (v4 and v6) and docker-proxy → container mapping with fixtures.
- Installer: a shell test (or CI job) that binds :80 with a dummy listener, runs the installer, and expects exit 0 with the warning line.
- API: take-over and undo round trip with a fake docker, audited.

## Verify on `opsbox-vm`
Run `docker run -d --name web -p 80:80 -p 443:443 nginx`, then install or update opsbox. Expect: installer exits 0 with the warning, `opsbox status` shows `conflict`, watchdog is not in safe mode, the panel banner names container `web`. Click "Let opsbox take ports 80/443": nginx stops, proxy is on :80 within 30 s, `docker inspect web` shows restart policy `no`. Click Undo: the container is back, proxy is on the fallback. Record the results in `docs/STATUS.md`.

Out of scope: migrating the old container's sites or certificates into opsbox, sharing ports 80/443 with another proxy (SNI passthrough).
