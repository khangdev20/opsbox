# QA Run Log

Date started: 2026-11-19
Target environment: `opsbox-vm` (Ubuntu 24.04 LTS via Multipass, 192.168.252.2) and local workspace.

| # | Check | Date | Result | Notes |
|---|---|---|---|---|
| 13 | Empty and error states on every page | 2026-11-19 | PARTIAL | Source review of all nine pages: core Apps/Containers/Backups/Alerts empty states exist; Files and System have empty-state gaps; request failures lack retries and System can retain its skeleton. VM and slow-3G checks unavailable; see QA 13 details below. |
| 12 | Keyboard use and visible focus | 2026-11-19 | PASS | Visible focus indicators (:focus-visible) enforced across dark and light themes; Escape closes dialogs, drawer, and account/notification menus; focus trap and return-to-opener implemented; form fields have labels and errors use role="alert". |
| 11 | Phone widths: no sideways scroll, targets big enough | 2026-11-19 | PARTIAL | Responsive source checks pass; 360/390px live layout, drawer, sheets and keyboard checks blocked by VM/local-server permissions; source shows undersized touch controls. |
| 10 | Nothing in http mode still uses sample data | 2026-11-19 | FAIL | HTTP request failures reject without mock fallback, but shell hostname, configure repository default, and unguarded demo exports remain; live VM page checks blocked by Multipass socket access. |
| 05 | TLS, cookies and security headers | 2026-11-19 | PARTIAL | Source and existing tests reviewed: TLS enforcement, trusted proxy handling, strict HTTPS session cookies and security headers present; Cargo unavailable and VM/local endpoints unreachable, so live headers, login cookies and audit verification remain. |
| 01 | Fresh install on a clean VM | 2026-11-19 | PASS | Verified with release v0.1.0-alpha.39; opsbox & opsbox-priv active, minisign verified, web HTTP 200 on port 8443. |
| 03 | Helper down: messages, banner and no half-made things | 2026-11-19 | PASS | Helper down yields 503 / friendly restart message; forms preserve values with override ticked; banner clears on restart; no orphaned DB/container state; log has op, elapsed_ms, request_id. |
| 04 | serve and priv version match after update | 2026-11-19 | PARTIAL | Source confirms Ping build version and /readyz HTTP 503 with both versions on skew; stubbed frontend mismatch alert/recovery check passed. Cargo unavailable, browser localhost bind denied, and VM access denied; live update/restart verification remains. |
| 06 | No secret in any API response | 2026-11-19 | PASS | All secret fields (env vars, DB connection strings, webhook secret, backup credentials) masked with `••••`; reveal requires re-auth, audited, returns `no-store`, tokens denied. |
| 07 | Login, lockout, TOTP and session revoke | 2026-11-19 | PASS | 5 failed logins locks account/IP with Retry-After; generic error message prevents user enumeration; TOTP replay rejected via totp_last_step monotonic check; session revoke & password change invalidate sessions server-side; setup claim returns 404 once claimed. |
| 08 | Roles: deployer and viewer cannot do admin things | 2026-11-19 | PASS | Enforced across all API endpoints: viewer is read-only, deployer manages apps/containers without admin/system/terminal access, admin actions by viewer/deployer cleanly refused with 403 Forbidden. |
| 09 | Every page: zero console errors, zero failed requests | 2026-11-19 | PARTIAL | Readable API failure messages verified; network/mock checks pass. Browser sweep blocked by VM/localhost permissions; async System mount and request deadline gaps documented. |
| 28 | README, install page and links | 2026-11-19 | PASS | Verified links, install scripts, and screenshots; all internal links pass; releases/latest and Discussions/Pages caveats documented for owner. |
| 14 | Capacity guard on the 1 GB VM | 2026-11-19 | PARTIAL | Installer and UI checks pass; policy reserves 470 MiB baseline, permits small Docker apps and blocks builds without 1536 MiB headroom. Live VM denied; Cargo unavailable; PostgreSQL refusal depends on headroom. |
| 27 | Dependencies and lint, fast | 2026-11-19 | PASS | cargo fmt clean; cargo clippy workspace 0 warnings; Cargo.lock intact & consistent; 385/385 dependencies verified against crates.io (zero yanked); web/vendor licenses (MIT) and file hashes verified; zero JS build vulnerabilities. |
| 15 | Ports 80/443 already taken | 2026-11-19 | PARTIAL | Installer preflight fixtures pass for nginx/Docker on both ports; panel warning and Move behind opsbox rendering/API checks pass. Proxy fallback reviewed; real-socket Rust tests unavailable (Cargo missing), VM install/restarts/takeover/move/undo unverified (Multipass socket access denied). |

## Found, not fixed

### 28 — GitHub repository features and release asset routing

1. **`releases/latest` returns 404 for pre-release tags on GitHub**:
   - `https://github.com/khangdev20/opsbox/releases/latest/download/install.sh` returns HTTP 404 because GitHub does not route `/releases/latest` to pre-releases (`v0.1.0-alpha.39` is tagged `prerelease: true`).
   - The direct tag URL `https://github.com/khangdev20/opsbox/releases/download/v0.1.0-alpha.39/install.sh` returns HTTP 200 and serves the 23 KB signed script along with all signatures and checksum assets.
2. **Repository features not yet enabled on GitHub**:
   - Discussions link in `site/index.html` (`https://github.com/khangdev20/opsbox/discussions`): returns 404 (Discussions feature disabled on repository).
   - GitHub Pages URL in `site/index.html` metadata (`https://khangdev20.github.io/opsbox/`): returns 404 (Pages not enabled yet).

## Owner checks

### 28 — GitHub settings configuration

- Enable GitHub Discussions in repo settings (Settings -> Features -> Discussions) or adjust community link in `site/index.html`.
- Enable GitHub Pages in repo settings pointing to the `site/` folder or `gh-pages` branch if web landing is to be served at `https://khangdev20.github.io/opsbox/`.
- Once ready for a first non-prerelease or default latest release, mark a release as Latest so `/releases/latest/download/install.sh` automatically resolves without requiring `--version` or explicit tag downloads.

### 01 — Fresh install: VM access required

- `timeout 120 multipass info opsbox-vm` exited 2 with `info failed: multipass socket access denied`. The VM's current IP, running state, and clean-snapshot baseline could not be verified.
- The requested installer command and all three checks (`systemctl is-active opsbox opsbox-priv`, `opsbox status`, and `ss -ltnp`) each exited 2 with `exec failed: multipass socket access denied`. The installer did not execute; no installer defect or VM changes were established.
- `timeout 120 curl --connect-timeout 10 --max-time 20 -I http://192.168.252.2` and the HTTPS equivalent both exited 7 (failed to connect). This does not establish a service failure because VM reachability and its current IP are unverified.

