# Implementation decisions

Choices made while building the backend where `BACKEND_SPEC.md` left room or where reality differed. `ARCHITECTURE.md` holds the product decisions; this file holds the engineering ones. Newest phase last.

## Phase 0 (skeleton)

### Storage

**SQLite through `rusqlite` (bundled), not `sqlx`.**
- opsbox has one writer process, short queries and a small schema. It needs neither an async driver nor compile-time-checked query macros.
- `sqlx` brings an async runtime integration, a macro layer and a much larger dependency tree. The spec asks for a small binary and says `rusqlite` keeps it smaller.
- Cost: calls are synchronous. `Store` holds one connection behind a mutex; async code that needs it uses `spawn_blocking` (as `/readyz` does).
- `rusqlite` 0.43 is current and was released days before this was written.

**Hand-rolled numbered migrations, not `refinery`.** A table `schema_migrations` plus SQL files in `crates/opsbox-store/migrations/`, each applied in an `IMMEDIATE` transaction and re-checked under the lock so two processes starting together cannot both apply one. It is about 60 lines, has no extra dependency, and refuses to open a database whose version is newer than the binary knows (an older binary must not run against a newer schema).

**`settings` is keyed by `key`, with no ULID `id`.** Spec section 5 says every table has an `id`. For a key/value table the key is the identity; a second identifier would only duplicate it. `created_at` and `updated_at` are kept.

**`audit` has `ts` instead of `created_at`/`updated_at`, plus a `seq` column.** The log is append-only, so there is no update time. `seq` (`AUTOINCREMENT`) fixes the order of the chain; the ULID `id` is still unique but is not the ordering.

**Audit hash chain.** `hash = sha256(prev_hash || canonical_json([id, ts, actor, actor_kind, ip, action, target, result, detail]))`, with the genesis `prev_hash` as 64 zeros. The fields go into a JSON array (strings are escaped, so field boundaries cannot be forged) and `detail` is hashed as the exact text stored, so verification never depends on JSON key order. Database triggers also refuse `UPDATE` and `DELETE` on `audit`; the chain is what still shows tampering by someone who drops the triggers. Known limit: removing entries from the *end* is not detectable without an external anchor (exporting the head hash is the planned answer).

**`serve` writes one `system.start` audit entry at startup.** Phase 0 has no mutating endpoint, so without it the audit table would never be written by real code. It records the version and schema version.

**The database file is created `0600` before SQLite opens it**, and the `-wal`/`-shm` files inherit that. WAL, `synchronous=NORMAL`, `foreign_keys=ON` and a 5 s busy timeout are set at open.

### Privileged helper

**JSON frames, not bincode.** Spec 7.2 allows either. JSON is debuggable with `socat`, and the volume is tiny. Frame = 4-byte big-endian length + JSON, capped at 1 MiB; the length is checked *before* a buffer is allocated.

**Wire form.** `{"id": "<request id>", "actor": null, "op": {"type": "ping"}}`; operations with arguments will use `"args"`. Requests with unknown fields or unknown operations are rejected (`bad_request`, request id echoed). `PrivOp` is a closed enum and `ops::dispatch` matches on it exhaustively, so adding an operation without handling it does not compile.

**Peer credentials via tokio's `UnixStream::peer_cred()`.** On Linux this is `SO_PEERCRED`; on macOS it is the equivalent, so tests and development behave the same. It needs no `unsafe`. The check runs before any byte from the peer is read. A rejected peer gets a `forbidden` frame, then the server drains what the peer already sent: closing a socket with unread data resets the connection and would hide the reply.

**The default allow-list is `["opsbox", "root"]`, explicit rather than implicit.** The spec says only `serve` and `proxy` users; but `sudo opsbox status` needs to ping `priv`. Root can already do anything, so listing it grants nothing, and it is visible and removable in the config. An empty effective list makes `priv` refuse to start.

**One connection per call in the client.** A restarted `priv` is picked up with no reconnect logic. The server supports several frames per connection, so a pooled client can come later (the terminal will need long-lived streams anyway).

**Socket setup.** Stale socket files are replaced; a socket that still answers, or any non-socket file, is never touched. Mode and group are applied after `bind`, so for a moment the socket has the process umask's mode; the peer-credential check is the real gate. A missing `opsbox` group is a warning, not an error (development machines).

**`request id` is a string** and `serve` passes its HTTP request id through, so one request can be followed in the logs of both processes.

**`HostInfo` collection uses `sysinfo` (0.40) for hostname, kernel, uptime, CPUs and memory, and reads `PRETTY_NAME` from `/etc/os-release` itself** (falling back to `NAME VERSION`, then to sysinfo). `sysinfo` 0.40 changed its API (fallible calls); the version in the original `Cargo.toml` was 0.30.

### HTTP API

**axum 0.8, tower-http 0.7.** Both were stale in the original `Cargo.toml` (0.7 and 0.5).

