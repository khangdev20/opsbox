# opsbox backend specification

Status: draft, not implemented. Companion to `ARCHITECTURE.md` (the decisions) and `API.md` (the mock contract in `web/js/api.js`).

This document says what the Rust backend must do and how to build it. It does not contain code.

> **Scope change, 2026-11-17.** The web panel on one instance is the product and is on by default (see `ARCHITECTURE.md`). The desktop app, WireGuard, pairing and multi-server features are a separate, later product ("opsbox Desktop"). Sections that describe them (6.2 device auth, 7.17, 8, Phase 6) are kept for reference but are out of scope for the opsbox MVP.

---

## 1. Goals and non-goals

**Goals**

1. One static binary, `opsbox`, installed by one command on any systemd Linux (x86_64, aarch64).
2. After install, nothing about the server needs SSH: apps, containers, system, files, terminal, backups, alerts, security.
3. Safe by default for people who do not know security: the panel is reachable from the browser, but only after a claim with the setup token, with mandatory 2FA, lockout and audit; the first-run flow pushes the user to attach a domain with HTTPS. `--local-only` keeps it on localhost for users who prefer an SSH tunnel.
4. Small: idle RSS target under 40 MB for the agent, under 10 MB for the watchdog. CPU near zero when idle.
5. Survives its own failure: apps keep running if opsbox dies; a watchdog restores opsbox.
6. The existing UI in `web/` works unchanged; only `web/js/api.js` swaps its mock for real calls.

**Non-goals for v1**

- Kubernetes, Swarm, or scheduling across servers. Each server runs its own workloads. (Multi-instance belongs to opsbox Desktop, later.)
- High availability of the control plane.
- Windows or macOS servers.
- Cloud provider APIs (EC2 snapshots, resize). Possible later behind a plugin boundary.

---

## 2. Components

| Component | Runs where | Process | Purpose |
|---|---|---|---|
| `opsbox serve` | each server | user `opsbox`, unprivileged | HTTP API, WebSockets, auth, database, scheduler, deploy pipeline, proxy control, alerts. Serves `web/` when the web panel is on. |
| `opsbox priv` | each server | root, no network | Executes a closed set of typed privileged operations for `serve` over a Unix socket. |
| `opsbox proxy` | each server | user `opsbox` with `CAP_NET_BIND_SERVICE` | Reverse proxy on :80/:443 for app domains and the optional panel domain. ACME certificates. |
| `opsbox watchdog` | each server | root | Health-checks the others, restarts, rolls back a bad update, serves safe mode. |
| `opsbox` CLI | each server | invoked by an admin over SSH | Status, pairing, web panel toggle, update, rollback, uninstall, repair. |
| Desktop app | user's computer | Tauri | Not part of this spec beyond the protocol it speaks (section 8). |

All five server roles are subcommands of the **same binary**. Each runs as its own systemd unit, so a crash in one does not take down the others, and a single file is updated and rolled back as a unit.

### Why privilege separation

The agent needs root for most of what it does (packages, systemd, firewall, any file, Docker). Running the network-facing HTTP server as root would make any request-handling bug a root compromise. So:

- `serve` parses every request, authenticates, authorizes, validates, and audits. It never touches the system directly.
- `priv` accepts only typed requests (an enum, not shell strings), re-validates paths and arguments, and executes. It has no network access (systemd `RestrictAddressFamilies=AF_UNIX`, `PrivateNetwork` not possible because it manages the network, so it relies on not opening sockets).
- The socket `/run/opsbox/priv.sock` is owned `root:opsbox`, mode 0660. `priv` checks `SO_PEERCRED` and accepts only the `serve` and `proxy` users.

Docker access is root-equivalent. Only `priv` talks to `/var/run/docker.sock`; `serve` never joins the `docker` group.

---

## 3. Repository and crate layout

Cargo workspace at the repo root. `web/` stays where it is.

```
Cargo.toml                 workspace
crates/
  opsbox/                  the binary: clap CLI, subcommand dispatch, systemd integration
  opsbox-proto/            shared types: API request/response structs, privileged op enum, events. serde. Also used by the Tauri app.
  opsbox-store/            SQLite schema, migrations, repositories, secret encryption
  opsbox-auth/             users, passwords, TOTP, sessions, API tokens, device keys, roles, lockout
  opsbox-api/              axum router, middleware, WebSocket handlers, static UI embedding
  opsbox-priv/             privileged server and client; each operation in its own module
  opsbox-sys/              system facts and actions used by priv: packages, systemd, journal, timers, firewall, users, ssh, swap, disks
  opsbox-docker/           Docker Engine client wrapper, compose, image and volume management
  opsbox-deploy/           GitHub integration, build detection, build, release, health check, rollback
  opsbox-proxy/            reverse proxy, routing table, ACME, certificates
  opsbox-metrics/          sampling, ring buffers, downsampling, attention rules
  opsbox-backup/           restic orchestration, destinations, schedules, restore
  opsbox-alerts/           rule engine, channels, dedupe, quiet hours
  opsbox-net/              WireGuard interface, peers, network rules, pairing
  opsbox-update/           self-update, signature verification, version slots
  opsbox-watchdog/         health checks, restart and rollback policy, safe-mode server
```

Rule of thumb: crates below `opsbox-api` never depend on axum; crates used by `priv` never depend on the network stack.

### Main dependencies (choose current stable versions at implementation time)