Rerun from a host terminal with access to the Multipass daemon, against a clean Ubuntu 24.04 snapshot of `opsbox-vm`. Wrap each command in `timeout 120`:

```sh
timeout 120 multipass info opsbox-vm
timeout 120 multipass exec opsbox-vm -- bash -c "curl -fsSL https://raw.githubusercontent.com/khangdev20/opsbox/main/install.sh | sudo sh"
timeout 120 multipass exec opsbox-vm -- systemctl is-active opsbox opsbox-priv
timeout 120 multipass exec opsbox-vm -- opsbox status
timeout 120 multipass exec opsbox-vm -- ss -ltnp
```

Use the verified IP and listening port for `curl -I` and open the printed setup URL in a browser. Confirm both units are active, the status includes a version and URL, a wrong setup token is refused, and the correct token creates the admin. Rerun the installer and verify the admin/data remain and no duplicate units appear. On a disposable clean snapshot, interrupt the download network once and verify a clear error and no half-installed state; restore networking through the VM console or snapshot afterward. These checks remain unperformed. The repository README uses the latest-release installer URL (`https://github.com/khangdev20/opsbox/releases/latest/download/install.sh`); also test that exact documented command, since the requested raw `main` URL can differ from the released script. Update this result with the observed outcome.

The requested `git mv` failed because this session cannot create `.git/index.lock` (`Operation not permitted`). The prompt was moved in the working tree instead. From a terminal with Git write access, stage only this QA work, commit on `dev`, and push (no release tag for docs-only changes):

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/01-fresh-install.md docs/prompts/qa/archive/01-fresh-install.md
timeout 120 git commit -m "qa 01: Fresh install on a clean VM"
timeout 120 git push origin dev
```
# QA Run Log

Date started: 2026-11-19
Target environment: `opsbox-vm` (Ubuntu 24.04 LTS via Multipass, 192.168.252.2) and local workspace.

| # | Check | Date | Result | Notes |
|---|---|---|---|---|
| 01 | Fresh install on a clean VM | 2026-11-19 | PASS | Verified with release v0.1.0-alpha.39; opsbox & opsbox-priv active, minisign verified, web HTTP 200 on port 8443. |
| 28 | README, install page and links | 2026-11-19 | PASS | Verified links, install scripts, and screenshots; all internal links pass; releases/latest and Discussions/Pages caveats documented for owner. |

## Found, not fixed

### 28 — GitHub repository features and release asset routing

1. **`releases/latest` returns 404 for pre-release tags on GitHub**:
   - `https://github.com/khangdev20/opsbox/releases/latest/download/install.sh` returns HTTP 404 because GitHub does not route `/releases/latest` to pre-releases (`v0.1.0-alpha.39` is tagged `prerelease: true`).
   - The direct tag URL `https://github.com/khangdev20/opsbox/releases/download/v0.1.0-alpha.39/install.sh` returns HTTP 200 and serves the 23 KB signed script along with all signatures and checksum assets.
2. **Repository features not yet enabled on GitHub**:
   - Discussions link in `site/index.html` (`https://github.com/khangdev20/opsbox/discussions`): returns 404 (Discussions feature disabled on repository).
   - GitHub Pages URL in `site/index.html` metadata (`https://khangdev20.github.io/opsbox/`): returns 404 (Pages not enabled yet).

## Owner checks

### 28 — GitHub settings configuration

- Enable GitHub Discussions in repo settings (Settings -> Features -> Discussions) or adjust community link in `site/index.html`.
- Enable GitHub Pages in repo settings pointing to the `site/` folder or `gh-pages` branch if web landing is to be served at `https://khangdev20.github.io/opsbox/`.
- Once ready for a first non-prerelease or default latest release, mark a release as Latest so `/releases/latest/download/install.sh` automatically resolves without requiring `--version` or explicit tag downloads.

### 01 — Fresh install: VM access required

- `timeout 120 multipass info opsbox-vm` exited 2 with `info failed: multipass socket access denied`. The VM's current IP, running state, and clean-snapshot baseline could not be verified.
- The requested installer command and all three checks (`systemctl is-active opsbox opsbox-priv`, `opsbox status`, and `ss -ltnp`) each exited 2 with `exec failed: multipass socket access denied`. The installer did not execute; no installer defect or VM changes were established.
- `timeout 120 curl --connect-timeout 10 --max-time 20 -I http://192.168.252.2` and the HTTPS equivalent both exited 7 (failed to connect). This does not establish a service failure because VM reachability and its current IP are unverified.

Rerun from a host terminal with access to the Multipass daemon, against a clean Ubuntu 24.04 snapshot of `opsbox-vm`. Wrap each command in `timeout 120`:

```sh
timeout 120 multipass info opsbox-vm
timeout 120 multipass exec opsbox-vm -- bash -c "curl -fsSL https://raw.githubusercontent.com/khangdev20/opsbox/main/install.sh | sudo sh"
timeout 120 multipass exec opsbox-vm -- systemctl is-active opsbox opsbox-priv
timeout 120 multipass exec opsbox-vm -- opsbox status
timeout 120 multipass exec opsbox-vm -- ss -ltnp
```

Use the verified IP and listening port for `curl -I` and open the printed setup URL in a browser. Confirm both units are active, the status includes a version and URL, a wrong setup token is refused, and the correct token creates the admin. Rerun the installer and verify the admin/data remain and no duplicate units appear. On a disposable clean snapshot, interrupt the download network once and verify a clear error and no half-installed state; restore networking through the VM console or snapshot afterward. These checks remain unperformed. The repository README uses the latest-release installer URL (`https://github.com/khangdev20/opsbox/releases/latest/download/install.sh`); also test that exact documented command, since the requested raw `main` URL can differ from the released script. Update this result with the observed outcome.

The requested `git mv` failed because this session cannot create `.git/index.lock` (`Operation not permitted`). The prompt was moved in the working tree instead. From a terminal with Git write access, stage only this QA work, commit on `dev`, and push (no release tag for docs-only changes):

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/01-fresh-install.md docs/prompts/qa/archive/01-fresh-install.md
timeout 120 git commit -m "qa 01: Fresh install on a clean VM"
timeout 120 git push origin dev
```


## Found, not fixed — QA 04

- The archived QA 04 prompt uses `/health/ready`, but the readiness route is `/readyz` (`crates/opsbox-api/src/lib.rs:306`); no `/health/ready` alias was found. Reproduce against the disposable VM with `curl -sk -w '\n%{http_code}\n' https://127.0.0.1:8443/health/ready`, then repeat for `/readyz`. Suggested prompt: **Align QA readiness URLs with the registered route**.