**Own request-id middleware instead of `tower-http`'s `request-id` + `trace`.** `tower-http`'s version pulls in `uuid`; this is 30 lines, generates a ULID (matching the spec's id format), puts it in the tracing span so every log line carries it, returns it as `X-Request-Id`, and ignores any inbound `X-Request-Id` because it is untrusted input. Only the path is logged, never the query string (later phases may carry tokens there). Health probes and static assets log at `debug`; `/api/*` and failures at `info`.

**No authentication in Phase 0, so the default is loopback-only and `serve` warns at startup on any other address.** `GET /api/v1/host` exposes only hostname, OS, kernel, uptime, CPU count and RAM. Auth is Phase 1.

**`GET /api/v1/host` returns the mock's shape** (`name`, `os`, `kernel`, `cores`, `memTotal` in GB, `upDays`, `version`) plus `uptimeSecs`, so `overview.js` is unchanged. In `http` mode `getHost()` returns only real fields; it does not blend in mock values.

**Errors** follow spec 6.1. Added codes beyond the examples: `method_not_allowed` (405), `rate_limited`, `unavailable` (503), `internal`. When `priv` is down the client gets a sentence ("The privileged helper is not running. Start opsbox-priv and try again.") and the technical detail goes to the log only.

**`/healthz` has no dependencies; `/readyz` checks the database and `priv`** (the proxy joins in Phase 4). `readyz` is 503 until both answer.

**Embedded UI.** `rust-embed` with `mime-guess`; `serve.py` and `.DS_Store` are excluded; debug builds read `web/` from disk. Differences from spec 7.19:
- Files are embedded uncompressed and text assets are gzipped at request time (`CompressionLayer`, fonts skipped). Precompressed gzip/brotli would need a build step; not worth it before the UI is final. Brotli is deferred.
- `fonts/` and `vendor/` get `public, max-age=31536000, immutable`; everything else `no-cache` with an `ETag` (304 on match). The catch: the files keep their names, so upgrading a vendored file in place leaves browsers on the old copy for up to a year. Rename it or version its directory when upgrading.
- Paths with empty, `.` or `..` segments, backslashes or NUL are refused; percent-escapes are not decoded.

**Limits.** 1 MiB request bodies and a 30 s request timeout. The streamed upload endpoint will get its own limit.

### CLI and configuration

**Every command in spec section 9 exists in `clap`;** those not built print "not implemented yet" and exit 2 (distinct from 1 for failure). `audit verify` is among them even though the store can already verify the chain: Phase 0 scope says it is not built.

**Precedence is flag > environment > file > default**, with unknown keys rejected (`deny_unknown_fields`). Allow-lists are additive. The default config path may be absent; an explicit `--config` must exist. See `CONFIG.md`.

**`opsbox install` is a stub that prints its plan and changes nothing.** So in Phase 0 `install.sh` does the provisioning itself (user, directories, config, units). That logic moves into the binary in Phase 1.

**`opsbox status` uses a small HTTP probe over a raw TCP socket** instead of an HTTP client crate, for one `GET`. It reads at most 64 KiB. It exits 0 only if `serve` is ready and `priv` answers.

**`opsbox selftest` is hermetic**: in-memory database with an audit chain, frame round trip, embedded UI check, and a real `priv` on a temporary socket called as the current user. `install.sh` runs it before touching the system, and the future updater will too.

**Logging** is `tracing` to stderr (text or JSON); systemd sends stderr to the journal. `tracing-journald` from the spec's list is not used: it adds nothing the journal does not already do with stderr.

**Runtime.** `serve` uses two Tokio worker threads; `priv` and `status` use a current-thread runtime. Measured idle RSS of `serve` and `priv` on macOS arm64 was about 9 MB and 7 MB.

### Workspace and tooling

- Edition 2024, resolver 3, stable toolchain pinned in `rust-toolchain.toml` (no MSRV is promised yet).
- `unsafe_code = "forbid"` for the whole workspace. Nothing needed it.
- All 16 crates from spec section 3 exist; 11 are empty stubs with a one-line doc comment saying what they will hold.
- **Dropped from the original `Cargo.toml`** until a phase needs them: `portable-pty`, `argon2`, `rand`, `hex`, `base64`, `futures-util`, `libc`, `subtle`, `axum-server`, `rcgen`, and axum's `ws`/`multipart` features. `hex` was also replaced by a three-line function: its last release was 2021.
- **Crate health check (crates.io, November 2026):** every crate used had a release in the last twelve months except `mime_guess` (2024-06, a transitive dependency of `rust-embed`'s `mime-guess` feature; stable and tiny). `rust-embed` declares itself "passively maintained" but released on 2026-11-11.
- CI actions are pinned to commit SHAs.

### Packaging

**`/var/lib/opsbox` is `root:opsbox` mode `1770` (sticky), not owned by `opsbox`.** See the first item under "Problems found in the spec".

**`install.sh` fails closed on signatures.** With no public key embedded (true until the first signed release), a normal install of a download aborts. `--dev` skips only the signature, never the SHA-256 check, and is required with `--binary`. `OPSBOX_RELEASE_BASE` overrides the download location for mirrors and tests. The WireGuard kernel-support probe from spec section 10 is not included: nothing uses WireGuard before Phase 6.

**systemd.** `StartLimitIntervalSec=0` on both units: with `Restart=always` and a 2 s delay, the default start limit can stop a crash-looping service for good, which defeats `Restart=always`. `opsbox.service` uses `Wants=` on the helper, not `Requires=`, so `serve` stays up and reports "not ready" if `priv` restarts. `/run/opsbox` is created by the helper's unit (`RuntimeDirectory`, preserved across restarts). The units were checked for syntax only: this was developed on macOS and no systemd host was available, so the first real Linux install is still to be done.

### Web UI

`MODE` is `'mock'` unless the page is opened with `?mode=http`, so the real agent can be tried without editing or rebuilding. Only `getHost()` is real in `http` mode; every other function keeps using the mock. No page module was changed.

## Phase 1

### M1: honest UI

`api.js` has one `REAL` set; a function is real in http mode only if its name is in it. Every mock function records when it is called, and `PAGE_USES` (generated from what each page references) adds data a page only fetches on a click or shows from static sample constants. `main.js` badges a page with "sample data" (or "partly sample data") from the union, so wiring a function removes the badge on every page that no longer depends on a mock. The badge is per page, not per section: pages are not split into sections that know their data source, and the badge's tooltip lists exactly which functions are still sample.

### M2: auth and first run

**TOTP lives in `priv`, not in `serve`.** Spec 5 and 4 say `priv` decrypts secrets on request for `serve` (`SecretDecrypt`). Doing that would let a compromised `serve` read every TOTP secret. Instead `priv` has two narrow operations: `TotpNewSecret` (returns the sealed value to store, plus the plaintext once, for the QR code) and `TotpVerify` (sealed secret + code in, matching time step out). `serve` never holds a secret except for that single enrolment response. Generic `SecretSeal/SecretOpen` will be added when something needs them (API tokens, backup credentials). Secret-bearing protocol fields use a `Secret` type whose `Debug` prints `[redacted]`.

**Master key.** `/etc/opsbox/master.key`, 32 random bytes, created by `priv` with `create_new` and mode 0600 on first use, read only by `priv`; a key file readable by group or others, or of the wrong size, is refused (and never overwritten). Sealing is XChaCha20-Poly1305 with a random 192-bit nonce and the owner (`totp:<user id>`) as authenticated data, so a sealed value cannot be moved to another row. Format `enc:v1:<nonce>:<ciphertext>` as the spec says.

**Crates.** `argon2` 0.6, `chacha20poly1305` 0.11, `hmac` 0.13 and `sha1` 0.11 (RFC 6238 implemented directly and checked against the RFC's test vectors instead of `totp-rs`: it is 40 lines, and `priv` stays free of extra dependencies), `getrandom` 0.4, `subtle`, `zeroize`, `rpassword` 7. QR codes: `qrcode-generator` 2.0.4 (MIT), vendored in `web/vendor/qrcode` with its licence, loaded only by the setup and enrolment pages. The QR output was checked by decoding it with an independent reader (jsQR).

**Passwords.** Argon2id at the spec's cost (64 MB, 3 passes, 1 lane), configurable in `[auth]`; at most two hashes run at once so concurrent sign-ins cannot exhaust memory (`MemoryMax=256M`). Policy (the spec names none): at least 12 characters, at most 256 bytes, not one repeated character, not containing the username. Unknown user names are verified against a dummy hash so response time does not reveal which accounts exist.

**TOTP.** SHA-1, 6 digits, 30 s, accepted from one step before to one step after. The newest accepted step is stored per user and only a newer step is accepted, enforced by an atomic `UPDATE ... WHERE totp_last_step < ?`, so two simultaneous sign-ins with one code cannot both win (tested).

**Lockout** (spec 6.2): failures are rows in `login_failures`. Five in 15 minutes for a user name, or for an address, locks it for 30 s, doubling with each further failure up to 15 minutes. Attempts made while locked are not counted, so an attacker cannot extend a lock indefinitely; a nonexistent user name is counted too. This does allow someone to lock a known admin name out; `opsbox user unlock` is the answer. The failed-attempt audit entry names the account if it exists; for unknown names it records the typed name only if it looks like a valid user name, so a password typed into the username box never reaches the log.

**Sessions.** 256-bit random tokens; only their SHA-256 is stored. 12 h idle, 7 d absolute (the spec's), `last_seen` refreshed at most once a minute, expired rows purged hourly. Kinds: `web`, and `enroll` for an account whose second factor was reset (15 minutes; it opens only the two enrolment endpoints). Times in the auth tables are unix seconds, because expiry and lockout are SQL comparisons and RFC 3339 strings with a variable number of fractional digits do not sort.

**CSRF and Origin.** The CSRF token is derived from the session token (`sha256("opsbox-csrf-v1:" + token)`), not an independent random value, so it cannot be planted by an attacker who does not already hold the session. The page reads it from the `opsbox_csrf` cookie and sends it in `X-Opsbox-CSRF`. Every state-changing request that uses the cookie must carry the token and an `Origin` header naming this host; a missing Origin is refused. Pre-login requests (sign-in, setup) have no session to protect, so their Origin is checked only when present (a browser always sends it on cross-site POSTs).

**Roles by extractor, not middleware.** A handler takes `Viewer`, `Deployer` or `Admin` as a parameter; the extractor authenticates, enforces the role and applies CSRF/Origin. This is the same guarantee as "each handler declares the permission it needs" with no way to forget the check on a handler that takes one, and a handler with none is visibly public.

**Cookies.** Session: `HttpOnly; SameSite=Strict; Path=/`. `Secure` is on when `secure_cookies` says so or, in `auto`, when `X-Forwarded-Proto: https` is present. The client address comes from the TCP peer; behind a local proxy (peer on loopback) from the rightmost `X-Forwarded-For` entry.

**First-run setup** (spec 6.3): `serve` creates a token at start-up if none is live and no admin exists, and prints it to the journal and `/dev/console`; `opsbox setup-token` and `install.sh` print a fresh one. Tokens are 12 characters from a 32-symbol alphabet (60 bits), stored hashed, single use, valid one hour; claiming it yields a 30-minute setup session in a cookie scoped to `/api/v1/setup`. Setup finishes when the first admin confirms a TOTP code. After that the setup endpoints return 404 for everybody, before any cookie, Origin or body check.

**Flaw found while testing the first version:** setup was defined as "no admin has finished two-factor enrolment". `opsbox user reset-2fa` leaves the only admin unenrolled, which re-opened setup, and creating an admin deletes unfinished ones, so a setup token could have replaced the admin. Setup now completes permanently (`setup.complete`), with a regression test.

**Recovery.** `reset-2fa` clears the secret and ends the user's sessions; the next sign-in with the password alone yields an `enroll` session and the UI shows the enrolment page. `opsbox user ...`, `setup-token` and `audit verify` open the database directly, and when run as root first switch to the `opsbox` user: running as root would create root-owned `-wal`/`-shm` files that `serve` could then not open.

**`GET /api/v1/session` is public** and says whether setup is required, so the page knows what to show first.

**Tests depend on the real clock** for TOTP (`priv` uses it, and a step is 30 s wide). The tests use steps 0 and +1 relative to now; a test that straddles a step boundary could fail, a chance of well under one in a thousand per run.

### M3: metrics

**Sampling goes through `priv`.** Reading `/proc` needs no privilege, but the rule is that `serve` never touches the system, and a once-a-second Unix-socket round trip of a few kilobytes costs nothing. `priv` stays stateless: `MetricsSample` returns cumulative counters (CPU ticks per core, memory, per-mount usage, disk and network byte counters, established TCP connections, optionally every process) and `serve` (`opsbox-metrics`) turns consecutive samples into rates. Counter resets (an interface re-created, a reboot) count as zero for that second instead of a huge spike; a new interface or process has no baseline and counts as zero until its second sample.

**Process scans every other second.** Walking `/proc/<pid>/stat` is the costly part, so it happens on every second sample; the list in between is carried over. CPU per process is percent of one core (as `top`), the list is the six busiest plus the three largest by memory that are not already in it (at most eight), and anything named `opsbox*` is marked as this pane. The reply is capped at 2000 processes so it always fits one frame.

**Disks.** Real filesystems only (ext*, xfs, btrfs, zfs, vfat, ...), each device once, via `statvfs`; the Overview card shows `/`. Disk I/O sums whole block devices (not partitions, loop or ram devices).

**Storage.** One-second points live in an in-memory ring (one hour, a few hundred KB). Per-minute averages (with the minute's peak CPU) are written to `metrics_minute` when the next minute starts, hourly averages to `metrics_hour`; 7 and 90 days of retention, purged hourly. A restart loses at most the minute in progress. Rows are written from a blocking thread so a slow disk never delays sampling.

**API.** `GET /api/v1/metrics?window=48s` returns the mock's exact shape (history arrays as percent, memory and disk in GB, network in MB/s) plus extras (`cores`, swap, `disks`, disk I/O, `ts`). The history is averaged into buckets of at least two seconds and at most 120 points, aligned to the newest sample, so the default window gives the mock's 24 points. A request that arrives before the first sample (about two seconds after start) waits for it; if sampling is failing it answers 503 with a sentence.

**WebSocket.** `WS /api/v1/stream/metrics` sends the newest frame at once and then one frame per second, each the same object `GET /metrics` returns, so the page is stateless. It authenticates before looking at the upgrade, and requires an `Origin` that names this host (browsers always send one on a WebSocket), which closes cross-site WebSocket hijacking. A slow client skips frames instead of buffering. M8 adds live session/token, role and allowed-address revalidation every five seconds; revocation ends open streams.

**Replacing the mock tick.** In http mode the page opens one socket after sign-in (not one per page); every frame refreshes the status bar and the pages that subscribed. If the socket drops, it reconnects with a growing delay and, if the reason is an ended session, sends the person to sign-in instead of retrying. Mock mode keeps its two-second tick.

**Attention** (spec 7.3): security updates, reboot required, disks over 75% (not at 75%), failed units, each with a sentence and a link in the UI's terms. Facts come from a new `SystemFacts` operation, refreshed every five minutes because it can run `apt-check`: reboot flag from `/run/reboot-required`, failed units from `systemctl list-units --state=failed`, updates from Ubuntu/Debian's `apt-check`. Children run with a cleared environment, absolute paths, no shell and a timeout. On other distributions update counts are unknown and simply not reported rather than reported as zero; M4's package module replaces this. The disk item says how full the disk is; the mock's "N GB can be reclaimed" returns with the cleanup scanners (M7).

**Development off Linux.** Off Linux the sampler uses `sysinfo` (CPU ticks synthesized from instantaneous usage, memory, load, processes, `/` usage, no network or disk I/O) so the whole UI can be developed on a laptop. The `/proc` parsers are pure functions tested on fixture text everywhere, and a Linux-only test reads the real files; CI is its first run.

### M4: system

**Initial M4 implementation (superseded by the M8 review): `systemctl` and `journalctl` through `priv`.** The brief and spec section 3 name `zbus`. I used the command-line tools with `--output=json` (and a text fallback for older systemd) because the parsing is testable here with fixture output, whereas D-Bus code could not be run on the development machine; it also adds no async runtime or dependency tree to `priv`. Nothing prevents moving the list and action calls to D-Bus later (the `UnitList`/`UnitAction` operations are the boundary); property subscriptions are the real reason one would.

**Every operation is typed, validated twice, and never a shell string.** `serve` validates for good error messages; `priv` validates again with the same rules before it starts any tool: package names must match `[A-Za-z0-9][A-Za-z0-9+.:_-]*`, unit names must end in `.service`, job ids are 8 hex digits, cursors and time zone names are character-checked, ports, protocols and sources are parsed. Children run with a cleared environment, absolute paths, fixed argument lists and a time limit. A caller cannot name packages at all: `PkgInstall` carries only "security" or "all" and `priv` works out what is pending.

**Updates.** apt (via `apt list --upgradable`, security from the `-security` suite) and dnf (`check-update` plus `updateinfo list --security`). Installing refreshes the index first and uses `install --only-upgrade <pending security packages>` (apt) or `upgrade --security` (dnf). It runs as a background task (`202 {task}`, `GET /tasks/{id}`, one per kind at a time, kept 24 h) because it takes minutes; the UI polls it. Automatic updates: apt writes `/etc/apt/apt.conf.d/20auto-upgrades` (installing `unattended-upgrades` if needed); dnf installs `dnf-automatic`, sets `upgrade_type = security`, `apply_updates = yes` and enables the timer. zypper is not supported.

**Reboot** is `shutdown -r +N` (1 to 60 minutes; the page uses 1). That gives the cancel window for free (`shutdown -c`), survives a restart of opsbox, and the pending time is read back from `/run/systemd/shutdown/scheduled`.

**Services.** One call lists loaded services, their unit-file state and memory (`MemoryCurrent`). Shown: failed, running, and inactive ones that are enabled; opsbox's own job units are hidden (they live under Jobs). **Refusal list:** `opsbox*` can only be started or enabled; `ssh`/`sshd` cannot be stopped or disabled (restart is allowed); `systemd-networkd`, `NetworkManager`, `networking`, `network`, `systemd-resolved`, `dbus`, `systemd-logind` cannot be stopped, disabled or restarted. Each refusal is a sentence that says why.

**Journal.** Newest-first query with unit and priority; the text filter runs in `priv` over up to 3000 entries instead of being handed to `--grep`, so no pattern reaches the tool. The live tail (`WS /api/v1/stream/journal`) polls with a cursor every two seconds rather than streaming a `journalctl -f` child through the socket: `priv` keeps its simple request/response protocol, and a stalled client holds no process. Like the metrics stream it needs an `Origin` that names this host.

**Jobs.** `opsbox-job-<id>.service` and `.timer` in `/etc/systemd/system`; the original cron text and command are kept in `X-Opsbox-*` keys so listing never has to parse `OnCalendar`. The command runs as root through `/bin/sh -c` with systemd quoting (`$` and `%` doubled). One line only, so nothing a person types can add a directive to a unit file. Cron to `OnCalendar` supports numbers, lists, ranges, steps, month and weekday names and `@daily`-style aliases; **setting both day-of-month and day-of-week is refused**, because cron treats that as "either" and systemd as "both". Errors say which field is wrong. The next run time is not shown (it needs a time-zone-aware parse of systemd's text output).

**Firewall.** Backends in order: `ufw` if active, `firewalld` if running (ports from anywhere only), otherwise our own `inet opsbox` nftables table. The nftables ruleset is regenerated whole from `/etc/opsbox/firewall.json` and loaded atomically after a `nft -c` check; it blocks by default but always keeps loopback, established traffic, ICMP, SSH (ports from `sshd_config`) and the private-network UDP port 51820. Turning it on needs confirmation. It is restored when `priv` starts (it does not survive a reboot otherwise).

**The lockout guard** (spec 7.8) is a pure, tested function: removing a rule is risky if it is on an SSH port and no other rule still lets the caller in; an unknown caller makes every such removal risky. A risky change needs `confirm`, is applied provisionally and is undone after 60 seconds unless confirmed; the janitor thread (every 10 s) performs the undo, so it works even if the browser tab is closed. **The caller's address:** through an SSH tunnel or a local proxy every request comes from loopback, which says nothing about the administrator's SSH source, so loopback counts as unknown. Emergency SSH therefore asks the person for the address they connect from when it cannot tell. Entries expire after an hour (state is persisted, so a restart of `priv` does not leave SSH open). `firewalld` emergency access is a runtime rich rule.

**Roles.** Reading the System pages needs any account; every change needs an admin. A strict reading of "deployer: no system" would also hide the reads; with a linear role order a deployer can see them but not change anything.

**`opsbox-priv.service` allows `AF_UNIX AF_NETLINK AF_INET AF_INET6`.** The first version restricted it to Unix sockets (spec section 2). Children inherit that restriction, and apt/dnf need the network and nft/ufw need netlink, so it could not work. `priv` itself still opens no listener.

**Swap, host name and time zone** are implemented in `priv` and exposed as `PUT /api/v1/system/swap|hostname|timezone` (swap as a task; size 128 MB or more and at most half of the free disk, 0 turns it off and edits `/etc/fstab` exactly). The Settings page is wired in M7 with real settings data, because it is prefilled from mock values and saving it now would apply those to a real server.

**Tests never change the machine.** System endpoint tests cover roles, CSRF, validation, the refusal list and error mapping, all decided before any tool starts, so they are safe on CI and as root.

### M5: files

**Everything is a `priv` operation, and the rules live in one place.** Every path goes through the same steps: it must be absolute without NUL bytes; it is resolved to a canonical path (symbolic links followed) and *that* path is what all later checks and the operation use; `/proc`, `/sys`, `/dev` and the master key are refused; a jail, if the request has one, must contain the canonical path; and changing or deleting opsbox's own binaries, database and state is refused. A property test generates arbitrary mixes of `.`, `..`, empty parts and system-area names, and arbitrary paths through a fixture with links in and out of a jail, and checks that nothing resolves into a denied area or out of a jail. The deny check also runs on the *lexical* path first, so `/tmp/../proc/1` is refused before the disk is touched.

**Roles are enforced in `priv` through a jail, not by serve filtering paths.** `serve` cannot canonicalize (it does not touch the system) and a string prefix check would be defeated by symbolic links. A deployer's requests carry `jail = <apps dir>` and `priv` requires the resolved path inside the resolved jail. Admins have no jail; viewers cannot use Files. (The spec's "deployer restricted to `/var/lib/opsbox/apps/<their services>`" needs the services model of Phase 3; until then a deployer sees the whole apps directory.)

**Saving is atomic and guarded.** A temporary file in the same folder (opened with `O_NOFOLLOW`, mode 0600), written, `fsync`ed, given the old file's mode and owner (a new file gets 0644 and its folder's owner), then renamed over the target, then the folder is synced. The editor remembers the modification time it read and sends it back; a different time on disk is a `409 conflict` and nothing is overwritten. Editing is limited to 5 MB of valid UTF-8 text without NUL bytes: anything else says to download it, because saving a lossy conversion would corrupt it.

**Transfers go through a staging folder, not passed file descriptors.** Spec 7.2 prefers `SCM_RIGHTS`. Passing descriptors needs `unsafe` or another dependency and a binary frame protocol; the staging design is plain files and the existing typed protocol. An upload is streamed from the request body to `xfer_dir/<id>.part` (never held in memory, rejected past the limit, removed on any failure), then `priv` installs it (checks it is a regular file directly inside the transfer folder, opens it with `O_NOFOLLOW`, writes a temporary file beside the destination and renames). A download is copied by `priv` into the transfer folder, opened by `serve` and unlinked immediately, so nothing is left behind however the transfer ends. The cost is one extra copy; the trade is acceptable until very large files matter, and fd passing can replace it behind the same two operations. `priv`'s frame limit rose from 1 MiB to 16 MiB so an editor save (5 MB, more once JSON-escaped) fits.

**Router tiers.** Ordinary API calls keep the 1 MiB body limit and 30 s timeout; the editor route allows 12 MiB (and raises axum's separate `Json` extractor limit, which would otherwise reject it at 2 MiB); upload and download use the configured upload limit and a two-hour timeout. Downloads are never compressed.

**Deleting moves to a trash** (`<unix time>-<id>-<name>` plus a `.meta` note with the original path), kept seven days and purged hourly by `priv`. Deleting a symbolic link removes the link, never its target. A request deleting several things is checked as a whole first: either all of it goes or none. Never deleted: `/`, the top-level system folders, `/var/lib`, and anything of opsbox's own state except the apps tree. Across disks, a file is copied to the trash; a folder is refused ("delete permanently instead"); the page offers trash only.

**A bug the real binary found:** an upload failed on macOS because `/tmp` is a link to `/private/tmp`, so the staged file's folder (as `serve` named it) never matched the canonical transfer folder. The same would happen on Linux with a symlinked configured path. `priv` now compares real paths (the file itself must still not be a link), with a regression test.

### M6: terminal

**The shell runs in `priv`, and the same socket carries the bytes.** `PtyOpen{user, cols, rows}` is a normal typed operation, except that on success the connection stops speaking JSON and becomes a stream of small binary frames (`u32` length, one kind byte, payload: data, resize, exit; at most 16 KiB each). Spec 7.2 prefers passing the PTY master as a file descriptor (`SCM_RIGHTS`); like for file transfers that would need `unsafe` or an extra dependency, and the relayed design also lets `priv` end the shell the moment the stream closes. The cost is one extra copy of terminal bytes, which is nothing at keyboard speed. The frame decoder is pure and tested (round trips, size limits before allocation, malformed input).

**Crate: `pty-process` 0.5.4, not `portable-pty`.** The spec lists `portable-pty`. It is a blocking API (the output thread needs a bridge into async), its last release is from February 2025, and it is built for being a cross-platform terminal emulator host. `pty-process` wraps `tokio::process::Command`, makes the child a session leader with the PTY as controlling terminal (the part that needs `unsafe` with the standard library, which this workspace forbids), is maintained, and is async-native. Both are MIT licensed.

**Login shell as a real login.** When `priv` is root the shell is `runuser -l <user>`: it sets the environment, groups and PAM session like a login does, and it needs no code of ours that changes user ids. Only `TERM=xterm-256color` is added. When `priv` is not root (development, CI) only the user it runs as is allowed, with their own shell as a login shell, and anything else is refused with a sentence. The user name is validated (`useradd` characters only, no leading dash) before it reaches a command line, must exist (looked up in `/etc/passwd`, then through the system's user database so sssd and LDAP users work) and must have a real shell (not `nologin`/`false`).

**Which user, and root.** The spec's default is "the non-root admin user, root only for `admin` with confirmation." `GET /terminal/users` offers accounts with a login shell and an ordinary uid (1000 and up; 500 on macOS for development) plus root; the default is the first ordinary one, never root. The cloud-image question (spec open question 3) is therefore answered by detection: `ubuntu`, `ec2-user`, `admin` are all just the first ordinary account. Root needs `confirm=root` on the socket URL, which the page sends only after a confirmation dialog; the server refuses without it, so a script cannot skip the question. Terminals are admin-only (deployers and viewers get 403 before the upgrade).

**Cleanup when the stream ends.** When the browser, `serve` or the stream goes away, `priv` sends `SIGHUP` to the shell's whole process group, waits two seconds and then `SIGKILL`s the group, and reaps the child, so nothing is left running. A test proves the shell's process is gone after the stream is dropped. When the shell exits by itself the exit code is sent as the last frame.

**Backpressure, with no unbounded buffer anywhere.** Output is read from the PTY only as fast as it can be written to the socket, and `serve` sends to the browser one frame at a time, awaiting each. A slow tab therefore fills the PTY's own kernel buffer and the program blocks, instead of memory growing in `priv` or `serve`. A test floods output (3,000 lines) while the reader is delayed. Pastes are cut into 16 KiB pieces.

**Limits and the audit trail.** 30 minutes without a keystroke closes the session (`[terminal] idle_timeout_min`, 1 to 1440); at most 8 terminals are open at once. Start and end are audited (`terminal.start`, `terminal.end` with duration and reason: `shell_exited`, `closed`, `idle`, `error`), with the account and the caller's address. What is typed or printed is never recorded.

**Known limits of a shell that is a child of `priv` (to be checked on the VM, not provable on macOS).** The shell inherits what `opsbox-priv.service` sets: only `AF_UNIX`, `AF_NETLINK`, `AF_INET` and `AF_INET6` sockets (so `tcpdump` and other packet-socket tools fail), no realtime scheduling, native system calls only. `NoNewPrivileges` is deliberately not set, so `sudo` works. Separately, the shell may live in `opsbox-priv.service`'s control group, in which case systemd would end it, and anything started from it (`tmux`, `nohup`), when `priv` restarts during an update. `runuser -l` goes through PAM, which on most distributions registers a logind session scope for the shell and so moves it out of the service's group; M6's verify steps check this. If it does not hold on a target distribution the fix is to start the shell through `systemd-run --scope` (or a template service) instead of `runuser` directly; the call is isolated in one function.

**Errors are sent inside the socket.** A browser cannot read why an upgrade was refused. So role and Origin problems are HTTP errors before the upgrade (a page on another site must not even get a socket), and everything after that (no such user, root not confirmed, too many terminals, helper down) is an `{"type":"error","code","message"}` message followed by a close, which the page shows verbatim.

**xterm.js 6.0.0 and its fit addon 0.11.0 (both MIT) are vendored** under `web/vendor/xterm/` with their licence, and loaded only when the Terminal page opens. They draw with the DOM renderer, which needs inline styles: if a content security policy is added in M8 it has to allow `style-src 'unsafe-inline'` for the panel (scripts stay `'self'`). Keyboard users are not trapped: Tab belongs to the shell, **Shift+Esc** moves focus out.

**Other servers.** The "SSH to other servers" tabs are present but disabled with "needs the desktop app" (Phase 6). The page in http mode no longer uses the demo hosts or the demo shell, so it has no "sample data" badge; mock mode keeps the demo shell untouched.

### M7: cleanup, access, security and settings

**Cleanup and access.** Cleanup estimates journal, package cache, old release slots, temporary files and core dumps in `priv`. Docker scanners are available only when Docker is present; deployment sources wait for Phase 3. Automatic cleanup is a systemd timer running the safe set. SSH keys are parsed and fingerprinted, written atomically to each Linux user's `authorized_keys`, preserving ownership; locking expires an account rather than merely removing its password. Outgoing SSH keys wait for the desktop phase.

**Security.** Checks use real machine facts and account state; automatic fixes are limited to secret-file permissions and SSH configuration drop-ins (validated before reload). Account invitations are single use, require password and TOTP enrolment and expire after three days. The allowed-address list is checked on requests; changes cannot exclude the caller. Health probes remain accessible.

**Settings.** Host name, system time zone list and active swap are read through the typed `SettingsGet` operation. Host name and time zone changes reuse the M4 operations; swap runs as a task. Panel domains, disk growth, GitHub, self-update, export and uninstall remain sample sections, explicitly labelled and disabled in HTTP mode. Real general settings and token sections have no sample badge.

**API tokens.** `ops_` plus 256 random bits, only the SHA-256 stored, secret returned once. Names are unique, 3–32 lower-case letters, digits and dashes. Scopes in Phase 1 are `read` (host, metrics and attention), `deploy` (the same plus journal reads; no writes until Phase 3) and `full` (administrator operations except account, token, SSH-key and terminal management). Scope checks are endpoint capabilities in addition to role checks: deploy tokens cannot inherit deployer file access. Service-specific deployment scopes await Phase 3's service model. Tokens have no expiry in Phase 1; revoke them explicitly in Settings. Cookie authentication takes precedence over a bearer header, and a bearer secret cannot be used as a cookie. Non-ambient bearer requests require no CSRF token.

### M8: hardening and review of inherited implementation

**Limits without another dependency.** In-memory token buckets precede auth/body parsing (IP: burst 120, refill 2/s; auth/setup/invites/enrolment: 20, refill 1/3s). Authenticated session/token IDs have their own 120/4s budget. Each of the three tables is capped at 4096 keys; a full table rejects new keys instead of evicting a live attacker bucket. Idle entries become reclaimable after 15 minutes. Account/IP lockout remains persisted. Health/static requests are exempt. Tests cover independent budgets, cookie rotation, session sharing across IPs and full-table behavior.

**Request and stream budgets.** Ordinary/editor/transfer body and deadline tiers remain distinct, with JSON 413/408 errors. WebSocket frames/messages are capped at 64 KiB and queued writes at 256 KiB; 64 global stream permits supplement eight terminal slots. A stream rechecks the stored session/token, role and allowed address every five seconds and drops its socket/PTY on revocation, including sign-out elsewhere and CLI recovery. It stores the principal ID, never the credential, and does not refresh idle expiry just because output arrives. Regression tests cover open metrics and terminal revocation. API responses are not cached; CSP blocks embedding, base overrides and objects while retaining Monaco/xterm compatibility.

**Bounded system commands and tasks.** Captured child output is capped at 8 MiB per pipe while the pipe continues to drain (no deadlock). Timeout kills the command's process group, so grandchildren cannot hold the pipe open indefinitely. Tasks keep at most 256 records and 100 bounded log lines. Automatic-update setup and job execution now return task IDs, like installs/swap/cleanup, instead of being cut off by the ordinary HTTP timeout. Task progress uses polling; incremental task/event WebSockets are still deferred.

**Audit before dispatch.** Authentication/CSRF acceptance of an unsafe request must append a hash-chained `request.accepted` row before a handler runs; failure returns 503 without starting a change. Existing named entries add detailed completion results. The acceptance record contains method and path only (no body, query or secret) and distinguishes API-token actors. System commands cannot be transactionally rolled back if a later audit write fails; the pre-dispatch record still establishes the attempt, and detailed audit failures are logged loudly.

**M4 gaps closed during review.** Services now use `zbus` 6.3 on Linux (`ListUnits`, `ListUnitFiles`, `MemoryCurrent`, start/stop/restart/enable/disable and manager reload). This supersedes the original command-line substitution. The helper's existing Tokio runtime drives D-Bus with a 20-second deadline; no second executor or native libdbus dependency. The refusal list still runs before connection/dispatch. Package selection now has its own typed operation, accepts only validated names still present in the pending-update list, and has UI checkboxes. Other modes still install security/all. See the [systemd D-Bus interface](https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.systemd1.html) and [zbus API](https://docs.rs/zbus/6.3.0/zbus/).

**Doctor.** Read-only diagnostics in a typed `Doctor` operation: service/port reachability, two disk paths, NTP synchronization, the system resolver's ability to resolve github.com, optional Docker and encryption-key permissions. The CLI prints concrete fixes but executes none. Failed checks/helper errors exit 1; optional/missing components are warnings/skips. It does not require network egress to succeed: failed DNS resolution is a warning.

**API documentation.** Route inventory is generated from the real axum declarations with a small Python standard-library script and checked in CI. Most response shaping is currently dynamic JSON, so a full utoipa schema would be incomplete; typed request/response migration and full OpenAPI are deferred. Payloads, scopes, security, limits, task persistence and future-phase routes are documented explicitly.

**Shared vendor loader.** Browser smoke testing found that visiting Files first installed Monaco's AMD loader, causing xterm's UMD bundles to register AMD modules instead of browser globals. Both pages now use one shared lazy AMD loader and explicitly consume each module's exports. The full Files → Monaco save → Terminal shell sequence is verified in Chromium, along with opening Terminal directly. The terminal host tab also uses the API's `name` field.

**Verification portability.** A pre-existing file test assumed umask 0022; Ubuntu's 0002 created 0664. The fixture now sets its intended 0644 mode explicitly. CI-safe tests never install packages, alter firewall rules, reboot or mutate real users. Ubuntu arm64 is tested in opsbox-vm; dnf/firewalld and other distributions still need their own VM validation.

## Problems found in the spec

1. **Privilege escalation in the file layout (section 4).** Root runs `/usr/local/bin/opsbox`, a symlink into `/var/lib/opsbox/bin/`, while the unprivileged `opsbox` user must own `/var/lib/opsbox` (database, `ReadWritePaths`). A compromised `serve` could rename `bin/` and put its own binary in its place; root would run it on the next restart. Mitigation implemented: sticky bit (mode 1770, root-owned) so `opsbox` cannot rename or delete entries it does not own. Phase 2 (self-update) should keep the slots out of any directory that `opsbox` can write, or this property must be tested.
2. **`RestrictAddressFamilies=AF_UNIX` on `priv` (section 2) will not survive Phase 1.** Child processes inherit it, and `nft`, `ip` and `wg` talk netlink (`AF_NETLINK`). M4 added `AF_NETLINK AF_INET AF_INET6` so package tools and firewall operations work; priv itself still opens no network listener.

### Phase 1 safety follow-up: console and staging configuration

The network-facing serve process no longer opens `/dev/console`. A typed `ConsoleNotice` request carries a redacted `Secret`; priv bounds the notice to 4096 bytes, requires the setup-banner prefix and rejects control characters before writing the fixed console device. Console failure leaves the journal banner available.

The binary configuration fixes file staging at `/var/lib/opsbox/xfer` by default. Alternate paths (including CLI/environment overrides) require `[files] allow_external_xfer_dir = true`. The isolated memory probe opts in explicitly. This configuration guard does not claim to solve staging-directory replacement races; descriptor-relative file and staging access remains a separate required part.

### Phase 1 safety follow-up: systemd completion and firewalld sources

Service start/stop/restart now register JobRemoved reception and subscribe before submitting the job, match the returned job object path and wait within the existing 20-second operation deadline. A non-done result returns the job result, service Result and ExecMainStatus to the API/UI; a closed signal stream is an error. Enable/disable retain their synchronous manager calls.

Firewalld restricted-source rules use a single rich-rule argv value with an explicit IPv4/IPv6 family, validated source and port/protocol. Add, remove, listing and rollback share the same representation; general anywhere rules retain add-port/remove-port. Tests cover IPv4 addresses/CIDRs, IPv6 CIDRs, add/remove symmetry, listing round-trip and invalid-source rejection.

### Phase 4: proxy and panel domain (prompt 05)

**In-process Rust reverse proxy (`opsbox-proxy`) rather than managed Caddy (spec open question 1).** Managing Caddy would add an external 40+ MB Go binary or require Docker before Phase 3, duplicate configuration state, and increase idle memory footprint. `opsbox-proxy` keeps opsbox a single musl-linked Rust binary, reuses `hyper` 1, `tokio-rustls` 0.26, and `rustls` (already in the workspace with `ring`), adds `instant-acme` 0.8 (no-default-features `hyper-rustls,ring,rcgen`) for RFC 8555 ACME HTTP-01, `hickory-resolver` 0.26 (`tokio,system-config`) for DNS A/AAAA checks, and `x509-parser` 0.18 for certificate expiry tracking. Every dependency is pure Rust and compiles cleanly on `musl` without C OpenSSL or `aws-lc-rs`.

**Separate proxy service with capability drop and `RuntimeDirectory=opsbox-proxy`.** `opsbox-proxy.service` runs as unprivileged `opsbox` with `AmbientCapabilities=CAP_NET_BIND_SERVICE` only (binding `:80` and `:443`). Because `/run/opsbox` is owned `0755` `root:root` by `opsbox-priv`, `opsbox-proxy` uses its own `RuntimeDirectory=opsbox-proxy` and listens on `/run/opsbox-proxy/proxy.sock` (`0660`, kernel peer-credential check allowing only its own UID and root). `serve` pushes routing tables over this Unix socket on startup and whenever the panel domain changes; `opsbox-proxy` persists its last routing table atomically in `/var/lib/opsbox/proxy/routes.json` so restarts survive independently.

**ACME and TLS private keys sealed at rest via `opsbox-priv`.** `opsbox-proxy` runs as `opsbox` and must never read `/etc/opsbox/master.key` (mode `0600` `root:root`). Two typed privilege operations — `PrivOp::SecretSeal` and `PrivOp::SecretOpen` — allow sealing/unsealing AEAD-encrypted blobs only under context strings starting with `"proxy:"`, explicitly refusing `"totp:"` contexts so `proxy` or `serve` can never decrypt user 2FA secrets. ACME account credentials and per-domain PKCS#8 private keys are stored encrypted as `enc:v1:...` JSON in `/var/lib/opsbox/proxy` (mode `0600`), certificates renew automatically 30 days before expiry via a 6-hour background check, and HTTP-01 challenges answer directly on `:80` before redirecting other HTTP traffic (`308`) to HTTPS.

**Host preservation and pinned upstream TLS.** `opsbox-api` enforces `Origin` matching `Host` on cookie-authenticated requests and WebSockets. `opsbox-proxy` therefore forwards the external `Host` header verbatim alongside `X-Forwarded-Proto: https` and `X-Forwarded-For`, verifies `serve`'s self-signed TLS certificate by pinning its SHA-256 leaf fingerprint (`PinnedCertVerifier`), adds `Strict-Transport-Security: max-age=31536000` on panel responses, enforces per-route `max_body_bytes` (`413`), and proxies `101 Switching Protocols` WebSockets cleanly.

**Closing the raw `:8443` listener (`PrivOp::WebLocalOnly`).** Once the panel domain certificate is live, `serve` offers to restrict direct access to `127.0.0.1:8443` (reversible via CLI `opsbox web remote` or UI `Re-open port 8443`). `opsbox-priv` rewrites `[api] listen` atomically in `config.toml` and schedules a detached 0.5-second `systemd-run --no-block` restart of `opsbox.service` so the HTTP `200` confirmation returns to the browser before the listener moves.

**`/readyz` proxy semantics.** When `opsbox.panel.domain` is active, `/readyz` requires `proxy.sock` `Ping` to succeed (`503` if down). When no panel domain is configured, `/readyz` reports `"proxy": "ok"` if reachable and does not fail readiness if `opsbox-proxy` is omitted (preserving `tools/check-idle-memory.py` and existing two-process tests).

### Phase 2: watchdog and safe mode (prompt 06)

**Strict watchdog isolation from `serve` and shared runtime crates.** `opsbox-watchdog` intentionally shares no code with `serve` (`opsbox-api`), `opsbox-auth`, `opsbox-proxy`, or `opsbox-update` beyond `opsbox-proto` types (`WatchdogStatus`, `WatchdogCheck`, `PrivOp`) and read-only SQLite access via `opsbox_store::ReadOnlyStore`. Version slots, disk cleanup, TOTP/session crypto, and Safe Mode HTTP handling live self-contained inside `opsbox-watchdog` so a regression or panic in `serve` cannot prevent Safe Mode recovery. All crates reused in `opsbox-watchdog` (`hyper`, `tokio-rustls` + `ring`, `rcgen`, `argon2`, `chacha20poly1305`, `sha1`, `hmac`, `nix`) were already in `Cargo.lock` with musl compatibility and approved licenses (`deny.toml`).

**`MemoryMax=32M` vs Argon2id 64 MiB login cost.** Default panel accounts hash passwords with `argon2_memory_kib = 65536` (64 MiB), which would exceed `opsbox-watchdog.service`'s `MemoryMax=32M` cgroup limit and trigger OOM kills during Safe Mode login. Before verifying a password hash, `opsbox-watchdog` inspects `/proc/self/cgroup` (`memory.max`) and the PHC `m=` parameter: when constrained, it verifies the password via a short-lived `systemd-run --quiet --pipe --wait --collect --property=MemoryMax=128M <exe> watchdog-verify-pw` scope passing the PHC hash and password exclusively over `stdin` (never argv), keeping idle watchdog memory minimal without weakening panel hashing or hitting the 32 MiB cap.

**TCP loopback `/healthz` probing and shared panel TLS certificate.** `serve` binds TCP TLS on `[api] listen` rather than a Unix socket. `opsbox-watchdog` therefore polls `serve` over loopback TLS (`/healthz` with plain-HTTP fallback for loopback tests), `priv` over `/run/opsbox/priv.sock`, and `proxy` over `/run/opsbox-proxy/proxy.sock`. Because `opsbox-watchdog.service` starts first (`Before=opsbox-priv.service ...`), it loads `/etc/opsbox/panel-selfsigned.{crt,key}` or `/var/lib/opsbox/panel-selfsigned.{crt,key}` when present, or generates the certificate first with `opsbox:opsbox` ownership (`0644`/`0640`) so `serve` and Safe Mode (`:8444`) share the exact same certificate. By default `[watchdog] listen` tracks `[api] listen`'s IP on port `8444` so `opsbox web local-only` (`127.0.0.1` vs `0.0.0.0`) secures both ports together.

**Single-owner post-update verification & Recovery UI boundary.** When `opsbox-watchdog.service` is active, `opsbox-priv`'s `self_update_install` skips spawning the prompt 03 transient `opsbox-rollback-check` timer so `opsbox-watchdog` owns the 2-minute post-update verification and 3-failures-in-10-minutes schema-checked rollback exclusively (`opsbox-update` retains the transient timer fallback only if the watchdog unit is inactive). Per spec 6.4, recovery mutations remain exclusively on the `:8444` Safe Mode server, while `GET /api/v1/recovery` on `:8443` lets `web/js/pages/recovery.js` display live watchdog checks, journal logs, conditional rollback status, and the link/tunnel command to `:8444` (with `opsbox-api` reading `/run/opsbox-watchdog/status.json` via `PrivOp::WatchdogStatus`).

### Phase 3: Docker and Containers page (prompt 07)

**Pure-Rust Docker client (`opsbox-docker`) strictly isolated in `opsbox-priv`.** `opsbox-docker` wraps `bollard` 0.21 with `default-features = false, features = ["pipe"]` (Apache-2.0/MIT crates only: `bollard`, `bollard-stubs`, `hyperlocal`, zero OpenSSL/C dependencies) over `/var/run/docker.sock`. `serve` (`opsbox-api`) never depends on `opsbox-docker` or touches the Docker socket; all inspection, lifecycle actions, log streaming, stats, exec, and cleanup calls traverse typed `PrivOp` frames and re-validate references (`valid_docker_ref`, blocking traversal/NULs) inside `opsbox-priv`. Clients open Unix sockets lazily per call with an explicit version negotiation (`ping_version`) and a dual pre-1.52 / 1.52+ `/system/df` parser so Docker Engine releases before and after 29+ work uniformly without persistent idle sockets inflating RSS.

**Signed repository Docker installation without convenience scripts.** Missing-Docker installation in `opsbox-priv` (`PrivOp::DockerInstall`) never downloads or runs `get.docker.com`. Supported Debian/Ubuntu/RHEL/Fedora distributions either install distro packages (`docker.io`) or fetch Docker's GPG key over HTTPS (`ureq`), verify its primary fingerprint with `gpg --with-colons --show-keys` (`9DC858229FC7DD38854AE2D88D81803C0EBFCD88` for apt, `060A61C51B558A7F742B77AAC52FEB6B621E9F35` for rpm), write validated repo files atomically, and install `docker-ce` cleanly.

**Live logs, Overview integration, and Bollard cleanup scanners.** `GET /api/v1/metrics` preserves `procs` verbatim and adds `containers` (polled every 2 s when Docker is active, backing off to 15 s when missing/stopped); `Attention` raises `"containers"` alerts when containers are `restarting`. Live container logs stream via `PrivOp::DockerLogsFollow` (upgrading the `priv` Unix socket connection like `PtyOpen`) and `WS /api/v1/stream/containers/{id}/logs` with same-origin checks, `stream_slots` caps, and 5-second session revocation checks; read-only `deploy` API tokens can inspect `/api/v1/docker/*` and stream logs without write mutations. Cleanup scanners and pruning (`docker-images`, `docker-buildcache`, `docker-containers`, `docker-volumes`) use `opsbox-docker` directly with a runtime helper supporting both Tokio `priv` requests and synchronous `opsbox cleanup` CLI runs.

### Phase 3: GitHub connection (prompt 08)

**Pure-Rust GitHub App JWT, HMAC verification, and Ed25519 deploy keys with zero new crates.** `opsbox-deploy` reuses crates already in `Cargo.lock` (`ring` 0.17 for PKCS#1/PKCS#8 RS256 App JWT signing and Ed25519 key generation, `pem` 4, `hmac` 0.13 + `sha2` 0.11 for constant-time `X-Hub-Signature-256` verification, `ureq` 3 + `rustls` for GitHub API calls, `url` 2.5, `zeroize` 1.9). Deploy key private keys are encoded directly as `openssh-key-v1` PEM and verified against `ssh-keygen`, avoiding any C/OpenSSL or external keygen dependency at rest.

**Privilege split via `deploy:*` sealed secret contexts in `opsbox-priv`.** `crates/opsbox-priv/src/ops/sealed.rs` extends `ALLOWED_PREFIXES` from `["proxy:"]` to `["proxy:", "deploy:"]`, keeping `totp:*` unconditionally refused. GitHub App RSA private keys (`deploy:github_app:private_key`), webhook secrets (`deploy:github_app:webhook_secret`), and per-repo SSH deploy private keys (`deploy:deploy_key:<id>`) are sealed/opened solely through `PrivOp::SecretSeal` / `SecretOpen` and stored in SQLite (`0005_github_sources.sql`) as `enc:v1:...` ciphertexts.

**`SameSite=Strict` manifest handshake & panel-domain webhook reachability.** Because `opsbox_session` is `SameSite=Strict`, browsers omit it on the initial cross-site redirect from `github.com`. `GET /api/v1/github/callback` and `GET /api/v1/github/setup` therefore perform no state mutation and render a CSP-clean same-origin `<meta http-equiv="refresh">` to `/#/settings?...`, where the authenticated Settings page completes the exchange via `POST /api/v1/github/complete` (with session, CSRF, `Origin`, and single-use SHA-256-hashed `state`). When no panel domain is configured yet, manifests omit `hook_attributes` (with a clear UI notice and a locally generated fallback secret) and automatically or manually sync `https://<panel-domain>/api/v1/github/webhook` via `PATCH /app/hook/config` once a panel domain certificate goes live. `/api/v1/github/webhook` is exempt from the panel IP allow-list so GitHub's external webhook IPs reach it through `opsbox-proxy`, gated strictly by constant-time HMAC-SHA256 verification.

**Remote build-method detection without cloning.** `GET /api/v1/github/repos` and `POST /api/v1/github/public-repo` inspect root directory entries and `package.json` over GitHub's REST Contents API (bounded concurrency of 6, cached by `(full_name, pushed_at)`), detecting `Dockerfile`, `Docker Compose`, `static site`, `Nixpacks · <runtime>`, or `nothing to build` while keeping container building and running deferred to prompt 09.

### Phase 3: Deploy pipeline (prompt 09)

**Nixpacks as an optional host CLI rather than a bundled dependency.** Bundling `nixpacks` inside opsbox would ship a 30+ MB external Rust binary with its own transitive crate/toolchain tree and break the single-binary musl size/idle footprint budget. `opsbox-priv` therefore resolves builds in strict order: `Dockerfile`/`Containerfile` first, then built-in static-site Dockerfiles (`nginx:1.27-alpine` directly for static HTML or multi-stage `node:22-alpine` -> `nginx:1.27-alpine` when `package.json` defines `scripts.build`), and finally an optional host-installed `nixpacks` CLI (failing cleanly with installation guidance if absent).

**BuildKit `--secret` mounts and capability-probed builder flags.** All `service_vars` and Git credentials are sealed at rest with `PrivOp::SecretSeal` (`deploy:*` contexts) and opened only inside `opsbox-priv` (`ops/deploy.rs`). Non-secret variables pass as `--build-arg`, whereas secrets never pass on argv/ENV or enter image layers: `opsbox-priv` writes them to `0600` files inside an ephemeral `0700` directory and mounts them via `--secret id=KEY,src=...` alongside `0700` `GIT_ASKPASS` / `0600` SSH key files dropped when the build finishes. All build/health-check logs and errors pass through longest-first secret scrubbing before leaving `priv`. Before invoking Docker, `opsbox-priv` probes `docker buildx version` and `--help` so both `docker buildx build --load` (with `--progress=plain` / `--cache-from`) and legacy `DOCKER_BUILDKIT=1 docker build` work without flag mismatches.

**Zero-downtime loopback upstream and `opsbox-proxy` plain HTTP service routes.** New release containers start on the internal `opsbox-<project>` bridge network with `<service>` DNS alias and publish `127.0.0.1:0:<port>` (never `0.0.0.0`, or no published port for `port = 0` workers). `opsbox-priv` inspects the dynamically assigned host port and verifies HTTP `2xx`/`3xx` (default `/`, or `tcp` connect / worker stability check) before `serve` atomically flips DB state, updates `opsbox-proxy` (`handle_http` forwards routes with `redirect_https = false` so optional single-domain HTTP routes work before Prompt 10 issues per-app ACME certs), drains the previous container (`drain_secs = 10` default), stops it, and retains the newest 5 images per service so rollback runs steps 6–8 with zero rebuild.

**Per-service deployer jail and `deploy:<service>` token scopes.** Signed-in deployers (`Role::Deployer`) only own/manage their assigned service directories (`GET /files?path=<apps_dir>` filters to owned service directory names; all subpaths validate the first component against owned services before `opsbox-priv` enforces canonical symlink jailing). API tokens (`opsbox-store` migration `0006`) extend `TokenScope` with `DeployService(String)` (`deploy:<service>`), validated across `opsbox-auth` and `opsbox-api` so tokens can only trigger/read deploys, rollbacks, build logs, and tasks for that exact named service and never inherit file access, variable writes, or service deletion.