- Runtime: `tokio`, `tracing`, `tracing-journald`.
- HTTP: `axum`, `tower`, `tower-http` (compression, limits, timeouts), `hyper`, `rustls`.
- Serialization: `serde`, `serde_json`, `toml`.
- Storage: `rusqlite` (bundled SQLite) with `refinery` or hand-rolled migrations; or `sqlx` with SQLite. Pick one; rusqlite keeps the binary smaller.
- Crypto: `argon2`, `totp-rs` (or a small RFC 6238 implementation), `chacha20poly1305`, `ed25519-dalek`, `rand`, `sha2`, `subtle`, `zeroize`.
- System: `sysinfo` plus direct `/proc` and `/sys` reads, `zbus` (systemd over D-Bus), `nix`, `portable-pty`.
- Docker: `bollard`.
- Proxy and TLS: `hyper` + `rustls` + `instant-acme` (or `rustls-acme`), `hickory-resolver` for DNS checks.
- Scheduling: own minimal cron parser or `croner`.
- Email: `lettre`. HTTP client for webhooks and GitHub: `reqwest` with rustls.
- WireGuard: kernel WireGuard via netlink (`defguard_wireguard_rs` or `wireguard-control`), with `boringtun` as userspace fallback when the kernel module is missing.
- Embedding the UI: `rust-embed` with compression; in debug builds it reads `web/` from disk.

Release profile as already in `Cargo.toml`: `opt-level = "z"`, `lto = true`, `strip = true`, `panic = "abort"`, `codegen-units = 1`.

---

## 4. Files, users and units on the server

```
/usr/local/bin/opsbox                     symlink to the active slot
/var/lib/opsbox/bin/opsbox-<version>       version slots (keep current and previous)
/etc/opsbox/config.toml                    settings that must survive a database loss
/etc/opsbox/master.key                     32-byte key encrypting secrets in the database, root:root 0600
/var/lib/opsbox/opsbox.db                  SQLite, opsbox:opsbox 0600, WAL mode
/var/lib/opsbox/apps/<service>/            build contexts, compose files, per-service state
/var/lib/opsbox/backup/                    restic cache, restic binary
/var/lib/opsbox/acme/                      ACME account key and certificates (encrypted at rest)
/var/log/opsbox/                           nothing by default; logs go to journald
/run/opsbox/priv.sock                      privileged socket
```

The master key is read by `priv` only, which decrypts secrets on request for `serve`. That way a read of the database file alone does not leak secrets.

System user `opsbox` (no shell, no home login).

systemd units, all `Restart=always`:

- `opsbox-priv.service`, root.
- `opsbox.service` (`serve`), `User=opsbox`, `After=opsbox-priv.service`, hardened: `NoNewPrivileges`, `ProtectSystem=strict` with `ReadWritePaths=/var/lib/opsbox`, `ProtectHome`, `PrivateTmp`, `MemoryMax=256M`.
- `opsbox-proxy.service`, `User=opsbox`, `AmbientCapabilities=CAP_NET_BIND_SERVICE`, same hardening.
- `opsbox-watchdog.service`, root, `MemoryMax=32M`, `Nice=-5`, starts first and never depends on the others.

---

## 5. Data model (SQLite)

All tables have `id` (ULID text), `created_at`, `updated_at`. Secrets are stored as `enc:v1:<nonce>:<ciphertext>` encrypted with the master key.

- `users` (name, role, password_hash, totp_secret enc, totp_enabled, locked_until, failed_count)
- `sessions` (user_id, token_hash, kind web|desktop, ip, user_agent, expires_at, last_seen)
- `api_tokens` (name, token_hash, scopes json, last_used_at, expires_at)
- `devices` (name, wg_public_key, ed25519_public_key, user_id, last_seen) for paired desktop apps
- `audit` (ts, actor, actor_kind, ip, action, target, result, detail json, prev_hash, hash) append-only, hash-chained
- `settings` (key, value json)
- `projects`, `environments`
- `services` (project_id, env_id, name, kind app|db|tool|compose, source json, build json, runtime json, port, health json, resources json)
- `service_vars` (service_id, key, value enc, is_secret)
- `deploys` (service_id, commit, message, trigger, status, image, started_at, finished_at, log_ref)
- `domains` (service_id or panel, name, primary, redirect_to, force_https, hsts, dns_state, cert_state, cert_expires_at)
- `certificates` (domain, chain, key enc, not_after, issuer)
- `jobs` (schedule, kind, spec json, last_run_at, last_result)
- `firewall_rules` (port, proto, source, note, managed)
- `ssh_keys` (linux_user, name, type, fingerprint, public_key, added_by)
- `backup_destinations` (provider, name, config json, secrets enc, verified_at)
- `backup_jobs` (what json, schedule, retention json, destination_id, last_run json)
- `backup_snapshots` (job_id, restic_id, taken_at, size, verified)
- `alert_channels` (kind, config json, secrets enc, verified_at)
- `alert_rules` (kind, condition json, level, channels json, enabled)
- `alert_events` (rule_id, state firing|resolved|silenced, started_at, ended_at, last_notified_at, detail)
- `net_peers` (kind device|server, name, wg_public_key, address, endpoint, last_handshake)
- `net_rules` (source, destination, ports, action)
- `metrics_minute` (ts, cpu, mem, disk json, net_in, net_out, load) kept 7 days; `metrics_hour` kept 90 days