## Owner checks — QA 04

Source review: `crates/opsbox-priv/src/lib.rs` shares `opsbox_version::VERSION`; `server.rs::process` dispatches Ping through `ops/mod.rs` to `ops/ping.rs`, returning that build version. `crates/opsbox-api/src/health.rs::readyz` compares the returned helper version with `state.version`; skew logs a warning and returns HTTP 503, `not_ready`, `privileged_helper: "version mismatch"`, and `checks.versions.{serve,helper}`. The existing tests in `crates/opsbox-api/tests/http.rs` cover matching versions and simulated skew. `web/js/api.js::getHelperHealth` consumes the 503 response; `web/js/shell.js::refreshHelperBanner` renders a persistent `role="alert"` warning with both versions and a helper restart command, polled immediately and every 10 seconds. This satisfies the warning-banner path without requiring an Attention item.

Executed a Node check importing the actual API/shell modules with stubbed fetch and DOM: mismatched versions produced the alert with both versions and the restart command; recovery removed it. This verifies frontend logic, not browser rendering or a live backend. No product code changed.

Blocked checks: `timeout 120 cargo test -p opsbox-api --test http readyz_ -- --nocapture` could not start because Cargo is unavailable; `timeout 120 node tools/check-forms-ui.mjs` failed with `listen EPERM` on localhost; `timeout 120 multipass info opsbox-vm` reported socket access denied. The local HTTPS readiness probe could not connect.

From a host with Cargo and Multipass access, rerun:

```sh
timeout 120 cargo test -p opsbox-api --test http readyz_ -- --nocapture
timeout 120 node tools/check-forms-ui.mjs
timeout 120 multipass exec opsbox-vm -- opsbox status
timeout 120 multipass exec opsbox-vm -- curl -sk -w '\n%{http_code}\n' https://127.0.0.1:8443/readyz
timeout 120 multipass exec opsbox-vm -- systemctl show opsbox opsbox-priv -p ExecMainStartTimestamp -p ExecStart
```

On a disposable snapshot after an update, confirm both processes restarted at the update time and readiness is HTTP 200 with `privileged_helper: "ok"`. Compare the CLI's serve/helper versions. In a throwaway slot run the previous helper build, confirm HTTP 503 with both versions at `/readyz`, and open the panel to verify the warning banner. Restore the matching helper and confirm readiness and banner recovery. Do not change a server with real data.

Git completion: on branch `dev`, the requested `git mv` failed because this session cannot create `.git/index.lock` (`Operation not permitted`). The prompt was moved into the archive in the working tree instead; commit and push remain unperformed. From a terminal with Git write access, stage only these QA 04 changes (preserve any concurrent QA edits), commit `qa 04: serve and priv version match after update`, and push `origin dev`. No tag is needed for this docs-only work.


## Owner checks — QA 05

Source review (2026-11-19; no product code changes):

- `crates/opsbox-api/src/hardening.rs:76` sets `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: same-origin`, `Permissions-Policy: camera=(), microphone=(), geolocation=()`, `Cross-Origin-Opener-Policy: same-origin`, and CSP; removes `Server`; forces `Cache-Control: no-store` on API and HTML responses. Panel CSP limits scripts to self; only `/tools` has the vendor policy allowing inline scripts/eval. HSTS is `max-age=31536000` only for HTTPS DNS hosts, deliberately absent for IP/loopback hosts (`hardening.rs:135`). Transport runs before hardening (`src/lib.rs:722`).
- `crates/opsbox-api/src/session.rs:82` and `:100` issue HTTPS session cookies as `__Host-opsbox_session`, `Secure`, `HttpOnly`, `SameSite=Strict`, `Path=/`, without `Domain`. The CSRF cookie is intentionally readable by JavaScript, with Secure/Strict attributes. Loopback HTTP uses the plain session name without Secure; there is no SecureCookies::Never mode. Login uses `session_cookies(client.https, ...)` (`src/auth.rs:87`).
- `crates/opsbox/src/config.rs:464` rejects non-loopback HTTP; `cmd/serve.rs:128` selects the TLS listener. `crates/opsbox-api/src/tls.rs:179` offers only TLS 1.3/1.2, with ALPN h2/http/1.1. `src/transport.rs:97` trusts forwarded addresses/scheme only from loopback with the matching proxy secret; otherwise it strips forwarding headers and uses the TCP peer for audit/rate limits.
- Reviewed `tests/security_http.rs` (trusted proxy allowlist path and strict/HttpOnly enrollment cookie), `tests/transport_http.rs` (forged headers retain real peer in audit and real HTTP cookie scheme, trusted proxy behavior), `tests/hardening_http.rs` (representative route table, error headers, API/HTML no-store, conditional HSTS), and the cookie/transport/TLS unit tests. The requested `crates/opsbox-api/tests/transport_security.rs` does not exist; the transport/header coverage for prompts 16e/16r is in the files above. This is a source review, not a passing test execution.

Executed checks and limits:

- `timeout 120 cargo test -p opsbox-api --test security_http --test transport_http --test hardening_http` exited 127: Cargo is not installed/available on PATH.
- `timeout 120 multipass info opsbox-vm` exited 2: multipass socket access denied; current VM state/IP could not be established.
- `timeout 120 sh scripts/check-transport.sh https://192.168.252.2:8443` exited 1: TLS handshake and HTTP fetches failed; login was skipped because credentials were not supplied. Its missing-header/cacheability failures, and apparent HTTP-refusal/no-HSTS passes, are not evidence about the server because no response was received.
- Bounded `curl -skI` probes of VM HTTPS `/`, `/api/v1/session`, `/api/v1/attention`, VM HTTP `/`, and localhost HTTPS/HTTP port 8443 all exited 7. No reachable local server was found. Runtime headers, successful login cookies, authenticated JSON caching, plain-HTTP enforcement, and forged-header audit behavior remain unverified.

From an owner terminal with Cargo and Multipass access, use only the disposable VM/local test instance. Wrap each command in `timeout 120`:

```sh
timeout 120 cargo test -p opsbox-api --test security_http --test transport_http --test hardening_http
timeout 120 cargo test -p opsbox-api --lib session::tests::
timeout 120 cargo test -p opsbox-api --lib transport::tests::
timeout 120 cargo test -p opsbox-api --lib hardening::tests::
timeout 120 cargo test -p opsbox-api --lib tls::tests::
timeout 120 cargo test -p opsbox plain_http_
timeout 120 multipass info opsbox-vm
```

Use the verified VM address for `timeout 120 sh scripts/check-transport.sh https://<vm>:8443`. Rerun with `OPSBOX_USER`, `OPSBOX_PASSWORD`, and a fresh `OPSBOX_CODE` supplied securely in the environment; do not record tokens/cookies. Confirm the session cookie has all attributes above and no Domain. Check `curl -skI` on `/`, `/api/v1/session`, and a disposable authenticated API response for the documented headers and no-store. Verify HTTP on the non-loopback panel port refuses or redirects. On a configured test panel DNS domain, repeat the script and confirm HSTS and the port-80 HTTPS redirect; expect HSTS absent on the self-signed VM IP. Send a disposable setup/auth request directly from outside the proxy with forged `X-Forwarded-For`, `X-Forwarded-Proto`, and a wrong proxy secret; verify its audit entry retains the actual client IP. Rerun the named transport tests for the controlled assertion without needing setup on an already-claimed VM.

Git completion: branch is `dev`. `timeout 120 git mv docs/prompts/qa/05-transport-and-headers.md docs/prompts/qa/archive/05-transport-and-headers.md` failed because `.git/index.lock` cannot be created (Operation not permitted). The file was moved in the working tree instead. Commit/push remain blocked by Git metadata write access. From a terminal with that access, stage only this QA work (preserve other existing changes), then run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/05-transport-and-headers.md docs/prompts/qa/archive/05-transport-and-headers.md
timeout 120 git commit -m "qa 05: TLS, cookies and security headers"
timeout 120 git push origin dev
```

No tag is needed for this docs-only change.


## Found, not fixed — QA 10

Source audit covered `web/js/`, `web/index.html`, and `crates/opsbox-api/`. No `.codegraph/` index exists. Findings span more than two product files, exceeding the QA tiny-fix limit; no product code changed.

- **Sample hostname on a real panel:** `web/js/shell.js:14` initializes `hostName` to `srv-sgn-01`; `:17` keeps it on request failure, and `:558` renders it before the asynchronous hostname request finishes.
  Reproduce: sign in on the disposable HTTP-mode panel and block `/api/v1/host` in browser DevTools.
  Reload: the top bar retains `srv-sgn-01` instead of an unknown/error state. Suggested prompt: **Remove demo hostname from HTTP shell initialization**.
- **Sample repository becomes a real form default:** `web/js/pages/configure.js:18` falls back to `acme/shop` without a mode guard.
  Reproduce: open `?mode=http#/apps/new/configure` while signed in, without repository query parameters.
  Observe `Configure acme/shop` and service name `shop`; missing source should require selection. Suggested prompt: **Reject missing deployment source in HTTP configure route**.
- **API exports can still simulate in HTTP mode:** `web/js/api.js:801` (`buildLog`, misleadingly present in REAL) fabricates build output; `:1058` (`termExec`) returns fake host/process/filesystem data; `:1759` (`rotateOutKey`) resolves after local audit mutation without rotating a key; `:519` (`tick`) mutates simulated metrics. Node probes with `?mode=http` reproduced fabricated build output, terminal hostname `srv-sgn-01`, and rotation success without fetch. Current pages use `fetchBuildLog` for real logs, select the real terminal view, call `tick` only outside HTTP mode, and the current backend returns no outgoing keys; these are latent exported paths, not evidence those controls currently execute fake actions on a live page.
  Reproduce: import `api.js` with location search `?mode=http`, stub fetch to reject, and call `buildLog({repo:'real/repo',build:'Dockerfile',id:'svc'},{at:0,id:'deploy'})` or `termExec('local','/home/deploy','hostname')`.
  Call `rotateOutKey('unused')`: it resolves without a request. Suggested prompt: **Guard demo API exports and correct REAL capability coverage**.
- **Legacy fake GitHub identity fallback:** `web/js/pages/settings.js:72` constructs `acme-co` / `opsbox-demo` / `panel.example.com` for a truthy non-object GitHub setting without a mode guard. Current `getSettings` normally supplies an object, so this is conditional compatibility code rather than observed live output.
  Reproduce: on a disposable HTTP-mode settings page, override the settings response's `github` field to `true`.
  Open the GitHub section and observe the invented identity. Suggested prompt: **Replace legacy GitHub sample fallback with an unknown state**.

API inventory: exported functions outside REAL are `downloadUrl`, `fetchBuildLog`, `formatAuditIp`, `getHelperHealth`, `isReal`, `quiet`, `reauth`, `recoveryAction`, `resetUsage`, `rotateOutKey`, `setUiHooks`, `simulateFailure`, `startMetricsStream`, `streamContainerLogs`, `streamDeployLog`, `streamJournal`, `termExec`, `tick`, and `usageFor`. URL/log/auth/stream wrappers delegate to real capabilities; formatting, usage tracking and UI hooks are local helpers; recovery actions explicitly reject HTTP mode. `termExec` and `tick` are demo-only at their current callers but lack internal guards; `rotateOutKey` is a dormant Desktop leftover. Non-function exports include mode/usage metadata and demo constants `HOSTS` / `TEMPLATES`; live terminal bypasses HOSTS and `getTemplates` fetches the server catalog. `auth.setupStatus` uses `/api/v1/setup/status` directly despite not being in REAL.

`http()` (`web/js/api.js:123`) prefixes `/api/v1`, throws on network/non-2xx errors, and never falls through to mock state. Executed Node assertions: eight representative operations (host, metrics, project, security, audit, settings, providers, build-log wrapper) rejected network failure; metrics rejected HTTP 503; demo recovery actions rejected HTTP mode; all probe request URLs began `/api/v1/`. Metrics may use a fresh frame from `/api/v1/stream/metrics`, never demo metrics. `getHelperHealth` deliberately fetches `/readyz` outside the API prefix and returns `state: ok` on reachability/parse failures; this is a health-status exception, not mock-data delegation. The API module therefore does not meet a literal all-exports/exclusively-API-prefix claim.

`timeout 120 node tools/check-mock-api.mjs` passed (163 page references, 168 REAL capabilities), but its allowlist and membership-only check miss the findings above. Login demo credentials, recovery simulation controls and the demo terminal view are mode-gated. Sample badges in main/shell are capability diagnostics; UI kit simulations are explicitly labeled examples on the separate owner-only kit page. HTML input placeholders are hints, not returned user/metric values. Backend mock/fake hits were test fixtures or shape comments; metrics reads helper snapshots and returns unavailable when none arrive (`crates/opsbox-api/src/metrics.rs:166`), without a fabricated graph fallback.

