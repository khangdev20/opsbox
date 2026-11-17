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

**WebSocket.** `WS /api/v1/stream/metrics` sends the newest frame at once and then one frame per second, each the same object `GET /metrics` returns, so the page is stateless. It authenticates before looking at the upgrade, and requires an `Origin` that names this host (browsers always send one on a WebSocket), which closes cross-site WebSocket hijacking. A slow client skips frames instead of buffering. A session that ends while a socket is open is noticed at the next reconnect, not mid-stream; ending open sockets on sign-out is left for the hardening pass.

**Replacing the mock tick.** In http mode the page opens one socket after sign-in (not one per page); every frame refreshes the status bar and the pages that subscribed. If the socket drops, it reconnects with a growing delay and, if the reason is an ended session, sends the person to sign-in instead of retrying. Mock mode keeps its two-second tick.

**Attention** (spec 7.3): security updates, reboot required, disks over 75% (not at 75%), failed units, each with a sentence and a link in the UI's terms. Facts come from a new `SystemFacts` operation, refreshed every five minutes because it can run `apt-check`: reboot flag from `/run/reboot-required`, failed units from `systemctl list-units --state=failed`, updates from Ubuntu/Debian's `apt-check`. Children run with a cleared environment, absolute paths, no shell and a timeout. On other distributions update counts are unknown and simply not reported rather than reported as zero; M4's package module replaces this. The disk item says how full the disk is; the mock's "N GB can be reclaimed" returns with the cleanup scanners (M7).

**Development off Linux.** Off Linux the sampler uses `sysinfo` (CPU ticks synthesized from instantaneous usage, memory, load, processes, `/` usage, no network or disk I/O) so the whole UI can be developed on a laptop. The `/proc` parsers are pure functions tested on fixture text everywhere, and a Linux-only test reads the real files; CI is its first run.

## Problems found in the spec

1. **Privilege escalation in the file layout (section 4).** Root runs `/usr/local/bin/opsbox`, a symlink into `/var/lib/opsbox/bin/`, while the unprivileged `opsbox` user must own `/var/lib/opsbox` (database, `ReadWritePaths`). A compromised `serve` could rename `bin/` and put its own binary in its place; root would run it on the next restart. Mitigation implemented: sticky bit (mode 1770, root-owned) so `opsbox` cannot rename or delete entries it does not own. Phase 2 (self-update) should keep the slots out of any directory that `opsbox` can write, or this property must be tested.
2. **`RestrictAddressFamilies=AF_UNIX` on `priv` (section 2) will not survive Phase 1.** Child processes inherit it, and `nft`, `ip` and `wg` talk netlink (`AF_NETLINK`). It is set now as the spec says; expect to add `AF_NETLINK` when the firewall and WireGuard operations land.