Migrations are numbered and run at startup inside a transaction. The database is backed up into the opsbox backup set automatically.

---

## 6. API surface

### 6.1 Conventions

- Base path `/api/v1`. JSON in and out. Times in RFC 3339 UTC. IDs are ULIDs.
- Errors: HTTP status plus `{ "error": { "code": "validation", "message": "human sentence", "field": "domain" } }`. The UI already shows `message` verbatim, so messages are written for people, like the mock's.
- Long operations return `202` with `{ "task": "<id>" }`; progress and logs stream on `/api/v1/tasks/<id>/stream` (WebSocket) and the event bus.
- Pagination: `?limit=&cursor=`.
- Every mutating call writes an audit entry before returning.
- Rate limits per session and per IP (token bucket), stricter on auth endpoints.
- Request body limit 1 MB, except file upload (streamed, limit configurable, default 2 GB).

### 6.2 Authentication

Three kinds of caller:

1. **Web panel session** (browser): `POST /auth/login` with username, password, TOTP code. Returns an `HttpOnly; Secure; SameSite=Strict` cookie, 12 h idle / 7 d absolute. CSRF: double-submit token in header `X-Opsbox-CSRF`, plus `Origin` check.
2. **Desktop device**: requests arrive over WireGuard from a paired device. Each request carries a short-lived token obtained by signing a server challenge with the device's Ed25519 key. The WireGuard source address must match the device's peer address. Desktop users still sign in with password + TOTP once per device and on a configurable interval.
3. **API token**: `Authorization: Bearer ops_…`, scoped (`deploy:<service>`, `metrics:read`, `full`), hashed at rest, shown once.

Passwords: Argon2id, memory 64 MB, iterations 3, parallelism 1 (tune to keep login under 300 ms on a 1 vCPU instance). TOTP: RFC 6238, SHA-1, 6 digits, 30 s, accept ±1 step, reject reuse of the last accepted code. Lockout: 5 failures in 15 minutes per user and per IP, exponential backoff, audit plus alert.

Roles for v1: `admin` (everything), `deployer` (apps, logs, no terminal, no files outside app dirs, no system), `viewer` (read only). Each handler declares the permission it needs; middleware enforces it.

### 6.3 First run

- Install generates a single-use setup token, prints it to the console and the boot log (`/dev/console` and journald), valid 1 hour. That is how the UI's Setup screen works.
- `POST /setup/claim {token}` then `POST /setup/admin {username, password}` then `POST /setup/totp/confirm {code}`. Setup endpoints exist only until an admin exists.
- Until claimed, the panel listens on the public address, port 8443, with a self-signed certificate, and only `/setup/*` and static assets answer. After the admin exists, the listener stays on (the panel is the product) and Settings offers attaching a domain with Let's Encrypt in one step; an optional IP allow-list can restrict it.
- `--local-only` at install keeps every listener on localhost (current Phase 0/1 behaviour).
- (opsbox Desktop, later: pairing adds a second first-run path.)

### 6.4 Endpoint map

The UI calls these through `web/js/api.js`. Left column is the current mock function.