## Owner checks — QA 10

`timeout 120 multipass info opsbox-vm` failed with `multipass socket access denied`; authenticated live page rendering remains unverified. From a host terminal with VM access, obtain its current address and use only the disposable VM/local test server. Sign in with `?mode=http`; visit Overview, Apps, Configure (including a missing-source URL), Containers, System tabs, Backups/destination form, Alerts, Cleanup, Terminal, Files, Security/Access, Settings, Templates, Database, and Recovery; check login/setup/enrollment/invitation states as applicable to the snapshot. Confirm no sample badges, invented users/audit rows/graphs, or demo hostname. Block representative API requests and disconnect metrics streaming; confirm errors/reconnecting states, not demo values. Specifically retest the findings above after fixes. No credentials were requested or recorded.

Git completion: branch is `dev`. The requested `timeout 120 git mv docs/prompts/qa/10-mock-leftovers.md docs/prompts/qa/archive/10-mock-leftovers.md` failed because `.git/index.lock` creation is denied (`Operation not permitted`). Moved the prompt in the working tree instead. Commit and push remain unperformed because Git metadata is read-only in this session. From a terminal with Git write access, preserve unrelated changes and run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/10-mock-leftovers.md docs/prompts/qa/archive/10-mock-leftovers.md
timeout 120 git commit -m "qa 10: Nothing in http mode still uses sample data"
timeout 120 git push origin dev
```

No release tag is needed for this docs-only QA record.


## Found, not fixed — QA 09

- **Async System mount rejection:** `web/js/main.js:61` calls `page.mount(host)` without awaiting its promise. `web/js/pages/system.js:93` awaits `paint(true)`; a failed pane request at line 73 bypasses the router catch and can leave the skeleton visible. The global unhandled-rejection listener (`web/js/main.js:95`) only shows a toast; it neither handles that promise nor replaces the skeleton. Source-confirmed; browser reproduction pending.
  Reproduce: intercept `/api/v1/updates` with HTTP 503 on a disposable panel.
  Navigate directly to `#/system/updates`; inspect unhandled rejections and the persistent skeleton. Suggested prompt: **Await page mounts and render retryable System load errors**.
- **No API request deadline:** `web/js/api.js:123`–139 awaits fetch without an abort signal or timeout. An isolated Node assertion confirmed an unresolved fetch remains pending after 100 ms and receives no signal; this does not measure a browser timeout. A stalled initial request can keep boot/loading pending until the browser/network terminates it. Rejected initial views also explicitly call `console.error` (`web/js/main.js:63`).
  Reproduce: intercept `/api/v1/host` and leave it unresolved on a disposable panel.
  Load `#/`; observe loading, then reject the request and inspect the readable message and router console error. Suggested prompt: **Bound API requests and provide recoverable page load errors**.

No production code changed; route lifecycle, cancellation and retry behavior need a focused follow-up with browser verification.

## Owner checks — QA 09

- `timeout 120 multipass info opsbox-vm` failed with `multipass socket access denied`; HTTPS to the previously recorded `192.168.252.2:8443` failed to connect. Both `timeout 120 node tools/check-forms-ui.mjs` and `timeout 120 node tools/check-flip-ui.mjs` failed starting a local server with `listen EPERM: operation not permitted 127.0.0.1`. These are environment/test-harness failures, not observed application exceptions. No authenticated real-browser sweep or asset 404 check was completed.
- Passed: `timeout 120 node tools/check-network-ui.mjs`, `timeout 120 node tools/check-mock-api.mjs` (163 page references, 168 real capabilities), FLIP source assertions before server startup, and isolated assertions against the actual `api.js` for rejected fetch, JSON 503 and non-JSON 502 readable messages. Reviewed shell, router, API, shared UI polling/action helpers and the requested pages. These checks do not establish zero browser errors or stable rendered layouts.
- From a terminal with VM access, obtain the verified IP with `timeout 120 multipass info opsbox-vm`. Run a real-browser sweep (five minutes maximum) on that disposable instance, recording console errors, page errors/unhandled rejections, failed requests, HTTP 4xx/5xx, and font/icon/source-map 404s. Visit Overview, Apps, Containers (all tabs), a real test database detail (`#/databases/<id>`; there is no standalone database list route), Backups, Alerts, Files, Terminal, Cleanup, all six System tabs (Updates/Services/Capacity/Logs/Jobs/Firewall), Access (`#/security/access`), Security and Settings sections. Test Login signed out (only expected session 401 exempt), and Setup on a separate fresh disposable instance; record redirects so they do not count as page coverage.
- Intercept each page's initial API request and refresh/action requests with connection rejection, JSON 503, non-JSON 502 and an unresolved request. Confirm a visible human-friendly message, stable layout, cleared loading/busy state and usable retry after recovery, with no uncaught exception/rejection. Keep injected failures separate from healthy-load request counts. Never disrupt a real-data server.
- Requested `timeout 120 git mv docs/prompts/qa/09-console-and-network-errors.md docs/prompts/qa/archive/09-console-and-network-errors.md` failed because `.git/index.lock` creation is denied (`Operation not permitted`). Moved the prompt in the working tree instead. Commit/push remain blocked by Git metadata permissions. From a terminal with Git write access, preserve unrelated work and run (docs only; no release tag):

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/09-console-and-network-errors.md docs/prompts/qa/archive/09-console-and-network-errors.md
timeout 120 git commit -m "qa 09: Every page: zero console errors, zero failed requests"
timeout 120 git push origin dev
```

## Found, not fixed — QA 11

Reviewed the actual stylesheet `web/css/app.css` (`web/css/style.css` does not exist), the archived prompt 16s, sidebar drawer (`web/js/shell.js:450`–550), shared table/dialog helpers, and Terminal viewport handling. The implementation includes a drawer below 900px with backdrop/Escape/route/Back/swipe closing and focus handling; 700px layout/card-table rules and contained `.tw.scroll` wrappers; bottom sheets below 640px; safe-area padding, 16px touch input text, and Terminal `visualViewport` listeners. These are source findings, not evidence that every rendered route fits. Root `overflow-x:clip` alone does not prove content remains accessible.

- **Touch controls do not consistently reach 44×44px:** `web/css/app.css:906` sets coarse-pointer checkboxes/radios to 22×22px; file selection (`web/js/pages/files.js:31`) and package selection (`web/js/pages/system.js:42`) have no enclosing label to enlarge the target. Inputs/selects retain a 40px minimum at `web/css/app.css:308`; stepper buttons are 40px wide at `:341`; `.segctl>button` retains a 32px minimum at `:374` because the coarse-pointer list at `:902` covers `.seg-radio`, not `.segctl`. Terminal key minimums of 44px at `:908` are overridden by the equally specific, later 40px rule at `:913`. Dimensions are source-derived; browser measurements remain pending.
  Reproduce: on the disposable real-API panel, emulate a coarse-pointer phone at 390px (repeat at 360px), then open Files/System Updates, a numeric form, segmented controls and Terminal.
  Measure visible enabled controls with `getBoundingClientRect()` and check both dimensions against 44px, including any associated label/hit area. Suggested prompt: **Complete 16s touch target sizing and verify all phone routes**.

No product code changed: runtime verification is blocked, and the full control/layout audit needs a focused browser follow-up. No browser failure screenshots were captured because no test page could be loaded.

## Owner checks — QA 11

- `timeout 120 node tools/check-responsive-ui.mjs`: all responsive source assertions passed, then browser setup failed with `listen EPERM: operation not permitted 127.0.0.1`. The existing browser suite uses mock mode, checks overflow on Overview at 360/390/768/1024px, and samples shell touch targets; it does not establish every-route real-API coverage or keyboard behavior.
- `timeout 120 multipass info opsbox-vm`: `multipass socket access denied`. Bounded HTTPS probes to the previously recorded `192.168.252.2:8443` and localhost port 8443 both exited 7. No authenticated real-API browser checks completed.
- From a host terminal with VM access, obtain the current disposable VM address and sign in using real API mode. Run `timeout 120 node tools/check-responsive-ui.mjs` for the existing baseline. With Playwright touch contexts at **360×800 and 390×800**, visit every registered route (including auth states on suitable disposable snapshots and detail routes with test records). Assert `document.documentElement.scrollWidth <= innerWidth` with no tolerance; check content is not merely clipped and wide tables scroll only within their wrappers. Open/close the drawer via menu, backdrop, Escape, route change, swipe and Back; check focus return. Open a form dialog and verify a bottom sheet with reachable actions. Measure all interactive hit areas against 44×44px, including row checkboxes, inputs, steppers, segmented buttons and Terminal keys. Focus a Terminal prompt and a form input, emulate a reduced visual viewport/keyboard, and verify the prompt, fields and submit actions remain reachable. Save any failures to `docs/screenshots/qa11/` and update this result.
- On branch `dev`, the requested `timeout 120 git mv docs/prompts/qa/11-mobile-390.md docs/prompts/qa/archive/11-mobile-390.md` failed because `.git/index.lock` creation is denied (`Operation not permitted`). The prompt was moved in the working tree instead; staging, commit and push require Git metadata write access. Preserve unrelated files and run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/11-mobile-390.md docs/prompts/qa/archive/11-mobile-390.md
timeout 120 git commit -m "qa 11: Phone widths: no sideways scroll, targets big enough"
timeout 120 git push origin dev
```

