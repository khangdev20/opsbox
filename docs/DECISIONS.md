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

## Problems found in the spec

1. **Privilege escalation in the file layout (section 4).** Root runs `/usr/local/bin/opsbox`, a symlink into `/var/lib/opsbox/bin/`, while the unprivileged `opsbox` user must own `/var/lib/opsbox` (database, `ReadWritePaths`). A compromised `serve` could rename `bin/` and put its own binary in its place; root would run it on the next restart. Mitigation implemented: sticky bit (mode 1770, root-owned) so `opsbox` cannot rename or delete entries it does not own. Phase 2 (self-update) should keep the slots out of any directory that `opsbox` can write, or this property must be tested.
2. **`RestrictAddressFamilies=AF_UNIX` on `priv` (section 2) will not survive Phase 1.** Child processes inherit it, and `nft`, `ip` and `wg` talk netlink (`AF_NETLINK`). It is set now as the spec says; expect to add `AF_NETLINK` when the firewall and WireGuard operations land.