| Mock function | Method and path |
|---|---|
| `auth.login` | `POST /auth/login` |
| `auth.claim` | `POST /setup/claim` |
| `signOutAll` | `POST /auth/sessions/revoke-all` |
| `getHost` | `GET /host` |
| `getMetrics`, `tick` | `GET /metrics?window=48s` and WS `/stream/metrics` |
| `getAttention` | `GET /attention` |
| `listContainers`, `listDocker(kind)` | `GET /docker/containers`, `/docker/images`, `/docker/volumes`, `/docker/networks` |
| `containerAction` | `POST /docker/containers/{id}/{start|stop|restart|delete}` |
| `containerLogs` | `GET /docker/containers/{id}/logs?tail=` and WS `/stream/containers/{id}/logs` |
| `getProject` | `GET /projects/{id}` (services, graph, state) |
| `getRepos` | `GET /github/repos?q=` |
| `createService` | `POST /services` |
| `redeploy` | `POST /services/{id}/deploys` |
| `rollback` | `POST /services/{id}/deploys/{deployId}/rollback` |
| `buildLog` | `GET /deploys/{id}/log` and WS `/stream/deploys/{id}` |
| `addVar`, `removeVar` | `PUT /services/{id}/vars/{key}`, `DELETE …` |
| `removeService` | `DELETE /services/{id}` |
| `getDomains` | `GET /services/{id}/domains` |
| `addDomain` | `POST /services/{id}/domains` |
| `checkDns` | `POST /domains/{name}/check` |
| `removeDomain` | `DELETE /domains/{name}` |
| `makePrimary` | `POST /domains/{name}/primary` |
| `TEMPLATES`, `addTemplate` | `GET /templates`, `POST /templates/{id}/instances` |
| `deployCompose` | `POST /compose` |
| `getUpdates`, `installUpdates`, `setAutoUpdates` | `GET /system/updates`, `POST /system/updates/install`, `PUT /system/updates/auto` |
| `reboot` | `POST /system/reboot` |
| `getUnits`, `unitAction` | `GET /system/units`, `POST /system/units/{name}/{action}` |
| `getJournal` | `GET /system/journal?unit=&prio=&q=&cursor=` and WS `/stream/journal` |
| `getJobs`, `runJob`, `addJob` | `GET /jobs`, `POST /jobs/{id}/run`, `POST /jobs` |
| `getFirewall`, `addRule`, `removeRule` | `GET /firewall`, `POST /firewall/rules`, `DELETE /firewall/rules/{id}` |
| `openSsh` | `POST /ssh/emergency` |
| `scanCleanup`, `runCleanup`, `setAutoCleanup`, `getReclaimable` | `GET /cleanup/scan`, `POST /cleanup/run`, `PUT /cleanup/auto` |
| `listDir`, `readFile`, `writeFile`, `chmod`, `mkdir`, `removePaths`, `upload` | `GET /files?path=`, `GET /files/content?path=`, `PUT /files/content`, `POST /files/chmod`, `POST /files/mkdir`, `POST /files/delete`, `POST /files/upload` (streamed), `GET /files/download?path=` |
| `termExec` (mock only) | replaced by WS `/stream/terminal` (section 7.9) |
| `getSecurity`, `toggleCheck`, `setAllowIps`, `inviteUser`, `getAudit` | `GET /security`, `POST /security/checks/{id}/fix`, `PUT /security/allow-ips`, `POST /users/invites`, `GET /audit` |
| `getAccess`, `addKey`, `removeKey`, `lockUser`, `rotateOutKey` | `GET /access`, `POST /access/keys`, `DELETE /access/keys/{id}`, `POST /access/users/{name}/lock`, `POST /access/outgoing/{id}/rotate` |
| `getBackups`, `runBackup`, `restoreBackup` | `GET /backups`, `POST /backups/jobs/{id}/run`, `POST /backups/snapshots/{id}/restore` |
| `PROVIDERS`, `testDestination`, `saveDestination`, `addDestination` | `GET /backups/providers`, `POST /backups/destinations/test`, `POST /backups/destinations` |
| `getAlerts`, `setRule`, `testAlert`, `connectChannel`, `silenceAlert`, `setQuiet` | `GET /alerts`, `PUT /alerts/rules/{id}`, `POST /alerts/test`, `PUT /alerts/channels/{id}`, `POST /alerts/events/{id}/silence`, `PUT /alerts/quiet-hours` |
| `getSettings`, `saveSettings`, `setSwap`, `growFs`, `setOpsboxAutoUpdate`, `disconnectGithub`, `exportConfig` | `GET/PUT /settings`, `PUT /system/swap`, `POST /system/disks/grow`, `PUT /update/auto`, `DELETE /github`, `GET /export` |
| `createToken`, `revokeToken` | `POST /tokens`, `DELETE /tokens/{id}` |
| `getRecovery`, `recoveryAction` | safe-mode server only (section 7.16) |

Event stream: WS `/stream/events` pushes `{type, ts, data}` for deploy progress, container state, alerts, audit, tasks. The UI's 2-second polling `tick()` becomes a subscription.

`docs/API.md` is regenerated from `opsbox-proto` (OpenAPI via `utoipa`) once the backend exists, so the UI and backend cannot drift.

---

## 7. Subsystems

Each subsection: responsibility, behaviour, implementation notes, and what can go wrong.

### 7.1 Event bus and tasks

- In-process `tokio::sync::broadcast` for events, bounded; slow subscribers are dropped with a resync hint.
- `Task` = a long operation with id, status, progress, log lines (kept 24 h). Deploys, backups, updates, cleanup and restores are tasks.
- Only one task of a kind per target at a time (e.g. one deploy per service); a second request returns `409` with the running task id.

### 7.2 Privileged operations (`priv`)

- Request: `PrivOp` enum from `opsbox-proto`, length-prefixed JSON or bincode over the Unix socket, with a request id and the authenticated actor for audit.
- Examples: `PkgList`, `PkgInstall{which}`, `SystemdAction{unit,action}`, `JournalQuery{…}`, `FileRead{path,range}`, `FileWrite{path,tmp_ref,mode}`, `FileList{path}`, `Chmod`, `Mkdir`, `Remove{paths}`, `PtySpawn{user,cols,rows}`, `Docker{…}`, `FirewallApply{ruleset}`, `SshKeysSet{user,keys}`, `UserLock{name}`, `Swap{size}`, `GrowFs{device}`, `Reboot{delay}`, `WgApply{config}`, `SecretDecrypt{ref}`.
- `priv` rejects anything not in the enum. Paths are canonicalized; `/proc`, `/sys`, `/dev`, and `/etc/opsbox/master.key` are refused for file ops. Writes go to a temp file in the same directory, `fsync`, then `rename`, preserving owner and mode.
- Large payloads (file upload, download, PTY bytes) use passed file descriptors (`SCM_RIGHTS`) rather than copying through JSON.

### 7.3 Metrics and Overview

- Sample every 1 s: CPU per core and total, load, memory and cache, swap, disk usage per mount, disk IO, network per interface, connection count, top processes by CPU and memory, container stats (from Docker stats stream, every 2 s).
- Keep a 1-hour in-memory ring at 1 s; write 1-minute aggregates to SQLite (7 days) and hourly (90 days).
- `GET /attention` evaluates a fixed list: security updates pending, reboot required (`/var/run/reboot-required` or kernel mismatch), disk above 75%, failed units, restarting containers, expiring certificates, backups failing or only on the same disk, opsbox update available. Each item has text, level and a link target matching the UI.

### 7.4 Containers