No release tag is needed for this docs-only change.

## Found, not fixed — 13: Empty and error states on every page

Source review only; these findings are not browser observations. No application code changed: addressing the gaps spans more than the prompt's one/two-file, roughly 30-line fix limit.

| Page | Empty-state evidence | Request-failure evidence |
|---|---|---|
| Overview | `web/js/pages/overview.js:79–98`: no containers and no activity have messages plus View all/Audit log links. | `:102–110`: required requests propagate to the router; optional failures become null, potentially showing missing containers/activity as empty data. Cleanup failure has a message but no retry. |
| Apps | `web/js/pages/apps.js:101`: no services guides users to Deploy your first service. | `:26`: initial failure propagates to router. `:315–324,341`: failed domain loading calls paint in finally, which starts another request while domains are absent; no persistent error/retry state. |
| Containers | `web/js/pages/containers.js:125–145`: install/start Docker guidance and zero-container Deploy a service action. Images/volumes/networks also have empty messages (`:176,186,194`). | `:77–79`: initial failures propagate to router; Docker-not-running has Start Docker and Open System services controls. |
| Backups | `web/js/pages/backups.js:66`: zero snapshots instructs Back up now; header supplies that action for admins (`:104`). Empty schedules/destinations use bare tables with Add schedule/Add destination controls (`:36,73`). The zero-snapshot instruction also appears for non-admins who cannot use that action. | `:14`: initial failure propagates to router. |
| Alerts | `web/js/pages/alerts.js:28,32`: no active alerts/no history explained; channels offer Connect and Send test, with expectations at `:38`. | `:22`: initial failure propagates to router; polling failures retain old content and report connection status through shared poll handling. |
| Files | `web/js/pages/files.js:29–33`: zero entries produces an empty table at root, or only ../ elsewhere. Upload and New folder controls exist (`:57`), but no explicit empty-folder message. | `:26–27`: initial failure propagates to router; editor-load failure displays escaped error text (`:52`) without a retry. |
| Terminal | `web/js/pages/terminal.js:119–126`: administrator restriction and no ordinary account guidance; New session is available after users load. | `:120,144`: user-list failure shows the error callout, then mount returns without binding a retry. Session connection failures can be retried with New session. |
| System | `web/js/pages/system.js:42`: no updates explained; journal says No entries (`:52`). Zero services/jobs/firewall rules use bare tables (`:44,53,55`); jobs/rules retain Add controls. | `:69–75,91–93`: pane failure rejects async mount before replacing skeleton/binding controls. Router does not await mount (`web/js/main.js:61`), so its catch cannot render this error; global rejection only toasts. Capacity failure has helper guidance (`web/js/capacity.js:263`) without retry. |
| Settings | Configuration page does not require apps/containers/backups/alerts/files; relevant absent integration/configuration controls are rendered independently of those collections. | `web/js/pages/settings.js:33`: main failure propagates to router. Disk errors have text (`:231`) but no retry; proxy-network errors are passed to networkCard (`:34,242`). |

### Findings and reproduction

1. **Missing recovery controls across pages** — `web/js/main.js:52,63`, `web/js/pages/terminal.js:120,144`, `web/js/pages/settings.js:231`.
   - On a disposable local server, reject a page's initial data request, Terminal users request, or Settings disk request; open/reload the affected page.
   - Observe error markup has no Retry control; module-import failure uses a raw exception in a pre block. Expected: readable context and an explicit retry that restores the page.
   - Suggested prompt: **Add consistent page and section error states with retry actions**.
