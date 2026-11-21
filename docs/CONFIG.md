# Configuration

opsbox reads `/etc/opsbox/config.toml`. Every setting has a default, so the file is optional and `install.sh` writes it with everything commented out.

## Precedence

Highest wins:

1. command-line flag
2. environment variable
3. `config.toml`
4. built-in default

`--config FILE` (or `OPSBOX_CONFIG`) chooses another file. A file given this way must exist; the default path may be missing. Unknown keys and bad values are errors, so a typo never silently falls back to a default.

The allow-lists (`allowed_users`, `allowed_uids`) are the exception to "highest wins": flags and environment variables **add** to the file's lists instead of replacing them.

## Settings

| Section / key | Default | Flag | Environment | Used by |
|---|---|---|---|---|
| `[api] listen` | `0.0.0.0:8443` | `--listen ADDR` | `OPSBOX_LISTEN` | serve, status |
| `[priv] socket` | `/run/opsbox/priv.sock` | `--socket PATH` | `OPSBOX_PRIV_SOCKET` | serve, priv, status |
| `[priv] proxy_socket` | `/run/opsbox/priv-proxy.sock` | `proxy --socket PATH` | `OPSBOX_PRIV_SOCKET` (proxy only) | priv, proxy |
| `[priv] allowed_users` | `["opsbox", "root"]` | `--allow-user NAME` (repeatable, or comma list) | `OPSBOX_PRIV_ALLOW_USERS` | priv |
| `[priv] allowed_uids` | `[]` | `--allow-uid UID` (repeatable, or comma list) | `OPSBOX_PRIV_ALLOW_UIDS` | priv |
| `[priv] socket_mode` | `"0660"` | `--socket-mode MODE` | `OPSBOX_PRIV_SOCKET_MODE` | priv |
| `[priv] socket_group` | `"opsbox"` (`""` for none) | `--socket-group GROUP` | `OPSBOX_PRIV_SOCKET_GROUP` | priv |
| `[api] tls` | `true` (`false` only with a loopback `listen`) | | | serve |
| `[api] secure_cookies` | `"auto"` (`"always"`) | `--secure-cookies MODE` | `OPSBOX_SECURE_COOKIES` | serve |
| `[priv] master_key` | `/etc/opsbox/master.key` | `--master-key PATH` | `OPSBOX_PRIV_MASTER_KEY` | priv |
| `[auth] argon2_memory_kib` | `65536` | | | serve, CLI |
| `[auth] argon2_iterations` | `3` | | | serve, CLI |
| `[files] xfer_dir` | `/var/lib/opsbox/xfer` | `--xfer-dir PATH` | `OPSBOX_XFER_DIR` | serve, priv |
| `[files] trash_dir` | `/var/lib/opsbox/trash` | | | priv |
| `[files] apps_dir` | `/var/lib/opsbox/apps` | | | serve |
| `[files] max_upload_mb` | `2048` | | | serve |
| `[terminal] idle_timeout_min` | `30` | | | serve |
| `[backup] dir` | `/var/backups/opsbox` | | | serve, priv, CLI |
| `[backup] cache_dir` | `/var/cache/opsbox/restic` | | | priv |
| `[proxy] mode` | `"builtin"` | | | proxy, status |
| `[proxy] listen_http` | `0.0.0.0:80` | `--http ADDR` | `OPSBOX_PROXY_HTTP` | proxy |
| `[proxy] fallback_http` | `8080` | | | proxy |
| `[proxy] fallback_https` | `8445` | | | proxy |
| `[proxy] listen_https` | `0.0.0.0:443` | `--https ADDR` | `OPSBOX_PROXY_HTTPS` | proxy |
| `[proxy] socket` | `/run/opsbox-proxy/proxy.sock` | `--proxy-socket PATH` | `OPSBOX_PROXY_SOCKET` | proxy, serve, status |
| `[proxy] state_dir` | `/var/lib/opsbox/proxy` | `--state-dir PATH` | `OPSBOX_PROXY_STATE_DIR` | proxy |
| `[proxy] acme_staging` | `false` | `--staging` | `OPSBOX_ACME_STAGING` | proxy |
| `[proxy] acme_directory` | unset (Let's Encrypt production) | | | proxy |
| `[proxy] acme_email` | unset | | | proxy |
| `[proxy] public_ips` | `[]` (auto-detected) | | | proxy, serve |
| `[watchdog] listen` | `<api.listen ip>:8444` | `--listen ADDR` | `OPSBOX_WATCHDOG_LISTEN` | watchdog, status |
| `[watchdog] allow_ips` | `[]` (all addresses) | | | watchdog, status |
| `[watchdog] runtime_dir` | `/run/opsbox-watchdog` | `--runtime-dir PATH` | `OPSBOX_WATCHDOG_RUNTIME_DIR` | watchdog, recovery-code |
| `[store] db_path` | `/var/lib/opsbox/opsbox.db` | `--db PATH` | `OPSBOX_DB` | serve, watchdog |
| `[update] channel` | unset (`"beta"` while the running version is a pre-release, else `"stable"`) | `opsbox update --channel` | | serve, update |
| `[log] level` | `"info"` | `--log-level FILTER` | `OPSBOX_LOG_LEVEL` | all |
| `[log] format` | `"text"` (`"json"`) | `--log-format text\|json` | `OPSBOX_LOG_FORMAT` | all |
| (config file path) | `/etc/opsbox/config.toml` | `--config FILE` | `OPSBOX_CONFIG` | all |

`--log-level` takes a `tracing` filter: `info`, `debug`, or per-crate like `info,opsbox_priv=debug`. Logs go to stderr, which systemd sends to the journal (`journalctl -u opsbox -u opsbox-priv`).

### What the privileged-helper settings mean

The proxy runs as `opsbox-proxy` and connects to `[priv] proxy_socket`, fixed at root:opsbox-proxy 0660. That socket accepts only the proxy UID; the panel socket rejects that UID, even if an allow-list accidentally includes it (startup fails). Proxy and panel use separate groups. The proxy control socket remains accessible to panel/root via kernel UID checks. Operation restrictions for the proxy helper connection are tracked separately in Fix 27.

`priv` accepts a connection on the panel socket only if the connecting process runs as one of the allowed users. The kernel reports who is on the other end (`SO_PEERCRED` on Linux); the check happens before anything the peer sent is read. Names in `allowed_users` are looked up when `priv` starts; names that do not exist are skipped with a warning. If nobody is allowed, `priv` refuses to start.

The default includes `root` so an administrator can run `sudo opsbox status`. Remove it if you do not want that.

`socket_mode` and `socket_group` are the first gate (who may open the socket at all); the user check is the second.

### Sign-in settings

- `secure_cookies`: `auto` marks the cookies `Secure` whenever the request arrived over TLS: on `serve`'s own TLS listener, or through the opsbox proxy (which proves itself with a shared secret; see Listening address). `always` forces it on any listener. There is no `never`: a config that still says `secure_cookies = "never"` is refused at start with a clear error; delete the line. Over HTTPS the session cookie is named `__Host-opsbox_session` (browsers then accept it only with `Secure`, `Path=/` and no `Domain`); over plain loopback HTTP it is `opsbox_session`. After updating from a build that used the old name, everyone signs in once more.
- `master_key`: the 32-byte key that seals TOTP secrets (and later other secrets). `priv` creates it with mode 0600 the first time it is needed and refuses a key file that others can read. Back it up with the database: without it, sealed values cannot be opened, and every user would need `opsbox user reset-2fa`.
- `argon2_memory_kib` and `argon2_iterations`: password hashing cost (default 64 MB, 3 passes, one lane; the spec's values). A sign-in should take well under a third of a second. On a very small machine lower the memory (for example `16384`) rather than the passes. Existing hashes keep working after a change, because each hash records its own cost. Two sign-ins may hash at once, so peak use is about twice the memory setting.

### File settings

`xfer_dir` is where uploads and downloads are staged. `serve` creates it (mode 0700, owned by the `opsbox` user) and writes uploads there; `priv` moves them into place, or copies a file there for a download. `trash_dir` holds deleted files for seven days. `apps_dir` is the only place deployers can see. `max_upload_mb` caps one upload (the proxy in front of the panel, when there is one, needs a matching limit).

### Terminal settings

`idle_timeout_min`: a terminal in which nothing has been typed for this long is closed (1 to 1440 minutes). Output from a running program does not count as activity, so a forgotten tab cannot keep a root shell open. At most eight terminals are open at once.

### Example

```toml
[api]
listen = "127.0.0.1:9000"

[priv]
allowed_users = ["opsbox", "root", "deploy"]

[log]
level = "info,opsbox_api=debug"
format = "json"
```

## Listening address

`serve` never speaks plain HTTP on an address the network can reach.

- **Default:** `serve` terminates TLS itself (TLS 1.2 or newer, 1.3 preferred, `h2` and `http/1.1`) with its self-signed certificate. `opsbox status` prints `https://…`, the TLS version and the certificate's SHA-256 fingerprint, read from the live listener; compare it with what the browser shows before accepting the certificate.
- **Plain HTTP:** `[api] tls = false` is accepted only with a loopback `listen` (`127.0.0.1`, `::1`), for an SSH tunnel. With any other address the config is refused at load (`serve`, `status` and every other command) with an error naming the address. No flag, environment variable or build type overrides this.
- **Panel domain:** the opsbox proxy terminates TLS and forwards to `serve`. `serve` believes `X-Forwarded-For` and `X-Forwarded-Proto` only from a loopback peer that also sends the proxy's secret (`X-Opsbox-Proxy-Secret`, stored in the database as `opsbox.proxy.forward_secret` and added by the proxy to panel requests only). From anyone else those headers (and `Forwarded`, `X-Real-IP`) are deleted before a handler sees them, so rate limits, the allowed-address list and the audit log use the real peer.

Reach a server that listens on loopback through an SSH tunnel:

```sh
ssh -L 8443:127.0.0.1:8443 you@server
# then open https://127.0.0.1:8443 (or http:// when tls = false)
```

`scripts/check-transport.sh <url>` prints the scheme, TLS version, certificate fingerprint, security headers and (with `OPSBOX_USER`, `OPSBOX_PASSWORD`, `OPSBOX_CODE`) the cookie attributes of a login, and fails when something is wrong.

## Commands that read the configuration

| Command | Notes |
|---|---|
| `opsbox serve` | Opens the database, applies migrations, listens. |
| `opsbox priv` | Creates the socket (and `/run/opsbox` if missing). |
| `opsbox proxy` | Listens on `:80`/`:443`, handles ACME HTTP-01 challenges, and proxies panel routes received over `proxy.sock`. |
| `opsbox watchdog` | Probes `serve`, `priv`, `proxy`, disk space and SQLite integrity every 10s, handles restarts/auto-rollback, and serves HTTPS Safe Mode on `:8444`. |
| `opsbox status` | Probes `serve`, `priv`, and (when installed/active) `proxy` and `watchdog`. Exit status 0 only if all healthy, 1 otherwise. |
| `opsbox recovery-code` | Prints a single-use 15-minute Safe Mode code (`XXXX-XXXX-XXXX`) into `/run/opsbox-watchdog/recovery-code.json` (mode `0600`). |
| `opsbox web local-only on\|off` | Switches `[api] listen` between `127.0.0.1:8443` and `0.0.0.0:8443` and restarts `opsbox.service`. |
| `opsbox web domain NAME [--skip-dns-check]` | Configures the panel domain, checks DNS, requests an ACME certificate from `proxy`, and saves `opsbox.panel.domain`. |
| `opsbox setup-token` | Prints a one-time token while no admin exists. Run as root, it switches to the `opsbox` user before opening the database. |
| `opsbox user reset-2fa\|unlock\|password NAME` | Account recovery over SSH. `password` asks twice on a terminal and reads one line from standard input when piped. |
| `opsbox audit verify` | Walks the hash chain of the audit log; exit status 1 and the first bad entry if it was altered. |
| `opsbox doctor` | Read-only diagnostics through the helper: service/port health, DNS resolution, free disk space, clock synchronization, optional Docker and master-key permissions. Exit 1 for failed checks or unavailable helpers; warnings include suggested fixes. |
| `opsbox backup list\|now` | Lists configured backup schedules, repository/same-disk status, and snapshots (`list`), or runs enabled schedules / `--job <ID>` synchronously through `priv` (`now`, with optional `--prune` / `--check`). |
| `opsbox selftest` | Ignores the configuration; uses a temporary directory. Exit status 0 if every check passes. |

Commands that exist but are not built yet (`pair`, `restore`, ...) print `` `<name>` is not implemented yet `` and exit with status 2.

## Running on a development machine

On a machine where opsbox is not installed there is no `opsbox` user and `/var/lib/opsbox` is not writable, so point the defaults somewhere you own.

Linux (two terminals):

```sh
cargo build --release

# 1. the helper, as root, allowing your own user. The socket group is your primary
#    group so mode 0660 lets you connect.
sudo ./target/release/opsbox priv --allow-uid "$(id -u)" --socket-group "$(id -gn)"

# 2. the server, as you, with a database in the current directory
./target/release/opsbox serve --db ./opsbox-dev.db

# 3. check, then open http://127.0.0.1:8443/?mode=http
./target/release/opsbox status
```

macOS or any machine without root (no `sudo`; the socket path must stay under about 100 characters):

```sh
./target/release/opsbox priv  --socket /tmp/opsbox.sock --allow-uid "$(id -u)"
./target/release/opsbox serve --socket /tmp/opsbox.sock --db ./opsbox-dev.db
./target/release/opsbox status --socket /tmp/opsbox.sock
```

`*.db` files are git-ignored.


## HTTP hardening and tokens (Phase 1)

Rate limits are fixed in this release: per IP burst 120/refill 2 per second; login, setup, invitation and TOTP routes share a stricter burst 20/refill 1 per 3 seconds; authenticated sessions/API tokens have burst 120/refill 4 per second. They return JSON 429 with `Retry-After`. At most 4096 active keys are tracked per bucket table. Persisted sign-in lockout remains five failures in 15 minutes with exponential backoff.

Ordinary API requests: 1 MiB body, 30 seconds; editor: 12 MiB JSON body/5 MiB text content, 60 seconds; upload: `files.max_upload_mb`, two-hour handler deadline. Package installs, automatic-update setup, job runs, cleanup and swap run as background tasks so a dropped HTTP request does not abandon the operation. Task history is in memory (up to 24 hours, capped at 256 entries) and disappears when serve restarts. At most 64 WebSocket streams/eight terminals, 64 KiB incoming messages, 256 KiB queued writes. Streams check revocation, expiry, role and allowed address every five seconds; output alone does not refresh the 12-hour session idle timer.

API responses are `no-store`; responses include request IDs, MIME-sniffing/frame protection and a same-origin referrer policy. CSP limits framing, base URLs and embedded objects while allowing the existing self-hosted Monaco/xterm runtime. Command output captured in priv is capped at 8 MiB per pipe; command timeouts kill the process group. Accepted authenticated mutations must be recorded in the audit chain before dispatch.

In Settings, host name/time zone/swap, API tokens, self-update settings, the panel domain (DNS check, ACME certificate, and local-only `:8443` toggle), disk growth, GitHub, export configuration, and uninstall are real. Tokens use `read`, `deploy`, or `full` scopes and are hashed at rest; copy the secret at creation. They have no expiry setting in Phase 1; revoke them explicitly. See [API.md](API.md) for exact capabilities. Host name/time zone changes use hostnamectl/timedatectl through priv and persist in the operating system; they are not duplicated in config.toml.

### Proxy modes

Set the mode in `/etc/opsbox/config.toml` and restart `opsbox-proxy`:

```toml
[proxy]
mode = "external"
```

- `builtin` (default): opsbox binds its configured HTTP/HTTPS addresses (normally 80/443), manages certificates and routes domains. Busy-port fallback and 30-second retries are unchanged.
- `external`: opsbox binds no proxy TCP listeners, including fallback ports, and does not run ACME. Service domains remain stored; configure routing and TLS in your own Caddy/nginx. Use the service's active host port as the loopback upstream, rather than its container port.
- `off`: disables opsbox proxy listeners and ACME without deleting stored domains or certificates.

Only these three lowercase strings are accepted; an invalid value prevents startup. Listener flags and environment overrides do not enable listeners in `external` or `off`. The proxy service waits for shutdown in these modes because its systemd unit uses `Restart=always`. The panel's own HTTPS listener on 8443 is separate.

The installer accepts `--proxy=external`, `--proxy=off` and explicit `--proxy=builtin`. An omitted flag preserves the existing mode; a fresh installation defaults to `builtin`. Repeating a flag updates only `[proxy] mode` without duplicate sections. `opsbox status` prints the configured mode and, in builtin mode when busy, each known holder's name and PID.

Admins can switch modes in Apps → Domains & SSL or Settings → Network. The confirmed, session-only action uses the root helper to update only the mode and restart the proxy; a failed restart restores the previous configuration. Undo existing web moves or port takeovers before disabling the proxy. In external/off mode, the Domains page shows the deployed service's actual `127.0.0.1:<active_host_port>` upstream when available, and certificate requests are refused with instructions to use your own proxy. Domain policies are stored for later builtin use; configure redirects and HTTPS in your external proxy.

The watchdog reads the current mode from the helper each tick and skips disabled proxy checks/restart counters. Readiness also skips the proxy in external/off even when a panel domain remains stored. Installer preflight leaves public ports to your server in those modes. If editing the file manually, restart `opsbox-proxy`; running serve/watchdog read the helper's current mode. Live Linux installation and restart-count checks remain in `docs/QA.md`.

### Busy proxy ports (builtin mode)

An occupied or inaccessible configured proxy port uses `fallback_http` / `fallback_https` on the same bind IP. If the preferred port is busy, HTTP searches 8080–8099 and HTTPS 8445–8464. Zero is invalid for fallback settings. If the entire fallback range is unavailable, startup fails with the bind error; inspect `journalctl -u opsbox-proxy -n 50`.

The proxy remains running, reports `port_conflict` and retries each primary port every 30 seconds. It promotes without restarting or interrupting accepted requests. The installer, CLI status, update verifier and watchdog treat this as healthy with a warning. Overview and Settings > Network show the ports actually serving traffic and the current holder. App routing still uses its configured Host/domain, including on fallback ports; HTTPS redirects include the actual HTTPS port.

Settings > Network can stop a confirmed Docker holder and set its restart policy to `no`, or stop/disable a confirmed systemd service. Undo restores its original policy/enablement and starts it after yielding opsbox's primary listeners. Root-only receipts next to `config.toml` survive restarts; the audit records the old policy and outcome. Unknown/ambiguous holders have no takeover action, and opsbox/Docker/containerd units cannot be stopped here. Containers and volumes are never removed.

“Keep it and use other ports” saves the current HTTP/HTTPS addresses as `listen_http` / `listen_https`, applies them live and ends primary retries. Automatic Let's Encrypt HTTP-01 for domains needs 80/443. File configuration should be authoritative when using this action: CLI/environment listener overrides still take precedence at the next proxy restart.

### Safe Mode source addresses and certificate

`[watchdog] allow_ips = ["203.0.113.7", "::1"]` accepts only those exact source IPs,
before TLS or HTTP. IPv4-mapped IPv6 addresses match their IPv4 equivalent; CIDRs
and invalid addresses are rejected at config load. An empty list keeps remote
recovery open to all source addresses; authentication is still required. Restart
`opsbox-watchdog` after editing. Include `127.0.0.1` / `::1` if you want local
health probes or SSH tunnels; there is no implicit loopback exception. Local-only
installation still follows the panel's loopback bind unless watchdog.listen is set.

Safe Mode generates its own `/etc/opsbox/watchdog/selfsigned.crt` and
`selfsigned.key` (root-owned, `0600`); it never reads the panel key or falls back
to the panel's writable data directory. `opsbox status` and the installer's status
summary show the allowlist and watchdog certificate SHA-256 fingerprint. When the
list excludes local probes, `sudo opsbox status` uses the root-only watchdog
snapshot (at most 45 seconds old). Compare the watchdog fingerprint with your
browser before signing in; the panel certificate has a different fingerprint.
