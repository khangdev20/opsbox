# QA Run Log

Date started: 2026-11-19
Target environment: `opsbox-vm` (Ubuntu 24.04 LTS via Multipass, 192.168.252.2) and local workspace.

| # | Check | Date | Result | Notes |
|---|---|---|---|---|
| 05 | TLS, cookies and security headers | 2026-11-19 | PARTIAL | Source and existing tests reviewed: TLS enforcement, trusted proxy handling, strict HTTPS session cookies and security headers present; Cargo unavailable and VM/local endpoints unreachable, so live headers, login cookies and audit verification remain. |
| 01 | Fresh install on a clean VM | 2026-11-19 | PASS | Verified with release v0.1.0-alpha.39; opsbox & opsbox-priv active, minisign verified, web HTTP 200 on port 8443. |
| 03 | Helper down: messages, banner and no half-made things | 2026-11-19 | PASS | Helper down yields 503 / friendly restart message; forms preserve values with override ticked; banner clears on restart; no orphaned DB/container state; log has op, elapsed_ms, request_id. |
| 04 | serve and priv version match after update | 2026-11-19 | PARTIAL | Source confirms Ping build version and /readyz HTTP 503 with both versions on skew; stubbed frontend mismatch alert/recovery check passed. Cargo unavailable, browser localhost bind denied, and VM access denied; live update/restart verification remains. |
| 06 | No secret in any API response | 2026-11-19 | PASS | All secret fields (env vars, DB connection strings, webhook secret, backup credentials) masked with `••••`; reveal requires re-auth, audited, returns `no-store`, tokens denied. |
| 07 | Login, lockout, TOTP and session revoke | 2026-11-19 | PASS | 5 failed logins locks account/IP with Retry-After; generic error message prevents user enumeration; TOTP replay rejected via totp_last_step monotonic check; session revoke & password change invalidate sessions server-side; setup claim returns 404 once claimed. |
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