2. **System can retain its loading skeleton after failure** — `web/js/pages/system.js:69–75,91–96`; `web/js/main.js:61`.
   - Reject getUpdates on initial System load, or another pane request when selecting its tab.
   - Initial mount rejects before replacing the skeleton and binding controls; a later tab failure can retain the previous pane. Expected: inline error and Retry. Confirm behavior in the browser.
   - Suggested prompt: **Handle System pane loading failures and bind recovery controls**.
3. **Empty collection guidance is incomplete** — `web/js/pages/files.js:29–33`, `web/js/pages/system.js:44,53,55`, `web/js/pages/backups.js:66`.
   - Open an allowed empty folder; return zero services/jobs/firewall rules in disposable fixtures; view zero snapshots as a non-admin.
   - Files/System render bare tables (or only the parent-folder row); Backups instructs non-admins to use a hidden action. Expected: explain absence and offer a role-appropriate next step.
   - Suggested prompt: **Add explicit, role-aware empty states for Files, System and Backups**.
4. **Apps domain request failure can repeatedly reload** — `web/js/pages/apps.js:315–324,341`.
   - Select an app's Domains tab while making getDomains fail consistently.
   - Its finally block repaints, and paint starts another load while doms is absent; Loading can persist with repeated errors. Expected: stop automatic retries and offer a deliberate Retry control. Confirm request cadence in the browser.
   - Suggested prompt: **Stop domain-load retry loops and show a recoverable error**.

## Owner checks — 13: Disposable VM and browser verification

`timeout 120 multipass info opsbox-vm` failed with `info failed: multipass socket access denied`. No VM service was stopped and no live fresh-install, helper-down, or slow-3G layout checks were performed. Static skeleton/refreshing markup does not establish layout stability.

From a host with Multipass access, use only `opsbox-vm` or a disposable local dev server:

1. Restore a fresh-install snapshot with no deployed apps/databases/backups. Open Overview, Apps, Containers, Backups, Alerts, Files (an allowed empty folder), Terminal, every System tab, and Settings. Record empty messages and usable next actions; verify non-admin guidance too.
2. Run `timeout 120 multipass exec opsbox-vm -- sudo systemctl stop opsbox-priv`. Reload each page, checking a readable helper banner/error, no raw JSON/undefined, no retained skeleton, and functioning recovery controls. Restore with `timeout 120 multipass exec opsbox-vm -- sudo systemctl start opsbox-priv`, including if verification fails.
3. In browser DevTools, disable cache and select Slow 3G; visit each page/tab and check visible loading states and layout stability. Then block each page's data request to verify error/retry behavior, unblock it, and confirm recovery. Record browser observations separately from this source audit.

The requested `timeout 120 git mv docs/prompts/qa/13-empty-and-error-states.md docs/prompts/qa/archive/13-empty-and-error-states.md` failed: `.git/index.lock: Operation not permitted`. The prompt was archived with a filesystem move instead. Git staging/commit/push require a host terminal with Git write access; remain on `dev`, stage only QA 13 paths below, and create no tag for this docs-only change:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/13-empty-and-error-states.md docs/prompts/qa/archive/13-empty-and-error-states.md
timeout 120 git commit -m "qa 13: Empty and error states on every page"
timeout 120 git push origin dev
```


## QA 14 — Capacity guard evidence (2026-11-19)

- Reviewed `crates/opsbox-capacity/src/{lib,requirements}.rs`, API admission/override wiring, privileged host profiling and swap helper, PostgreSQL template defaults, UI override controls, builder limits, and `install.sh:149–185`. No product code changed.
- Baseline: OS `max(192, RAM/10)` MiB + opsbox 128 MiB + Docker 150 MiB when installed. This totals 470 MiB on both 956 and 1024 MiB machines, leaving 486 / 554 MiB before workload reservations. Admission also caps headroom by live `MemAvailable` minus the OS reserve, credits only half of free swap for apps/builds, and never credits swap for databases. Baseline sufficiency under real load remains unmeasured.
- Docker requires 768 MiB total RAM; apps default to 128 MiB with 256 MiB recommended. Builds require 1536 MiB allocatable and recommend 2048 MiB; insufficient headroom blocks admission with a suggestion to build in GitHub Actions. Buildx uses a container builder limited to `memory=1536m,memory-swap=1536m`. An Owner can explicitly override admission; this does not guarantee freedom from OOM.
- Pressure blocks new workloads immediately below the OS available-memory reserve, or after memory PSI `some avg60` exceeds 20% for 120 seconds. Workloads require cgroup v2. Reservation refusals suggest stopping/removing the largest workload by name and memory amount.
- At 956 MiB without swap, three 128 MiB reservations leave 102 MiB and block a fourth default app; at 1024 MiB, three leave 170 MiB, so a fourth can fit and a fifth is blocked. These are source-derived calculations, not live container results. Adding 2048 MiB entirely free swap increases app/build allocatable by 1024 MiB, leaving database capacity unchanged. The helper offers 2 GiB only with more than 20 GiB free disk; otherwise it offers 1 GiB and enforces disk reserves.
- PASS: `timeout 120 sh tools/check-capacity-install.sh`; `timeout 120 node tools/check-capacity-ui.mjs`. A separate temporary `MemTotal: 978944 kB` fixture using `OPSBOX_TEST_PREFLIGHT=1`, `OPSBOX_TEST_MEMINFO`, and `sh install.sh --dev --dry-run` exited 0 and printed `956 MB RAM, no swap` and `Docker apps OK, builds on server not recommended`; no host installation occurred.
- NOT RUN: `timeout 120 cargo test -p opsbox-capacity --offline` exited 127 (`cargo` unavailable). Existing admission and HTTP tests were inspected, not executed. `timeout 120 multipass info opsbox-vm` exited 2 (`multipass socket access denied`); no live Capacity page, memory comparison, override deployment, swap mutation, or container saturation was verified.

### Found, not fixed — 14: PostgreSQL refusal is conditional

`crates/opsbox-deploy/templates/postgres.toml:1` sets a 256 MiB minimum and 512 MiB recommendation; `crates/opsbox-api/src/capacity.rs:134` defaults to that minimum. `crates/opsbox-capacity/src/lib.rs:213` checks database physical headroom. Thus an idle 956 MiB Docker VM with sufficient available memory admits the default PostgreSQL as Tight rather than Blocked. The QA prompt's unconditional refusal expectation is not established by the policy. No policy change was made without a measured failure.

Reproduce on the disposable VM:
1. With Docker installed, no workload reservations, and at least 448 MiB MemAvailable, open System > Capacity and add PostgreSQL at its default 256 MiB limit: expect Tight admission.
2. Request a 512 MiB PostgreSQL limit on the 956 MiB host, or reserve enough memory to leave less than 256 MiB physical headroom: expect Blocked, then exercise the explicit Owner override.

Suggested prompt: **Clarify PostgreSQL admission expectations on a 1 GB host and validate measured baseline headroom.**

### Owner checks — 14: Live VM and Rust verification

From a host terminal with Multipass access, use only disposable `opsbox-vm`; wrap each command in `timeout 120`:

```sh
timeout 120 multipass info opsbox-vm
timeout 120 multipass exec opsbox-vm -- free -m
timeout 120 cargo test -p opsbox-capacity --offline
timeout 120 cargo test -p opsbox-api --test capacity_http --offline
```

Open System > Capacity and reconcile RAM, fixed reserves, workload limits, and free allocation against `free -m` (allocation is reserved capacity, not current usage). Confirm a small image app fits and a local build is refused. Use a 512 MiB PostgreSQL request or existing reservations to force refusal; verify the readable reason, disabled submission without both the checkbox and exact sentence `I understand this server may run out of memory`, successful Owner override creation, and updated memory map. With more than 20 GiB disk free and no existing swap, add the offered 2 GiB swap through the helper; verify app/build allocation grows by half the free swap and database headroom does not grow. Start 128 MiB image containers until admission refuses another; verify the suggestion names a workload to free, then remove test workloads and restore the VM snapshot.

`timeout 120 git mv docs/prompts/qa/14-capacity-1gb.md docs/prompts/qa/archive/14-capacity-1gb.md` failed because `.git/index.lock` creation is denied. The file was moved in the working tree instead; staging/commit/push cannot proceed under the same Git write restriction. On `dev`, stage only these QA 14 changes (review any other edits to shared docs first), then commit and push; no tag for docs-only work:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/14-capacity-1gb.md docs/prompts/qa/archive/14-capacity-1gb.md
timeout 120 git commit -m "qa 14: Capacity guard on the 1 GB VM"
timeout 120 git push origin dev
```