- Through `priv` → Docker Engine API (`bollard`). List, inspect, start, stop, restart, remove, logs (follow), stats, exec (for templates and pg_dump).
- Labels mark what opsbox manages: `opsbox.service=<id>`, `opsbox.deploy=<id>`. Unmanaged containers are shown and controllable but never deleted by cleanup without explicit selection.
- If Docker is not installed, the Containers page offers to install it (official convenience script is not used; packages from the distro or Docker's apt/dnf repo with signed keys).

### 7.5 Deploy pipeline

1. **Source**: GitHub App (preferred: fine-grained, per-repo, webhook built in) or a deploy key for a single repo. Public repo URL and Docker image are also sources.
2. **Trigger**: manual, webhook on push to the chosen branch (HMAC-verified), or API token from CI.
3. **Fetch**: shallow clone at the commit into `/var/lib/opsbox/apps/<svc>/src/<commit>`.
4. **Detect**: Dockerfile present → Docker build. Otherwise Nixpacks (bundled binary, optional) or a static-site builder (detect `package.json` build script with an output dir, or plain `index.html`). The detected method is stored and editable.
5. **Build**: `docker build` via BuildKit with cache per service, build args from non-secret vars, secrets via BuildKit secret mounts (never baked into layers). Timeout and memory limit per build. Logs stream to the task.
6. **Release**: start the new container on the internal network with env vars, resource limits (CPU, memory), restart policy, labels. The old container keeps serving.
7. **Health check**: HTTP path (default `/`, configurable `/healthz`) must return 2xx/3xx within N tries; or TCP connect for non-HTTP.
8. **Switch**: update the proxy route atomically to the new container; drain the old one for 10 s; stop it but keep the image.
9. **Retain**: last 5 images per service for rollback. Rollback = steps 6 to 8 with an old image, no rebuild.
10. **Failure**: anything before step 8 fails → the old version stays live, deploy marked failed, alert raised.

Preview environments per pull request are a later phase; the data model already allows multiple environments.

### 7.6 Templates and Compose

- Templates are TOML definitions shipped in the binary: image, version list, ports, volumes, env with generated secrets, health check, default backup job. PostgreSQL, MySQL, MariaDB, Redis, MongoDB, MinIO, Uptime Kuma, Umami, n8n, Grafana for v1.
- Creating an instance: generate credentials, create a named volume, start the container on the project network, register a service of kind `db` or `tool`, add `DATABASE_URL`-style vars to the project, create a backup job if chosen.
- Compose: validate with `docker compose config`, refuse `privileged`, host network, and bind mounts outside `/var/lib/opsbox/apps/<svc>` unless the user confirms; run with `docker compose -p opsbox-<svc>`; each compose service becomes a node in the graph.

### 7.7 Reverse proxy, domains and certificates

- `opsbox proxy` listens on :80 and :443. Routing table pushed by `serve` over the Unix socket on change: host → upstream (container IP:port on the opsbox network), plus redirects (www, http→https), HSTS, WebSocket upgrade, request size and timeout limits, standard forwarding headers.
- ACME with Let's Encrypt: HTTP-01 on :80 by default, TLS-ALPN-01 as fallback; DNS-01 with a DNS provider token for wildcards (Cloudflare first). Renew 30 days before expiry, retry with backoff, alert after 3 failures. Staging environment in tests.
- DNS check: resolve A/AAAA (and CNAME) with `hickory-resolver` against public resolvers, compare with the server's public IPs (from metadata services when available, else a STUN-like echo or user setting). The UI's "waiting for DNS" uses this.
- Internal names (`<svc>.<server>.opsbox.internal`) resolve only on the private network and use an internal CA whose root the desktop app trusts.
- Decision to revisit: an in-process Rust proxy keeps one binary but is real work. Fallback if it slips: run Caddy as a managed container and drive it through its admin API. The `opsbox-proxy` crate boundary allows either.

### 7.8 System

- **Updates**: detect package manager (apt, dnf, zypper). List upgradable packages with security flag (apt: origin `-security`; dnf: `updateinfo`). Install selected. Auto security updates via `unattended-upgrades` / `dnf-automatic` configuration rather than our own timer. Reboot-required detection.
- **Reboot**: delayed by 1 minute with a cancel window; scheduled reboot at a time.
- **Services**: systemd over D-Bus (`zbus`): list units with state, enable, start, stop, restart. Refuse to stop `opsbox*`, `sshd` (when emergency SSH is the only way back), and the network service.
- **Journal**: `journalctl --output=json --follow` spawned by `priv`, filtered by unit, priority, text; cursor-based paging. (Native `sd-journal` binding later if needed.)
- **Jobs**: systemd timers named `opsbox-job-<id>.timer` with a matching service, created by `priv`. The UI's cron syntax is converted to `OnCalendar`; invalid expressions are rejected with a readable message.
- **Firewall**: if `ufw` or `firewalld` is active, manage rules through it; otherwise own an nftables table `inet opsbox`. Always keep a rule for the opsbox WireGuard port. Before applying, compute whether the change would cut the caller's access; refuse unless confirmed and auto-revert after 60 s without confirmation (like a display-settings timeout).
- **Emergency SSH**: open 22/tcp for the caller's IP only, enable `sshd` if disabled, schedule closing after 1 hour (timer), alert while open.
- **Swap**: create or resize a swap file with `fallocate`/`dd`, `mkswap`, `swapon`, fstab entry.
- **Grow filesystem**: detect partition and filesystem (`growpart`, `resize2fs`, `xfs_growfs`); only offered when the block device is larger than the partition.
- **Hostname and time zone**: `hostnamectl`, `timedatectl`.

### 7.9 Terminal

- WS `/stream/terminal?user=&cols=&rows=`. `serve` asks `priv` for a PTY (`PtySpawn`) running a login shell as the chosen Linux user (default the non-root admin user, root only for `admin` role with confirmation).
- Binary frames carry bytes both ways; text frames carry control messages (resize, ping). Backpressure: pause reading the PTY when the socket buffer is full.
- Idle timeout 30 min, hard cap configurable. One audit entry per session (start, end, user); optional full recording to an encrypted file for compliance mode.
- Terminal to other servers (host tabs) goes through the desktop app's private network to that server's own agent, never by storing SSH keys here, except the explicit "outgoing keys" feature.

### 7.10 Files

- List, stat, read (range), write (atomic), upload (streamed, resumable chunks), download (streamed), mkdir, chmod, chown, rename, delete (moves to `/var/lib/opsbox/trash` for 7 days unless "delete permanently").
- Size limit for the in-browser editor: 5 MB; larger files are download-only. Binary detection by content sniffing.
- Warnings the UI shows (world-readable `.env`, keys with loose modes) come from a cheap scan of the listed directory.
- `deployer` role is restricted to `/var/lib/opsbox/apps/<their services>`.

### 7.11 Cleanup

- Scanners, each returning an estimate and a description: unused Docker images, stopped containers, dangling volumes (opt-in, flagged as data loss), build cache, journal above a size, package cache, old opsbox release slots, old deploy sources, `/tmp` files older than 10 days, core dumps.
- Run = selected scanners in order, each reports bytes freed. Weekly auto-clean runs only the safe set.

### 7.12 Backups

- Engine: **restic**, downloaded at install for the right architecture with a pinned checksum, run by `priv`. One restic repository per destination, path `<prefix>/<server>`; password derived from the master key and also exportable as the recovery key.
- Destinations: S3-compatible providers from the UI list (R2, S3, B2, Wasabi, Spaces, Hetzner, GCS interop, MinIO, custom) via restic's S3 backend; SFTP via restic's SFTP backend with an opsbox-generated key. Provider presets in `opsbox-proto` hold endpoint templates and the help text the UI shows.
- Test: create repository if missing, write and read a small object, check delete permission, report each step (the UI's checklist).
- What can be backed up:
  - Databases from templates: logical dump through `docker exec` (`pg_dump -Fc`, `mysqldump --single-transaction`, `redis-cli --rdb`, `mongodump --archive`) streamed to restic via stdin. No stop needed.
  - Volumes: restic on the volume path; optional pause of the container for consistency.
  - Paths: any directory.
  - opsbox itself: database (SQLite online backup API) and `/etc/opsbox` except the master key.
- Schedule and retention (`restic forget --keep-daily --keep-weekly --prune`), run as tasks, with alerts on failure or a missed run.
- Restore: by default into a copy (new database container or `<path>.restored-<date>`); "replace" requires typed confirmation and takes a safety snapshot first. Monthly automatic restore test into a temporary container, result shown as "restore-tested".

### 7.13 Alerts

- Rule kinds: metric threshold with duration (CPU, memory, disk), state (unit failed, container stopped or restarting), events (deploy failed, backup failed or missed, cert expiring, security updates waiting N days, repeated failed sign-ins, emergency SSH open, opsbox unhealthy).
- Evaluation every 15 s over in-memory metrics; state machine per rule and target: ok → pending (duration) → firing → resolved. Notify on firing, on worsening, on resolve; repeat at most every 4 h while firing. Silence for a period.
- Channels: email (SMTP via `lettre`, or a provider API later), Slack incoming webhook, Telegram bot, generic webhook (JSON, HMAC-signed). Secrets encrypted.
- Quiet hours delay warnings, never critical alerts.

### 7.14 Security checks and audit

- `GET /security` runs cheap checks: TLS on panel, 2FA on all admins, lockout on, allow-list set, admin port bound to localhost or WireGuard only, SSH daemon state and password login, root login, secrets encrypted, watchdog running, update rollback enabled, world-readable secrets in app dirs. Each check has a fix action where safe.
- Audit log: every mutation and every sign-in, hash-chained (`hash = sha256(prev_hash || entry)`), exportable. `opsbox audit verify` checks the chain.

### 7.15 Self-update

- Channels: stable, beta. Check daily against GitHub Releases (`latest` API) unless disabled.
- Download the archive for the architecture, verify the minisign/Ed25519 signature against the public key compiled into the binary, verify SHA-256, unpack into a new slot, run `opsbox selftest`, switch the symlink, restart units in order (`priv`, `proxy`, `serve`).
- The watchdog watches the next 5 minutes; if health checks fail, it switches the symlink back and restarts, then reports.

### 7.16 Watchdog and safe mode

- Checks every 10 s: `serve` `/healthz` over the Unix socket, `priv` ping, `proxy` health, disk free on `/` and `/var/lib/opsbox`, database integrity on start.
- Policy: restart a failed unit; after 3 failures in 10 min, roll back to the previous slot if the failure began after an update; if disk is full, free opsbox's own space (old slots, logs, build cache) first.
- **Safe mode** server: tiny HTTP server inside the watchdog on `127.0.0.1:8444` and the WireGuard address, with its own minimal HTML (not the full UI). Auth with the same users table read-only (password + TOTP), or a one-time recovery code printed by `opsbox recovery-code` over SSH. Actions: show status and last logs, free disk space, restart opsbox, roll back version, open SSH for 1 hour, reboot. This is the `Recovery` screen.
- The watchdog shares no code paths with `serve` beyond `opsbox-proto` and `opsbox-store` read functions, to avoid one bug breaking both.

### 7.17 Private network and pairing (opsbox Desktop, later; not in the MVP)

- Interface `wg-opsbox`, UDP port 51820 by default, address space `100.96.0.0/16` (one /32 per peer, assigned by the device that adds the server).
- Kernel WireGuard via netlink when available; userspace `boringtun` otherwise.
- **Add server from the desktop app**: the app generates an install command containing a one-time pairing secret and the app's WireGuard public key and address. On the server, install sets up the interface with the app as a peer (no endpoint, since the laptop is behind NAT) and prints a pairing code: base32 of the server's WireGuard public key, public IP, and a short MAC over the pairing secret. The user pastes the code into the app, which adds the server as a peer with that endpoint and connects. Both sides then complete an authenticated handshake over the tunnel (device Ed25519 key registered, admin account bound).
- **Network rules**: stored centrally on each server as `net_rules`; applied as nftables rules on `wg-opsbox` (source peer, destination port, allow/deny, default deny). Server-to-server rules need both servers paired to the same desktop app, which distributes peer configs.
- The agent's API listens only on the WireGuard address and on localhost. The public interface sees only UDP 51820 (silent without a valid key), plus 80/443 if any domain is configured.

### 7.18 Web panel (the product, on by default)

- On by default. First run as in 6.3. `opsbox web domain panel.example.com` or Settings attaches a domain: the proxy routes it to `serve` with a Let's Encrypt certificate and the raw :8443 listener can then be closed. 2FA is mandatory for every account; an IP allow-list is offered.
- `opsbox web local-only on|off` switches between public and localhost-only listeners.
- The panel manages only its own server; the server switcher is hidden (the UI already knows from `GET /host` whether it runs as panel or desktop).

### 7.19 Static UI embedding

- `rust-embed` includes `web/` at compile time, gzip and brotli precompressed, with `ETag` and long cache for `fonts/` and `vendor/`, `no-cache` for `index.html` and `js/`.
- Debug builds serve from disk for live editing.
- `web/js/api.js` gets a `MODE` switch: `mock` (today), `http` (web panel, same origin with cookie), `tauri` (desktop, calls Rust commands that proxy to the agent over WireGuard).

---

## 8. Desktop app protocol (agent side only; opsbox Desktop, later)

- The agent treats the desktop app as an API client over WireGuard with device authentication (6.2).
- Server list, health and versions come from each agent's `GET /host` and `/stream/events`; the desktop app aggregates.
- No agent-to-cloud traffic. If both ends are behind NAT, v1 does not connect; a relay is a later decision.

---

## 9. CLI

`opsbox <command>`; root required except `status` and `version`.

| Command | Purpose |
|---|---|
| `install` | Used by `install.sh`: create user, dirs, units, keys, setup token or pairing. Idempotent. |
| `status` | Units, version, ports, WireGuard peers, disk, last backup, alerts. Exit code reflects health. |
| `pair` | Print a new pairing code (invalidates the previous one). |
| `web enable|disable [--domain]` | Toggle the web panel. |
| `setup-token` | Print a new setup token if no admin exists. |
| `recovery-code` | One-time code for safe mode. |
| `user reset-2fa <name>`, `user unlock <name>`, `user password <name>` | Recovery of panel accounts over SSH. |
| `update [--channel]`, `rollback` | Self-update and manual rollback. |
| `backup now`, `backup list`, `restore <snapshot> --to <path>` | Work without the UI. |
| `audit verify` | Check the hash chain. |
| `doctor` | Diagnose common problems (ports, DNS, Docker, disk, clock skew) and print fixes. |
| `uninstall [--keep-data]` | Remove opsbox; apps and data keep running. |
| `serve`, `priv`, `proxy`, `watchdog`, `selftest` | Internal, used by systemd units and the updater. |

---

## 10. Installer (`install.sh`)

1. POSIX sh, no bashisms; refuses to run without root (re-execs with sudo if available).
2. Detect OS (`/etc/os-release`), architecture, systemd, kernel WireGuard support.
3. Download `opsbox-<version>-<target>.tar.gz`, its `.sha256` and `.minisig` from GitHub Releases; verify with the minisign public key embedded in the script; abort on mismatch.
4. Place the binary in a version slot, symlink, run `opsbox install` with flags passed through (`--pair <secret>`, `--web`, `--channel`).
5. Print the next step: the pairing code, or the setup URL and token.
6. Works unattended from cloud-init user data; output also goes to `/dev/console` so it appears in the provider's system log.

---

## 11. Security model (summary)

Assets: root on the server, app secrets, backup data, the user's panel accounts.

| Threat | Mitigation |
|---|---|
| Internet scanning the panel | Before claim only `/setup/*` answers and needs the single-use token (expires in 1 h); after claim every route needs a session with 2FA; lockout and rate limits; security checks push a domain with HTTPS and offer an IP allow-list; `--local-only` for users who want nothing public. |
| Stolen session cookie | `HttpOnly`, `Secure`, `SameSite=Strict`, short idle timeout, bound to IP range optional, revoke-all. |
| Password guessing | Argon2id, TOTP required, lockout, alert. |
| Bug in request handling | `serve` is unprivileged; `priv` accepts only typed ops and re-validates. |
| Path traversal in files API | Canonicalization in `priv`, deny list, role-based roots. |
| Malicious Compose or template | Refuse privileged, host network and host mounts unless confirmed; run as non-root where images allow. |
| Secrets in the database file | Encrypted with a key only `priv` can read. |
| Compromised release | Signed releases, verification in installer and updater, reproducible build goal. |
| Bad update locking the user out | Watchdog rollback, safe mode, CLI over emergency SSH. |
| Firewall change cutting access | Lockout detection, confirm-or-revert after 60 s. |
| Audit tampering | Hash chain, export, verification command. |

A written threat model and `SECURITY.md` (private reporting address, response targets) ship before the first public release.

---

## 12. Observability of opsbox itself

- `tracing` with JSON to journald; levels per crate via config; request ids across `serve` and `priv`.
- `/healthz` (cheap) and `/readyz` (database, priv, proxy reachable).
- Internal metrics (request latency, task durations, queue sizes) exposed on `/api/v1/internal/metrics` for admins; no external telemetry.

---

## 13. Testing

- **Unit**: parsers (cron to `OnCalendar`, compose checks, domain validation, provider presets), auth (TOTP windows, lockout), path canonicalization (property tests and `cargo fuzz`), audit chain.
- **Contract**: OpenAPI generated from `opsbox-proto`; a test asserts every function in `web/js/api.js` has a matching operation.
- **Integration** in throwaway VMs (Multipass or Lima locally, GitHub Actions with nested virtualization or a cloud runner): install on Ubuntu 22.04/24.04, Debian 12, Amazon Linux 2023, Fedora; deploy a sample repo; domain with Let's Encrypt staging; backup to MinIO and restore; firewall lockout guard; update and forced rollback; kill each unit and verify recovery.
- **UI end-to-end**: the existing Playwright smoke tests run against a real agent in a VM with `MODE=http`.
- **Resource budget test**: idle RSS and CPU measured in CI; fail if above targets.

---

## 14. Build and release

- Targets: `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (static; `cargo-zigbuild` or `cross`). glibc builds only if a dependency forces it.
- GitHub Actions: lint (`clippy -D warnings`, `rustfmt`), test, build, size report, sign with minisign (key in a protected environment), upload to Releases with checksums, update the install script's version pointer.
- Third-party binaries fetched at install (restic, optionally Nixpacks) are pinned by version and checksum in the source.
- `cargo deny` for licenses and advisories; SBOM attached to each release.

---

## 15. Implementation phases

Each phase ends with something usable on a real server.

| Phase | Scope | Done when |
|---|---|---|
| 0. Skeleton | Workspace, `opsbox-proto`, store with migrations, `serve` + `priv` + socket, embedded UI, `MODE=http` in `api.js`, install script, systemd units, `status`. | Fresh VM: one command installs, UI loads from the agent, `/healthz` green. |
| 1. Own the server | Auth (password, TOTP, sessions, lockout, audit), first-run claim, metrics and Overview, system (updates, units, journal, jobs, firewall with lockout guard, emergency SSH, reboot), files, terminal, cleanup, security checks, access (SSH keys, Linux users), settings. | A server can be managed for a week without SSH. |
| 2. Recover | Watchdog, safe mode, self-update with signatures and rollback, CLI recovery commands. | Killing or corrupting `serve`, and a bad update, both recover without SSH. |
| 3. Containers and deploy | Docker integration, GitHub App, deploy pipeline, variables, rollback, templates, Compose, resource limits. | Push to main deploys; failed build leaves old version live. |
| 4. Proxy and domains | `opsbox proxy`, ACME, DNS checks, redirects, panel domain (priority: it is how small users secure the panel), public/local-only toggle. | Custom domain with HTTPS in under 5 minutes after DNS, for both apps and the panel. |
| 5. Backups and alerts | restic, providers, schedules, restore to copy, restore tests, alert rules and channels. | Database restored from R2 onto a fresh server. |
| 5b. Databases | See ROADMAP.md "Databases" part B. | A Postgres created, linked to an app and restored without SSH. |
| Desktop (separate product) | WireGuard, pairing, device auth, network rules, multi-instance, apps across instances on an existing orchestrator, Tauri app with `MODE=tauri`. Own spec. | Not part of the opsbox MVP. |
| Later | Preview environments, Nixpacks, DNS-01 wildcards, relay for double NAT, multi-user roles beyond three, compliance recording, cloud provider integrations. | |

---

## 16. Open questions

1. Proxy: in-process Rust proxy vs managed Caddy. Decide at the start of phase 4 with a one-week spike.
2. Nixpacks: bundle (larger download) or fetch on first use.
3. Default Linux user for the terminal on cloud images (`ubuntu`, `ec2-user`, `admin`): detect or ask during setup.
4. Whether `serve` may read Docker state directly (read-only socket proxy) for performance, or always go through `priv`.
5. Desktop app sign-in cadence: per device once, or re-auth every N days.
6. Licensing of bundled third-party binaries in releases (restic is BSD-2-Clause; check Nixpacks and others before bundling).