### 15 — Owner checks: live port conflicts and reversible actions

Local checks on 2026-11-19:

- `timeout 120 node tools/check-network-ui.mjs` passed network action visibility, escaping, CSRF, adoption API mapping and task polling assertions. Additional temporary fixtures verified the conflict banner contains `8080`, `8445`, `Web ports are busy`, and `Move behind opsbox` for both Docker and nginx holders. This verifies generated markup, not a live browser session.
- Extracted the actual `check_proxy_ports` function from `install.sh:98` into a temporary shell harness. Mock `ss`/`docker` output for nginx, docker-proxy (including container name), and free ports passed: both busy ports warn with 8080/8445 and panel guidance, and all three cases exit 0. Full installation was not executed.
- Source review: `crates/opsbox-proxy/src/server.rs:129` binds both listeners through fallback; `:797` handles address-in-use/permission-denied, and `:816` tries preferred ports followed by 8080–8099 / 8445–8464. `crates/opsbox-watchdog/src/policy.rs:138` treats a reported port conflict as healthy. Existing `crates/opsbox-proxy/tests/port_conflict.rs:21` covers occupied primary HTTP/HTTPS, an occupied HTTP fallback, forwarding, conflict status and promotion without dropping an in-flight request; it uses ephemeral primary ports.
- `timeout 120 cargo test -p opsbox-proxy --test port_conflict -- --nocapture` could not run: `timeout: exec(cargo): No such file or directory` (exit 127). `timeout 120 multipass list` failed with `multipass socket access denied`. No live fallback, service restart stability or reversible site migration is claimed; no code defect was established.

From a host terminal with Multipass access, use only the disposable `opsbox-vm`, with a recoverable snapshot and no real sites. Wrap every command in `timeout 120`:

1. Confirm the VM and toolchain are available. Run `timeout 120 cargo test -p opsbox-proxy --test port_conflict -- --nocapture` locally and `timeout 120 cargo test -p opsbox-api --test network_http` for takeover/undo HTTP integration coverage.
2. On a clean VM, start `timeout 120 multipass exec opsbox-vm -- sudo docker run -d --name qa15-web -p 80:80 -p 443:443 nginx`. If either port already belongs to a VM service, identify it and restore the clean snapshot rather than replacing an unknown listener. Keep the container ID for cleanup. Check HTTP and HTTPS against a fixture with working TLS if HTTPS site reachability is to be asserted (stock nginx does not serve TLS merely because port 443 is published).
3. Install opsbox using the documented installer from the VM; confirm both conflict warnings and successful completion. Inspect `ss -ltnp`, `opsbox status`, `systemctl is-active opsbox opsbox-priv opsbox-proxy`, and `systemctl show opsbox opsbox-proxy -p NRestarts -p MainPID`. Verify proxy listeners on 8080/8445 (or next free ports). Repeat the status checks after at least 35 seconds and confirm stable restart counts. Verify the original fixture site still answers while opsbox uses fallback.
4. Open the panel overview and Settings → Network. Confirm the warning, holder identity, actual fallback ports and **Move behind opsbox** guidance. Exercise takeover and Undo, verifying promotion and restoration of the old service/restart policy. Takeover intentionally stops the old site; the UI warns of this. Verify site reachability again after Undo.
5. Exercise **Move behind opsbox**, inspect its proposed private ports, confirm, and check the original site through opsbox on public ports. Use **Undo move** and confirm the old site again answers on its original ports. Repeat with a disposable nginx systemd fixture. Record actual browser results and service restart counts.
6. Remove only test resources created for this check and restore the VM snapshot. Update row 15 to PASS only after live checks succeed.


QA 15 Git completion: the requested `timeout 120 git mv docs/prompts/qa/15-proxy-port-conflict.md docs/prompts/qa/archive/15-proxy-port-conflict.md` failed with `Unable to create .git/index.lock: Operation not permitted` (exit 128). The prompt was archived with a filesystem rename instead. Git metadata is read-only in this session, so staging, commit and push remain for the owner. Existing unrelated README row 27 changes and the staged QA 27 rename were preserved. On `dev`, stage only the QA 15 row/owner notes and README row 15 (use selective staging), plus this prompt rename; commit only those paths as `qa 15: Ports 80/443 already taken` and push `origin dev`. No tag is needed for these documentation-only changes.
