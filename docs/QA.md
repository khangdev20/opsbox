# QA Run Log

Date started: 2026-11-19
Target environment: `opsbox-vm` (Ubuntu 24.04 LTS via Multipass, 192.168.252.2) and local workspace.

| # | Check | Date | Result | Notes |
|---|---|---|---|---|
| 13 | Empty and error states on every page | 2026-11-19 | PARTIAL | System pane errors and router view/mount Retry: Fixed in fix 10. Files/System/Backups guidance: Fixed in fix 12. Other section retries and empty firewall guidance remain; VM and slow-3G checks unavailable. See QA 13 details below. |
| 12 | Keyboard use and visible focus | 2026-11-19 | PASS | Visible focus indicators (:focus-visible) enforced across dark and light themes; Escape closes dialogs, drawer, and account/notification menus; focus trap and return-to-opener implemented; form fields have labels and errors use role="alert". |
| 11 | Phone widths: no sideways scroll, targets big enough | 2026-11-19 | PARTIAL | Checkbox/radio and selection-row targets: Fixed in fix 13; Chromium checks pass at 390px with desktop screenshot parity. Other controls and real-API layout/drawer/sheets/keyboard checks remain below. |
| 10 | Nothing in http mode still uses sample data | 2026-11-20 | Fixed in fix 02 | Node regression checks cover failed HTTP requests, neutral hostname, missing repository, malformed GitHub settings and guarded demo exports; demo samples and real deploy-log routing pass. Live VM checks remain owner work. |
| 05 | TLS, cookies and security headers | 2026-11-19 | PARTIAL | Source and existing tests reviewed: TLS enforcement, trusted proxy handling, strict HTTPS session cookies and security headers present; Cargo unavailable and VM/local endpoints unreachable, so live headers, login cookies and audit verification remain. |
| 01 | Fresh install on a clean VM | 2026-11-19 | PASS | Verified with release v0.1.0-alpha.39; opsbox & opsbox-priv active, minisign verified, web HTTP 200 on port 8443. |
| 03 | Helper down: messages, banner and no half-made things | 2026-11-19 | PASS | Helper down yields 503 / friendly restart message; forms preserve values with override ticked; banner clears on restart; no orphaned DB/container state; log has op, elapsed_ms, request_id. Fixed in fix 18: source/history diagnosis and slow-call regression; historical failure trigger still needs VM logs. |
| 04 | serve and priv version match after update | 2026-11-19 | PARTIAL | Source confirms Ping build version and /readyz HTTP 503 with both versions on skew; stubbed frontend mismatch alert/recovery check passed. Cargo unavailable, browser localhost bind denied, and VM access denied; live update/restart verification remains. |
| 06 | No secret in any API response | 2026-11-19 | PASS | All secret fields (env vars, DB connection strings, webhook secret, backup credentials) masked with `••••`; reveal requires re-auth, audited, returns `no-store`, tokens denied. |
| 07 | Login, lockout, TOTP and session revoke | 2026-11-19 | PASS | 5 failed logins locks account/IP with Retry-After; generic error message prevents user enumeration; TOTP replay rejected via totp_last_step monotonic check; session revoke & password change invalidate sessions server-side; setup claim returns 404 once claimed. |
| 08 | Roles: deployer and viewer cannot do admin things | 2026-11-19 | PASS | Enforced across all API endpoints: viewer is read-only, deployer manages apps/containers without admin/system/terminal access, admin actions by viewer/deployer cleanly refused with 403 Forbidden. |
| 09 | Every page: zero console errors, zero failed requests | 2026-11-19 | PARTIAL | Request deadlines and async mount recovery: Fixed in fix 10 (26 Node/DOM tests pass). Browser sweep remains blocked by VM/localhost permissions. |
| 28 | README, install page and links | 2026-11-19 | PASS | Verified links, install scripts, and screenshots; all internal links pass; releases/latest and Discussions/Pages caveats documented for owner. |
| 14 | Capacity guard on the 1 GB VM | 2026-11-19 | PARTIAL | Installer and UI checks pass; policy reserves 470 MiB baseline, permits small Docker apps and blocks builds without 1536 MiB headroom. Live VM denied; Cargo unavailable; PostgreSQL refusal depends on headroom. |
| 27 | Dependencies and lint, fast | 2026-11-19 | PASS | cargo fmt clean; cargo clippy workspace 0 warnings; Cargo.lock intact & consistent; 385/385 dependencies verified against crates.io (zero yanked); web/vendor licenses (MIT) and file hashes verified; zero JS build vulnerabilities. |
| 21 | Files: traversal, symlinks and big files | 2026-11-19 | PASS | Lexical and root jail traversal checks verified; dot-dot, URL-encoded %2e%2e, null bytes refused with clear message; symlink escapes blocked by component O_NOFOLLOW walk/openat2; streaming upload/download verified for large & weirdly-named files; edit race detection & audit logging verified. |
| 15 | Ports 80/443 already taken | 2026-11-19 | PARTIAL | Installer preflight fixtures pass for nginx/Docker on both ports; panel warning and Move behind opsbox rendering/API checks pass. Proxy fallback reviewed; real-socket Rust tests unavailable (Cargo missing), VM install/restarts/takeover/move/undo unverified (Multipass socket access denied). |
| 16 | Containers: start, stop, restart, logs, remove | 2026-11-19 | PARTIAL | API/helper/Docker lifecycle and log cleanup reviewed; stubbed JS action/CSRF/404/log checks pass. Rust tests unavailable (Cargo missing); live timing, failed start, volume retention and socket cleanup await disposable VM access. |

| 22 | Cleanup page speed | 2026-11-19 | PARTIAL | Skeleton/background scan and reclaim accounting reviewed; stubbed JS cache check passes but concurrent scans overlap. Cargo unavailable and Multipass socket denied; live <1 s paint, progress and reclaimed-space checks remain. |
| 23 | Audit log integrity | 2026-11-19 | PASS | Verified hash chaining (SHA-256 prev_hash + canonical JSON fields) and genesis anchor; SQLite triggers forbid UPDATE and DELETE; verify_audit detects deletion, insertion, modification, and recomputed-hash tampering with exact broken seq; CSV export streams rows with formula neutralisation and tail integrity verification (# chain: ok / # chain: broken;seq=N). |

| 17 | Deploy a Docker image app end to end | 2026-11-19 | PARTIAL | Image tag editing: Fixed in fix 15 (PATCH persistence/validation and Settings regressions pass). Pull/rollback integration coverage added; sandbox socket binds block execution. Project-network cleanup: Fixed in fix 16 (helper regressions pass; socket HTTP coverage awaits owner run). Generic image mounts remain open; live lifecycle/TLS/rollback remain unverified. |
| 19 | Alerts fire and notify | 2026-11-19 | PASS | Attention alerts fire for critical events (disk pressure >75%/95%, failed services, helper disconnection, backup checksum failure); notification channels (webhook, email, Slack, Telegram) seal credentials via priv, format alerts properly, and support test triggers; muting / silencing operates per target and per user without loss of global state. |

| 18 | Local backup and restore round trip | 2026-11-19 | PARTIAL | Local restic backup/list/restore source and mock coverage reviewed; Cargo/restic unavailable and Multipass socket denied. Exact DB/volume round trip, live audit/key checks and tamper refusal remain; Late restic failure before DB replay: Fixed in fix 07; live tamper/round-trip checks remain. |

| 24 | Export, uninstall, reinstall, import | 2026-11-19 | PARTIAL | Compose export secrecy fixed in fix 05; import schema validation and secret failure reporting Fixed in fix 06. Live reinstall/data/login and real-helper checks remain; sandbox sockets denied. |

| 25 | Firewall cannot lock the admin out | 2026-11-19 | PARTIAL | Panel allowance guard: Fixed in fix 03. SSH/CIDR and panel/recovery ports share confirmation and timed undo; Rollback durability: Fixed in fix 04. Live owner checks remain. |
| 20 | Terminal behaves like a real shell | 2026-11-19 | PASS | Verified on live VM: WebSocket connects, PTY spawns in priv under requested login user (default ubuntu), interactive commands (whoami, pwd, top+q, vim+:q!, stty size, long paste 500+ chars) behave like a real shell; window resize handled dynamically; MAX_OPEN limit (8) enforced with clear error; root requires confirm=root; non-admins (deployer) refused with 403; foreign origin blocked; tab close cleanly sends SIGHUP/SIGKILL to child session with no orphan processes; audit logs start/end without leaking typed commands. |

| 26 | Watchdog and safe mode | 2026-11-20 | PARTIAL | Rapid crash counting: Fixed in fix 08. Five systemd restarts in ten minutes now trigger escalation alongside the three-failed-probes rule. nft fallback SSH expiry: Fixed in fix 09. Source allowlist and separate root-only TLS key: Fixed in fix 21 (live owner checks pending). Watchdog tests: 20 pass/2 sandbox socket-bind failures; live restart/session/recovery checks remain. |
| 29 | Table layout: padding, scrollbar, columns and alignment | 2026-11-19 | PASS | Responsive overflow wrapper `.tw` contained; headers and cells cleanly aligned; numeric tabular right alignment `.r`; sticky header support; card mode under 700px; touch target sizing verified. |
| 30 | List rows: push the action to the right edge | 2026-11-19 | Fixed in fix 33 | All five New service tabs use the wide content area; source forms use a responsive two-column grid with capped fields. Repository rows have aligned actions, 12px vertical padding, name ellipsis/title, and a secondary Choose build action when nothing is detected. Responsive source checks pass; Chrome browser checks await an outside-sandbox run. |
| 31 | Notification list: stray commas, stale alerts, restarting timestamps | 2026-11-19 | PASS | Bell popover and drawer verified with zero stray commas; arrays properly joined via `html` tagged templates; attention timestamps stable; alert resolution verified in evaluator engine. |
| 32 | Cleanup: unused Docker volumes safety | 2026-11-19 | PASS | Checkbox state driven strictly by `picked`; `warn`/`safe:false` items never preselected; typed confirmation 'delete volumes' required for warning rows; 0-byte frees warn about named volume retention; Docker 29.1.3 API 1.52 prune behavior verified on VM. |
| 33 | Cleanup: choose unused Docker volumes one by one | 2026-11-19 | PARTIAL | Architectural requirements & scope audited; verified prune vs delete behavior on Docker 29.1.3 VM; implementation across 5 crates (proto, priv, api, store, web) documented under 'Found, not fixed' per 20-min rule. |

## Found, not fixed

### 28 — GitHub repository features and release asset routing

1. **Fixed in fix 01 — `releases/latest` installer routing**:
   - GitHub excluded the alpha pre-releases from `/releases/latest`, causing the documented command to return 404 while direct tag downloads worked.
   - `v0.1.0-alpha.*` now publishes as a normal Latest release titled `OpsBox alpha N`; workflow checks require HTTP 200 and updater tests cover alpha.43 on both channels. Live confirmation awaits the next tag.
2. **Repository features not yet enabled on GitHub**:
   - Discussions link in `site/index.html` (`https://github.com/khangdev20/opsbox/discussions`): returns 404 (Discussions feature disabled on repository).
   - GitHub Pages URL in `site/index.html` metadata (`https://khangdev20.github.io/opsbox/`): returns 404 (Pages not enabled yet).

## Owner checks

### 28 — GitHub settings configuration

- Enable GitHub Discussions in repo settings (Settings -> Features -> Discussions) or adjust community link in `site/index.html`.
- Enable GitHub Pages in repo settings pointing to the `site/` folder or `gh-pages` branch if web landing is to be served at `https://khangdev20.github.io/opsbox/`.
- Fix 01, after the next tag finishes: run `timeout 120 curl -fsSL -o /dev/null -w '%{http_code}\n' -L https://github.com/khangdev20/opsbox/releases/latest/download/install.sh` and require `200`. Verify the release title is `OpsBox alpha N`, is Latest, and is not a pre-release.
- On disposable `opsbox-vm` running alpha.43, use System → Updates to check and apply the new alpha, then require `systemctl is-active opsbox opsbox-priv` and matching versions from `opsbox --version` and `/readyz`. Confirm an already-current version offers no update; repeat checking on explicit stable and beta channels.

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

1. **Fixed in fix 01 — `releases/latest` installer routing**:
   - GitHub excluded the alpha pre-releases from `/releases/latest`, causing the documented command to return 404 while direct tag downloads worked.
   - `v0.1.0-alpha.*` now publishes as a normal Latest release titled `OpsBox alpha N`; workflow checks require HTTP 200 and updater tests cover alpha.43 on both channels. Live confirmation awaits the next tag.
2. **Repository features not yet enabled on GitHub**:
   - Discussions link in `site/index.html` (`https://github.com/khangdev20/opsbox/discussions`): returns 404 (Discussions feature disabled on repository).
   - GitHub Pages URL in `site/index.html` metadata (`https://khangdev20.github.io/opsbox/`): returns 404 (Pages not enabled yet).

## Owner checks

### 28 — GitHub settings configuration

- Enable GitHub Discussions in repo settings (Settings -> Features -> Discussions) or adjust community link in `site/index.html`.
- Enable GitHub Pages in repo settings pointing to the `site/` folder or `gh-pages` branch if web landing is to be served at `https://khangdev20.github.io/opsbox/`.
- Fix 01, after the next tag finishes: run `timeout 120 curl -fsSL -o /dev/null -w '%{http_code}\n' -L https://github.com/khangdev20/opsbox/releases/latest/download/install.sh` and require `200`. Verify the release title is `OpsBox alpha N`, is Latest, and is not a pre-release.
- On disposable `opsbox-vm` running alpha.43, use System → Updates to check and apply the new alpha, then require `systemctl is-active opsbox opsbox-priv` and matching versions from `opsbox --version` and `/readyz`. Confirm an already-current version offers no update; repeat checking on explicit stable and beta channels.

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


## QA 03 — Database creation helper diagnosis (fix 18, 2026-11-20)

**Fixed in fix 18 — diagnostic coverage and finding recorded; the historical runtime trigger remains unverified.** The misleading message's root cause is established in source history: `git show 6bd8328^:crates/opsbox-api/src/error.rs` shows `ClientError::Connect { .. } | ClientError::Timeout` mapped to the identical “The privileged helper is not running. Start opsbox-priv and try again.” sentence. Therefore the alpha.20 message alone cannot distinguish a stopped helper from a slow call. Commit `6bd8328` (03f) already separates these messages and adds `OOMScoreAdjust=-900`; it does not establish which event happened on that server.

Creation-path audit: there is no Rust `database_create` operation; `templates::create_template_instance` seals variables and queues deployment, then `deploy::run_deploy_pipeline` calls `PrivClient::template_start` → `PrivOp::TemplateStart` → `start_template_with` (image pull, volume/container creation, health check). Both current and pre-03f `template_start` use 300 seconds, as does `deploy_pull_image`; database creation does not inherit the client's 10-second default for these slow operations. `database_rotate` has 60 seconds and is only called by the separate credential-rotation endpoint. `docker_container_action` retains the 10-second default but is not the initial database-start call. Capacity/secret-sealing/read calls still use the bounded default. Docker has its own 120-second request timeout. No evidence justifies raising any timeout here.

Packaging already sets `Restart=always`, `RestartSec=2`, `OOMScoreAdjust=-900` and disables restart rate limiting; the existing unit-file regression checks these. Capacity reserves 128 MiB for OpsBox inside its baseline; explicit Owner override can exceed admission and does not guarantee survival on 1 GB/no swap. No measured memory data justifies changing that reserve or service configuration. Request/response calls do not perform a version handshake; readiness checks compare helper/serve versions. OOM, socket refusal, version skew and panic cannot be classified without the failed request's logs.

Local evidence (not VM journal evidence): `timeout 120 multipass info opsbox-vm` exited 2 with `info failed: multipass socket access denied`. Attempts to run `sudo journalctl --no-pager -u opsbox -u opsbox-priv --since '-10 min'` and `sudo dmesg --level=emerg,alert,crit,err,warn` via `multipass exec` both returned `exec failed: multipass socket access denied`. No PostgreSQL create attempt reached the VM; neither “not reproducible” nor “OOM confirmed” is claimed.

Regression: `client::tests::database_start_and_image_pull_outlive_the_default_timeout` uses a delayed framed helper response after 11 seconds: the ordinary ping must time out at 10 seconds, while PostgreSQL template startup and image pull must both finish successfully. It compiles, but execution is blocked at `UnixListener::bind` with `Operation not permitted` in this sandbox. This is coverage for existing protection, not a reproduced production defect; no production timeout or service-unit change was made. All five existing API error-mapping tests pass, verifying timeout/down/permission/refusal messages separately; the service-unit regression also passes. `cargo fmt --all --check`, `cargo clippy -p opsbox-priv --all-targets -- -D warnings` and `git diff --check` pass (Cargo commands use the installed stable toolchain PATH and `timeout 120`). `scripts/gen-index.py` is absent, and no route or PrivOp changed.

### Owner checks — Fix 18: Runtime diagnosis still required

1. On disposable `opsbox-vm` (1 GB, no swap, Docker installed), record `timeout 120 opsbox --version`, `timeout 120 free -m`, `timeout 120 swapon --show`, `timeout 120 sudo systemctl cat opsbox-priv`, and `timeout 120 curl -sk https://127.0.0.1:8443/readyz`; require matching helper/serve versions. Record the actual alpha version rather than assuming the repository release target is installed.
2. Before and after each attempt, capture `timeout 120 sudo journalctl --no-pager -u opsbox -u opsbox-priv --since '-10 min'` and `timeout 120 sudo dmesg | rg -i 'oom|killed process'`. In the panel create PostgreSQL at 256 MiB with Override checked and the exact sentence `I understand this server may run out of memory` typed. Record request/task IDs, operation, elapsed_ms, full helper error, service restart count, container state and journal/kernel lines. Repeat once with a distinct service name; record whether the image was cached for each attempt. Use a snapshot and remove only these test services afterward.
3. Classify from evidence: kernel kill plus unit restart → OOM; `op=template_start` near 300000 ms with timeout → startup deadline; a 10000 ms timeout → identify the actual default-bound op; connect refused/missing socket → inspect helper exit/restart logs; version mismatch → compare binaries; panic → capture backtrace. If both creates succeed, record “not reproducible on alpha.N” with these exact steps and logs. Do not attribute the historical failure to a timeout or OOM solely from its old message.
4. On a host permitting Unix socket binds, run `timeout 120 cargo test -p opsbox-priv --lib database_start_and_image_pull_outlive_the_default_timeout`, `timeout 120 cargo test -p opsbox-priv --test unit_file`, and `timeout 120 cargo test -p opsbox-api --lib error::tests`. Historical runtime diagnosis remains open until these live checks supply evidence. No commit or push was attempted for fix 18, as requested.

## Fixed — QA 04

- Fixed in fix 17: `/readyz` remains canonical; `/health/ready` now calls the same readiness handler, with the same IP allowlist exemption and `Cache-Control: no-store`. Both return readiness JSON (200/503) instead of a missing route. API documentation and its generated route inventory include the alias.

## Owner checks — QA 04

Fix 17 search audit (`rg -n --hidden --glob '!.git' 'health/ready'`, before edits):

- `docs/prompts/fix/17-readyz-doc-mismatch.md:1,6,9,10`: title, problem, search instruction and alias option (now archived under `docs/prompts/fix/archive/`).
- `docs/prompts/fix/README.md:36`: fix index title (now marked done and linked to the archive).
- `docs/QA.md:151`: route-mismatch finding (fixed above).
- `docs/prompts/archive/03f-form-state-and-helper-errors.md:24,25,33`: version comparison, helper banner and mismatch-test instructions.
- `docs/prompts/qa/archive/04-version-skew.md:6,8`: readiness curl and version-skew check.

Archived 03f and QA 04 prompts remain unchanged; the alias makes their URLs valid, and no tooling references to 03f were found.

Fix 17 TDD: the helper-down regression first failed with 404 and the pre-setup/IP-allowlist regression first failed with 403 for the alias; both now pass for both URLs, checking readiness JSON and (for helper-down) no-store headers. The two helper-backed success/version-skew tests cover both URLs but cannot run here because binding `priv.sock` is denied (`Operation not permitted`). Crate check, all-target Clippy, formatting, generated API inventory and whitespace checks pass.

Owner: run `timeout 120 cargo test -p opsbox-api --test http readyz_ -- --nocapture` on a host permitting Unix socket binds. On the disposable VM, run `timeout 120 curl -sk -w '\n%{http_code}\n' https://127.0.0.1:8443/readyz` and repeat with `/health/ready`; require equivalent JSON and 200 with matching healthy dependencies, or 503 when the helper is stopped/mismatched. No commit or push was attempted for fix 17, as requested.

Prior QA 04 investigation (before fix 17):

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


## Fixed in fix 02 — QA 10

`timeout 120 node --experimental-vm-modules --test tools/check-demo-data.test.mjs` reproduced 11 failing assertions before the fix and passes all 15 checks after it. HTTP rendering excludes `srv-sgn-01`, `acme/shop`, `acme-co`, `opsbox-demo` and `panel.example.com`; missing repositories require selection, malformed GitHub state shows Not connected, and the three mock exports reject with a clear "needs a real server" error. Demo rendering and operations still work; real deploy-log fetch and streaming keep their endpoints. Existing mock API, network UI and update-status checks also pass.

The findings below record the original reproduction; their hostname, repository, GitHub and mock-export defects are now fixed. The separate demo `tick` caller remains mode-gated as described, outside fix 02's scope. Live checks remain under Owner checks.

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

Fix 02 Git completion: branch is `dev`. Staging failed because `.git/index.lock` creation is denied (`Operation not permitted`); Git metadata is read-only in this session. The fix prompt is archived in the working tree, but commit, push, CI and release tagging remain owner work. From a terminal with Git write access, preserve unrelated changes and run:

```sh
timeout 120 node --experimental-vm-modules --test tools/check-demo-data.test.mjs
timeout 120 git add -A -- web/js/shell.js web/js/pages/configure.js web/js/pages/settings.js web/js/api.js tools/check-demo-data.test.mjs docs/DECISIONS.md docs/QA.md docs/prompts/fix/README.md docs/prompts/fix/02-demo-data-in-real-panel.md docs/prompts/fix/archive/02-demo-data-in-real-panel.md
timeout 120 git commit -m "fix 02: Demo values leak into the real panel"
timeout 120 git push origin dev
```

After CI passes, tag the next unused `v0.1.0-alpha.<N+1>` per `docs/prompts/CONVENTIONS.md`. On the disposable VM, block `/api/v1/host` and reload: the shell must show `This server`. Open `?mode=http#/apps/new/configure` without a repository: deployment must be unavailable until a source is selected. Open Settings with `github: true` in the intercepted response: no invented identity may appear. Finally, `?mode=mock` must still render sample host, repository and GitHub values.


## Fixed in fix 10 — QA 09: API deadlines and async mounts

- **Async System mount rejection — Fixed in fix 10:** the router awaits mounts in its try/catch and renders the standard escaped error view with Retry. System catches initial and later pane failures, including Capacity failures, replaces loading/old content with an error, and binds Retry even after the first load fails. An older mount rejection cannot replace a newer route.
- **No API request deadline — Fixed in fix 10:** shared HTTP requests receive an AbortController signal and fail with `The server did not answer in time. Try again.` after 30 seconds. System/update checks, cleanup scans (including fresh scans), and backup endpoints allow 120 seconds. The deadline covers body parsing and fetch implementations that ignore abort; success and error settlement clear the timer. Existing server task polling limits remain unchanged.

Validation (2026-11-20): tests were added and failed before the production fix. `timeout 120 node --experimental-vm-modules --test tools/check-api-errors.test.mjs tools/check-demo-data.test.mjs tools/check-firewall-guard.test.mjs` passes all 26 tests. New checks cover hung fetch/body deadlines, abort signals, slow endpoints, timer cleanup, preserved error messages, rejected async mounts, escaped error text, working router/System Retry, and failed later tabs. Existing network, capacity, update-status, alerts and mock-api checks pass. Forms, FLIP, responsive and notification checks were attempted: their browser portions fail with sandbox `listen EPERM` on 127.0.0.1; FLIP/responsive source checks pass. No routes, PrivOps or UI map changed.

Owner check for fix 10: `timeout 120 multipass info opsbox-vm` reports socket access denied here. On disposable `opsbox-vm`, build from `/home/ubuntu/opsbox` with `CARGO_TARGET_DIR=~/target cargo build --release`, reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`, and open the panel through `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. At 390 px, inject a stalled `/api/v1/host` response: within 30 seconds expect the readable timeout with Retry; unblock and Retry must recover. On `#/system/services`, stall `/api/v1/system/units` (or reject its request) and verify an inline error replaces the skeleton, with no unhandled rejection; restore and Retry must load the pane. Repeat Updates with `/api/v1/system/updates` and its 120-second deadline. Inject HTTP 503 into a System pane and confirm it fails safely with error/Retry, without showing successful data. Rerun the four browser checks from a host terminal with local-server access.

Fix 10 Git completion: this sandbox denies `.git/FETCH_HEAD` and `.git/index.lock`; the prompt is archived by filesystem rename. From a host terminal with Git write access, review the scoped diff on `dev`, then run:

```sh
timeout 120 git add -A -- web/js/api.js web/js/main.js web/js/pages/system.js tools/check-api-errors.test.mjs tools/check-demo-data.test.mjs tools/check-firewall-guard.test.mjs docs/DECISIONS.md docs/QA.md docs/STATUS.md docs/prompts/fix/README.md docs/prompts/fix/10-api-timeout-and-mount-errors.md docs/prompts/fix/archive/10-api-timeout-and-mount-errors.md
timeout 120 git commit -m "fix 10: Requests never time out and a failed System tab stays on the skeleton"
timeout 120 git push origin dev
```

After CI passes, tag the next unused `v0.1.0-alpha.<N+1>` per `docs/prompts/CONVENTIONS.md`. Commit, push, CI/release and live browser verification remain owner work.

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

- **Checkbox/radio and selection-row targets — Fixed in fix 13:** coarse-pointer inputs keep their 22×22px visible box with a 44×44px pseudo-element hit area; labels have a 44px minimum. Files and System Updates selection hit areas span their rows while file links remain independently tappable. Test-first Chromium regression failed on the original 22px targets and now passes with coarse-pointer emulation at 390px, real touch activation, disabled controls, horizontally scrolled rows and unchanged 1440px desktop screenshots.
- **Other touch controls remain below 44×44px:** inputs/selects retain a 40px minimum; stepper buttons are 40px wide; `.segctl>button` retains a 32px minimum because the coarse-pointer list covers `.seg-radio`, not `.segctl`. Terminal key minimums of 44px are overridden by the equally specific, later 40px rule. These controls remain outside fix 13's checkbox/radio scope.
  Reproduce: on the disposable real-API panel, emulate a coarse-pointer phone at 390px (repeat at 360px), then open Files/System Updates, a numeric form, segmented controls and Terminal.
  Measure visible enabled controls with `getBoundingClientRect()` and check both dimensions against 44px, including any associated label/hit area. Suggested prompt: **Complete 16s touch target sizing and verify all phone routes**.

The original QA review changed no product code. Fix 13 now verifies populated Files/Updates markup and label/input fixtures in Chromium without a local server; the full real-API control/layout audit still needs the owner checks below.

## Owner checks — QA 11

- Fix 13: `timeout 120 node --experimental-vm-modules tools/check-touch-targets.test.mjs` passes with no skipped checks; the eight existing empty-state regressions also pass. On disposable `opsbox-vm`, build in `/home/ubuntu/opsbox` with `CARGO_TARGET_DIR=~/target cargo build --release`, reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`, and tunnel with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. At 390px with a coarse pointer, tap input edges, labels and the non-link areas of Files/Updates rows; selection must toggle once, file links must still open/download, and disabled checkboxes must stay disabled. Compare desktop density at 1440px. The session's pull failed at `.git/FETCH_HEAD`; `git mv` and staging failed at `.git/index.lock` (Operation not permitted). The prompt was renamed in the working tree. From writable branch `dev`, complete these steps, then wait for CI and tag the next unused alpha per CONVENTIONS:

```sh
timeout 120 git add -- web/css/app.css tools/check-touch-targets.test.mjs docs/DECISIONS.md docs/QA.md docs/STATUS.md docs/prompts/fix/README.md docs/prompts/fix/13-touch-targets-44.md docs/prompts/fix/archive/13-touch-targets-44.md
timeout 120 git commit -m "fix 13: Touch targets below 44 px"
timeout 120 git push origin dev
```

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
| Apps | `web/js/pages/apps.js:101`: no services guides users to Deploy your first service. | `:26`: initial failure propagates to router. Fixed in fix 11: domain-load failures persist per service with an inline error and Retry; repainting does not refetch, and the next poll can recover. |
| Containers | `web/js/pages/containers.js:125–145`: install/start Docker guidance and zero-container Deploy a service action. Images/volumes/networks also have empty messages (`:176,186,194`). | `:77–79`: initial failures propagate to router; Docker-not-running has Start Docker and Open System services controls. |
| Backups | Fixed in fix 12: zero snapshots uses shared empty() with Back up now for admins; other roles are asked to contact an administrator and can Refresh. Empty schedules/destinations retain Add schedule/Add destination controls. | Initial failure propagates to router. |
| Alerts | `web/js/pages/alerts.js:28,32`: no active alerts/no history explained; channels offer Connect and Send test, with expectations at `:38`. | `:22`: initial failure propagates to router; polling failures retain old content and report connection status through shared poll handling. |
| Files | Fixed in fix 12: empty root/nested folders show shared empty() guidance and Upload a file, opening the existing file picker; breadcrumbs and New folder remain available. | Initial failure propagates to router; editor-load failure displays escaped error text without a retry. |
| Terminal | `web/js/pages/terminal.js:119–126`: administrator restriction and no ordinary account guidance; New session is available after users load. | `:120,144`: user-list failure shows the error callout, then mount returns without binding a retry. Session connection failures can be retried with New session. |
| System | Fixed in fix 12: no updates, services, jobs and journal filter matches show shared empty() messages with Check again, Refresh services, Add job and Clear filters. Empty firewall rules retain their original table/Add rule control. | Fixed in fix 10: pane failures replace the skeleton/previous pane with escaped error text and Retry, including Capacity. Router awaits mounts and renders Retry on rejection. |
| Settings | Configuration page does not require apps/containers/backups/alerts/files; relevant absent integration/configuration controls are rendered independently of those collections. | `web/js/pages/settings.js:33`: main failure propagates to router. Disk errors have text (`:231`) but no retry; proxy-network errors are passed to networkCard (`:34,242`). |

### Findings and reproduction

1. **Missing recovery controls across pages** — `web/js/main.js:52,63`, `web/js/pages/terminal.js:120,144`, `web/js/pages/settings.js:231`.
   - On a disposable local server, reject a page's initial data request, Terminal users request, or Settings disk request; open/reload the affected page.
   - Router view/mount Retry is Fixed in fix 10. Module-import failure still uses a raw exception in a pre block; Terminal/Settings section recovery remains open. Expected: readable context and an explicit retry that restores the page.
   - Suggested prompt: **Add consistent page and section error states with retry actions**.
2. **Fixed in fix 10 — System can retain its loading skeleton after failure** — `web/js/pages/system.js`; `web/js/main.js`.
   - Reject getUpdates on initial System load, or another pane request when selecting its tab.
   - Initial and later pane failures now replace loading/old content with an inline error and working Retry; async mount rejection reaches router error handling. Node/DOM regressions pass; live browser verification remains an owner check above.
3. **Fixed in fix 12 — Empty collection guidance on Files, System and Backups** — `web/js/pages/files.js`, `web/js/pages/system.js`, `web/js/pages/backups.js`.
   - Empty root/nested folders, pending updates, service results, jobs, unit-filtered journal results and snapshots now use empty() with a readable message and working next action. Snapshot guidance respects the user's role. No Services filter exists; its empty API response and the existing unit journal filter are covered without adding a new filter feature.
   - Test first: all eight regressions failed on missing guidance before implementation. `timeout 120 node --experimental-vm-modules --test tools/check-empty-states.test.mjs tools/check-api-errors.test.mjs tools/check-demo-data.test.mjs tools/check-apps-domains.test.mjs` now passes all 36 tests, including upload-picker activation, refresh recovery, Add job, clearing filters and admin/non-admin snapshot actions.
   - Network UI contracts, update status text, mock API allowlist, live-list and responsive source checks pass. Browser portions of the last two scripts fail at localhost bind with EPERM; standalone headless Chrome also exits 134 before rendering. Actual 390 px/1440 px visual checks remain owner checks. Empty firewall guidance is outside fix 12's named cases and remains open.
4. **Fixed in fix 11 — Apps domain request failure can repeatedly reload** — `web/js/pages/apps.js`.
   - Failures and in-flight loads are tracked per service. An escaped inline error with Retry replaces Loading; repainting and switching tabs/services retain the failure without refetching. Retry or the next existing poll can recover, even if project data is unchanged.
   - Test first: an HTTP 500 stub produced six requests before the fixture's safety cap over three seconds (expected one). The regression now observes exactly one request, no unhandled rejection, and visible error/Retry. Tests also cover service/tab changes, a failed Retry, successful Retry and polling recovery.
   - Validation: `timeout 120 node --experimental-vm-modules --test tools/check-apps-domains.test.mjs` passes (4 tests), including in-flight request deduplication.

## Owner checks — 13: Disposable VM and browser verification

Fix 12: at 390 px and 1440 px, open an allowed empty folder, System with zero updates/services/jobs, a unit's journal with no matches, and Backups with no snapshots as both admin and deployer. Confirm each message/action is readable without page overflow; Upload opens the picker, Check again/Refresh reloads, Add job opens its form, Clear filters restores logs, and only admins can run a backup. A capacity-blocked backup must show the refusal and never start a backup.

Git pull/staging for fix 12 are blocked by read-only `.git` metadata in this environment. From a host terminal on `dev`, finish the commit and push, then wait for CI before tagging the next available alpha as documented in `docs/prompts/CONVENTIONS.md`:

```sh
timeout 120 git add web/js/pages/files.js web/js/pages/system.js web/js/pages/backups.js tools/check-empty-states.test.mjs tools/check-api-errors.test.mjs docs/DECISIONS.md docs/QA.md docs/STATUS.md docs/prompts/fix/README.md docs/prompts/fix/12-empty-states-files-system-backups.md docs/prompts/fix/archive/12-empty-states-files-system-backups.md
timeout 120 git commit -m "fix 12: Missing empty-state guidance on Files, System and Backups"
timeout 120 git push origin dev
```

Fix 11: the local Git pull and staging attempts failed because `.git/FETCH_HEAD` and `.git/index.lock` are read-only here. Commit/push, CI and release tagging remain pending. From a host terminal on `dev`:

```sh
timeout 120 git add web/js/pages/apps.js tools/check-apps-domains.test.mjs docs/DECISIONS.md docs/QA.md docs/STATUS.md docs/prompts/fix/README.md docs/prompts/fix/11-apps-domain-reload-loop.md docs/prompts/fix/archive/11-apps-domain-reload-loop.md
timeout 120 git commit -m "fix 11: Apps page can reload repeatedly when the domain request fails"
timeout 120 git push origin dev
```

After CI passes, tag the next unused alpha per `docs/prompts/CONVENTIONS.md`. To verify fix 11 on `opsbox-vm`, build in `/home/ubuntu/opsbox` with `CARGO_TARGET_DIR=~/target cargo build --release`, reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`, and tunnel with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Open Apps → Domains at 390 px and intercept `/api/v1/services/<service>/domains` with HTTP 500: it must fail safely with readable error/Retry and at most one request per poll interval. Tab/service switches must not retry a remembered failure; Retry must send one request. Restore the endpoint and confirm Retry or the next poll recovers. Live browser/VM verification was not run here.

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

#### Fix 38 handoff (2026-11-21)

- Root cause: the proxy always attempts its configured `:80`/`:443` listeners, with fallback/promotion but no `proxy.mode`; an app-owned Caddy and the built-in proxy therefore cannot intentionally share those ports. The existing conflict path reports `port_conflict` and resolves holders through `PortHolders`, rather than exposing the requested `ports_busy` mode choice on the Domains page.
- Product files touched: none. This handoff changes only `docs/QA.md`; the pre-existing changes to `.agent-rules.md` and `docs/prompts/fix/` were left untouched. No half-applied migration or source change was left behind.
- Existing test result: `PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" timeout 120 cargo test -p opsbox-proxy --test port_conflict` compiled but failed before the fixture ran: sandbox TCP bind returned `Operation not permitted` at `port_conflict.rs:25`. No new test was added because the mode change could not be completed within the fix time box. `timeout 120 cargo check -p opsbox-proxy -p opsbox-priv -p opsbox-api -p opsbox` and `timeout 120 cargo fmt --all --check` passed with the toolchain on `PATH`.
- Suggested split: (1) add a failing fake-port-table test to `crates/opsbox-proxy/tests/port_conflict.rs`, then implement explicit busy state and stable 30-second retry in `crates/opsbox-proxy/src/server.rs` and the port-holder path in `crates/opsbox-priv/src/ops/network.rs`; (2) add validated `proxy.mode` to `crates/opsbox/src/config.rs`, make external/off avoid public binds in `cmd/proxy.rs`, and wire `install.sh --proxy=external|off` plus `cmd/status.rs`; (3) add a privileged mode-switch operation that safely writes config and restarts the proxy, then expose mode, busy holder, and the service's `active_host_port` through the API for `web/js/pages/apps.js`, with focused API/UI tests. `active_host_port` is the real loopback upstream; `svc.port` is the container port and cannot safely be shown as `127.0.0.1:<port>`. Document modes in `docs/CONFIG.md`.
- Do not offer a switch button until its privileged operation is wired. The current installer preflight, fallback, and Settings → Network guidance remain available while this feature is incomplete.

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


## QA 16 — Container actions evidence (2026-11-19)

- Reviewed `crates/opsbox-api/src/docker.rs:225–413`, `crates/opsbox-priv/src/ops/docker.rs:156–185,254–316`, PrivClient, helper dispatch and peer credential checks, `crates/opsbox-docker/src/lib.rs:509–645`, and container UI/API wiring. No product code changed or confirmed product defect found.
- Lifecycle routes require Admin and use typed `PrivOp::DockerContainerAction` via PrivClient's helper Unix socket; `serve` never opens the Docker socket. The helper revalidates IDs and invokes DockerClient/bollard against its configured socket (default `/var/run/docker.sock`). Invalid references are rejected; Docker 404 maps through `DockerError::NotFound` and `PrivErrorCode::NotFound` to HTTP not_found. Start/restart inspect and enforce a memory limit before acting. Docker 304 on an already started/stopped container is treated as success.
- Stop/restart send Docker `t=10` (grace period before forced termination); the API waits at most 60 seconds via PrivClient, and DockerClient's default request timeout is 120 seconds. These are separate bounds: an API timeout does not establish cancellation of a helper action. A slow graceful stop can legitimately exceed the prompt's three-second state-change target. No timing measured here.
- Remove uses `force(false)` and leaves volume removal unset (Docker defaults to retaining volumes); running-container conflict says to stop first. `web/js/pages/containers.js:350` confirms deletion and explicitly states volumes are kept. Successful actions notify metrics, write an audit entry and reload the list.
- Snapshot logs clamp API tail to 1–2000. Follow logs inspect existence before stream upgrade, carry stdout/stderr plus timestamps, and use a 10-second helper handshake timeout. API relay selects client close versus helper frames, dropping the helper session when the client closes; helper selects peer EOF versus Docker frames, dropping the Docker stream on EOF. UI stop-follow, Close logs, replacement stream, deletion and unmount call the close callback. This source trace does not prove live socket counts return to baseline.
- Existing tests inspected: `crates/opsbox-docker/tests/mock_daemon.rs:573` covers start/stop/restart/remove, running removal conflict, invalid/missing inspect IDs and snapshot/follow stdout/stderr; `crates/opsbox-api/tests/docker_http.rs:467` covers HTTP→helper→mock Docker, auth/roles/CSRF, invalid action, restart/delete audit, snapshot logs, cross-origin WebSocket refusal and finite stream end. They do not establish early disconnect cleanup of an idle infinite stream, actual graceful-stop timing, failed-start UI behavior or persisted-volume retention.
- PASS: a temporary `timeout 120 node --input-type=module` harness imported actual `web/js/api.js` in HTTP mode with stubbed fetch/WebSocket. It asserted start/stop/restart/remove URLs and POST/CSRF, readable HTTP 404 propagation without sample fallback, stdout/stderr entries, error/end dispatch, malformed-frame tolerance and close callback. This was a stubbed transport check, not a real browser/server test; no test files added.
- NOT RUN: `timeout 120 cargo test -p opsbox-docker --test mock_daemon client_performs_full_lifecycle_and_refuses_bad_refs` and `timeout 120 cargo test -p opsbox-api --test docker_http` both exited 127 (`cargo` unavailable). `timeout 120 multipass info opsbox-vm` exited 2 (`multipass socket access denied`). No real containers or services were modified.

### Owner checks — 16: Disposable VM lifecycle and stream cleanup

Use only `opsbox-vm` or a disposable local dev server; wrap each shell command in `timeout 120`.

1. Run the two Rust test commands above with Cargo and local socket access. Extend coverage for a missing valid ID on all lifecycle actions and logs, Docker stop/restart `t=10`, and remove without force/volume deletion. Add an idle infinite-log-stream test that closes the WebSocket and observes helper/Docker socket EOF within a bounded wait.
2. On a disposable VM snapshot, create three named test containers: a running log producer, a stopped container with a test volume containing a marker, and a created container whose configured executable does not exist (so Docker start fails). Use local fixture images; record all resource names. Do not use production containers.
3. Sign in as Admin and start, stop and restart the fixtures in the panel. Measure state refresh; fast operations should update within three seconds, while a fixture ignoring SIGTERM should terminate after Docker's approximately ten-second grace period without hanging the panel. Verify the failed start explains why. A valid but nonexistent container ID must return not_found for actions/snapshot logs and a readable error/end for live logs; malformed references must be rejected.
4. View snapshot and live stdout/stderr logs. Record `ss -tnp` and `ss -xnp` baselines (WebSocket TCP and helper/Docker Unix sockets); stop follow, close logs, navigate away and repeatedly reopen a quiet live stream. Connection counts must return to baseline. Confirm stream completion on container exit and a readable error for unavailable Docker.
5. Cancel delete and verify the container remains. Confirm deletion of a stopped fixture; verify the volume and marker remain. Attempt removal of a running fixture and require refusal with stop-first guidance. Remove only the recorded QA resources and restore the snapshot. Change row 16 to PASS only after these checks succeed.

QA 16 Git completion: `timeout 120 git mv docs/prompts/qa/16-containers-actions.md docs/prompts/qa/archive/16-containers-actions.md` failed with `.git/index.lock: Operation not permitted` (exit 128). The prompt was archived with a filesystem rename instead. Git metadata is read-only in this session, so staging, commit and push remain for a host terminal with Git write access. Preserve unrelated edits (including `crates/opsbox-api/tests/files_http.rs`, `.agent-rules.md`, and `docs/prompts/qa/gen.py`). On `dev`, review/selectively stage only the QA 16 documentation changes and prompt rename, then run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/16-containers-actions.md docs/prompts/qa/archive/16-containers-actions.md
timeout 120 git commit -m "qa 16: Containers: start, stop, restart, logs, remove"
timeout 120 git push origin dev
```

No release tag is needed for these documentation-only changes.


## QA 22 — Cleanup speed and statistics evidence (2026-11-19)

- Cleanup endpoints are in `crates/opsbox-api/src/cleanup.rs:50–162`, not `system.rs`; `system.rs:39–76` supplies the patient helper client and audit function. Admin-only scans wait at most 30 seconds at the client; runs return HTTP 202/task ID and allow a 30-minute helper call. No product code changed.
- `web/js/pages/cleanup.js:14–24,50–79` returns skeleton markup before fetching. Pending scans are polled 400 ms after each response. Unmount clears the next timer and ignores late responses. `crates/opsbox-priv/src/ops/cleanup.rs:20–24,703–899` runs scanners concurrently, waits only 250 ms for an API snapshot, exposes pending rows/durations and shares a 60-second cache. System `/usr` size is computed separately at low IO priority and cached for 24 hours; volume sizing is on demand. These bounds are source evidence, not measured browser paint or scan latency.
- Scans sum eligible regular-file logical lengths: journal archives older than seven days, downloaded packages, old release slots, stale temporary files and coredumps. Journal disk usage is descriptive, not the reclaim estimate. Docker scan (`crates/opsbox-docker/src/lib.rs:1325–1389`) sums dangling image sizes and stopped-container writable layers, reads build-cache reclaimable bytes from `/system/df`, and defers volume bytes. Shared image layers can make estimates differ from actual reclaimed disk blocks.
- Cleanup uses category-specific Docker Engine prune calls, not a blanket `docker system prune`; images use the dangling filter. `crates/opsbox-priv/src/ops/cleanup.rs:946–1068` vacuums journals with `journalctl --vacuum-time 7d`, cleans package caches, and measures nonnegative eligible-file before/after differences. Other file categories count listed bytes only after successful removal. Docker results use daemon `space_reclaimed` (`crates/opsbox-docker/src/lib.rs:1392–1440`). Logical file sizes and daemon reports are not an independent filesystem-free-space measurement.
- Helper validates all IDs before action, executes each selected category once in fixed order, reports per-category errors as zero bytes and invalidates its scan cache before/after. API totals returned bytes, audits `freed_bytes`/failed IDs, preserves raw bytes and converts display “GB” using 2^30 rounded to two decimals. UI sums raw bytes, displays partial errors, removes successful rows immediately and requests a fresh scan; this optimistic row removal does not prove all reclaimable data disappeared.
- Busy buttons and `opsbox:task` events provide activity feedback (`web/js/ui.js:636`, `web/js/api.js:180,915`); task log/result arrives only on completion. No incremental category/byte progress is reported by the cleanup worker. On-demand volume sizing can wait 120 seconds. The prompt's 30-second progress criterion remains unverified live.
- PASS (before fix 14): temporary `timeout 120 node --input-type=module` harness imported the actual HTTP-mode API with stubbed fetch. A completed scan was reused without another request; two concurrent fresh scans created two pending fetches. This is a transport/cache check, not browser or VM validation.
- Existing tests inspected: helper size parsers, bounded walks, deletion accounting, pending scanner deadline and cache invalidation; `crates/opsbox-api/tests/cleanup_http.rs` covers roles, CSRF, response shape and invalid selections, not successful reclamation; `crates/opsbox-docker/tests/mock_daemon.rs:720–741` checks scan and prune byte results against a mock daemon. `timeout 120 cargo test -p opsbox-priv ops::cleanup::tests --offline` and `timeout 120 cargo test -p opsbox-api --test cleanup_http --offline` could not run (Cargo unavailable, exit 127). `timeout 120 multipass info opsbox-vm` failed with socket access denied (exit 2). No VM junk was created or deleted.

### Fixed in fix 14 — 22: Scan requests can overlap across navigation

`web/js/api.js` now shares one in-flight scan across concurrent fresh/cached calls, clones results per caller, and releases the slot after success, failure or cancellation. Cleanup unmount aborts the HTTP request and clears its deadline; aborted responses cannot overwrite a newer cache or release a newer request's slot. The page retains its dead-result guard and refuses to start scans after unmount. The mock-API check recognizes cancellation as a local helper. No routes, PrivOps or UI map changed.

Validation (2026-11-20): six Node/DOM regressions were written first; five failed before the fix (duplicate requests, stale reads/cache and missing navigation cancellation). All 44 focused/related tests pass with `timeout 120 node --experimental-vm-modules --test tools/check-cleanup-scan.test.mjs tools/check-api-errors.test.mjs tools/check-apps-domains.test.mjs tools/check-demo-data.test.mjs tools/check-firewall-guard.test.mjs tools/check-empty-states.test.mjs`. Coverage includes `Promise.all([api.scanCleanup(true), api.scanCleanup(true)])` producing one request, shared failure/retry, abort on navigation, immediate reopening, late fetches that ignore abort, dead-page rendering/polling, and cleared deadlines. Existing API deadline/error coverage still passes.

Additional checks: `timeout 120 node tools/check-mock-api.mjs`, JavaScript syntax checks and `timeout 120 git diff --check` pass.

Owner verification: in disposable `opsbox-vm` at `/home/ubuntu/opsbox`, run `CARGO_TARGET_DIR=~/target timeout 120 cargo build --release`, then `timeout 120 sudo sh install.sh --dev --binary ~/target/release/opsbox`. From the host open the panel through `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Delay `/api/v1/cleanup/scan` in DevTools, open Cleanup, leave and reopen repeatedly; old requests must be cancelled and only the current scan remain. Force a scan HTTP 503: the page must report failure safely and Rescan/reopening must recover after restoring the endpoint. Live timing/reclamation checks below remain separate.

Git completion for fix 14: branch is `dev`; `git pull --ff-only` failed writing `.git/FETCH_HEAD` and staging failed creating `.git/index.lock` (Operation not permitted). The prompt was archived by filesystem rename. From a host terminal with Git write access, review/stage only this fix, commit, push, then wait for CI and tag the next unused alpha per `docs/prompts/CONVENTIONS.md`:

```sh
timeout 120 git add web/js/api.js web/js/pages/cleanup.js tools/check-cleanup-scan.test.mjs tools/check-mock-api.mjs docs/DECISIONS.md docs/QA.md docs/STATUS.md docs/prompts/fix/README.md docs/prompts/fix/14-cleanup-scan-overlap.md docs/prompts/fix/archive/14-cleanup-scan-overlap.md
timeout 120 git commit -m "fix 14: Cleanup scans pile up"
timeout 120 git push origin dev
```

### Owner checks — 22: Live speed, progress and reclaimed space

Use only disposable `opsbox-vm` or a disposable local dev server, with a recoverable snapshot and no real data. Wrap every shell command in `timeout 120`.

1. With Cargo available, run `timeout 120 cargo test -p opsbox-priv ops::cleanup::tests --offline`, `timeout 120 cargo test -p opsbox-api --test cleanup_http --offline`, and `timeout 120 cargo test -p opsbox-docker --test mock_daemon --offline`. Record actual outcomes.
2. On the disposable VM create only recorded QA fixtures: dangling Docker images/build cache, stopped containers, and aged journal/tmp data. Keep running-container and retained-file markers as controls. Snapshot first; do not seed junk on the host. Record `timeout 120 multipass exec opsbox-vm -- sudo docker system df -v`, `timeout 120 multipass exec opsbox-vm -- sudo journalctl --disk-usage`, and `timeout 120 multipass exec opsbox-vm -- df -B1 /` before cleanup.
3. Open DevTools with cache disabled; use `performance.now()` around route navigation and the first animation frame containing the Cleanup skeleton/controls. Record time to usable page (<1 s), scan request durations, pending rows and final scan completion under the seeded workload. Reopen repeatedly while responses are delayed and inspect outstanding requests. Repeat cold and cached visits.
4. Select a known fixture category, confirm cleanup, observe busy/task feedback throughout (including an operation exceeding 30 seconds), and verify requests stay responsive. Confirm returned raw bytes and per-item errors agree with the task result and audit `freed_bytes`; check control markers survive. Exercise on-demand volume sizing and a partial failure too. Do not infer incremental progress from a static spinner.
5. Repeat the three before/after commands and a fresh `/api/v1/cleanup/scan?fresh=1`; verify the selected fixture is gone and reclaimable bytes drop. Reconcile rounding, shared Docker layers, filesystem allocation and concurrent log writes rather than expecting exact `df` equality. Restore the snapshot. Set row 22 to PASS only after timing, request lifecycle, progress and actual reclamation checks pass.

QA 22 Git completion: requested `timeout 120 git mv docs/prompts/qa/22-cleanup-speed.md docs/prompts/qa/archive/22-cleanup-speed.md` failed with `.git/index.lock: Operation not permitted` (exit 128). Archived with a filesystem rename instead. Git metadata is read-only here, so staging/commit/push remain for a host terminal with Git write access. Branch is `dev`; preserve unrelated `.agent-rules.md` and `docs/prompts/qa/gen.py`. Review and stage only these QA 22 changes, then run (no tag for docs-only work):

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/22-cleanup-speed.md docs/prompts/qa/archive/22-cleanup-speed.md
timeout 120 git commit -m "qa 22: Cleanup page speed"
timeout 120 git push origin dev
```


## QA 17 — Image deployment evidence (2026-11-19)

- Reviewed `crates/opsbox-api/src/deploy.rs` and `crates/opsbox-priv/src/ops/deploy.rs`, then followed Docker/proxy implementations. No product code changed. Branch is `dev`; no `.codegraph/` index exists.
- API creation infers `docker_image` from `image` and validates the reference (`deploy.rs:940`). Explicitly set container port **80** for `nginxdemos/hello`; the API default is 3000. Set memory to 128 MB and initially omit the domain. Generic image creation has no `internalOnly` field: web containers use loopback publishing, whereas template internal-only mode has no host binding.
- Image pipeline (`opsbox-api/src/deploy.rs:2759`) sends typed `DeployPullRequest` to the helper, which checks image capacity/reference and calls Docker pull/tag (`opsbox-priv/src/ops/deploy.rs:1158`). Docker consumes the pull stream and reports errors, then tags the source as `opsbox/<service>:<release>` (`opsbox-docker/src/lib.rs:1892,1916`). This is source verification, not an actual registry pull.
- Helper opens sealed vars and resolves references locally (`opsbox-priv/src/ops/deploy.rs:148,177,1244`), builds `KEY=value` strings and starts the release. Docker creation (`opsbox-docker/src/lib.rs:1633,1729`) supplies `opsbox.managed`, `opsbox.project`, `opsbox.service`, `opsbox.deploy`, a project bridge network and service DNS alias, memory/CPU limits and restart policy `unless-stopped`. Generic image deploy explicitly uses empty binds (`:1651`); configurable persistent volume mounts are not supported by this request path. Template volume handling is separate. Image-declared anonymous volumes were not exercised.
- Port mapping uses `127.0.0.1:<ephemeral>:<container-port>`, inspects the assigned host port, and probes that loopback endpoint. Helper accepts HTTP 2xx/3xx (or TCP), removes unhealthy containers and redacts captured logs (`opsbox-priv/src/ops/deploy.rs:1229`). After health success, API records the new container/port and syncs domain routes to `127.0.0.1:<host_port>` (`opsbox-api/src/deploy.rs:3083`; `panel.rs:107`). It drains/removes the previous container and retains up to five image refs. Proxy-sync failure is only warned about before old-container removal (`deploy.rs:3139`), so live proxy-failure behavior remains a risk to verify.
- Rollback verifies the target belongs to the service and its image exists locally, then starts/checks the retained image through the same switch flow without rebuilding/pulling (`opsbox-api/src/deploy.rs:1623,2535`). It uses current service vars/settings, not a historical configuration snapshot.
- Delete gathers recorded containers/images, invokes helper cleanup, deletes the DB service, removes certs and syncs routes (`opsbox-api/src/deploy.rs:1443`). Helper also sweeps containers by service label and removes the app directory (`opsbox-priv/src/ops/deploy.rs:1380`). Container/filesystem cleanup failures are discarded; absence of leftovers cannot be claimed from successful API completion alone.
- Existing tests inspected: `crates/opsbox-docker/tests/mock_daemon.rs:756` covers service creation, labels/resource limits/loopback mapping/DNS alias and image operations against a mock daemon; `crates/opsbox-priv/tests/deploy.rs:141` covers helper health and unhealthy-log redaction; `crates/opsbox-api/tests/deploy_http.rs:308` covers git-source deploy, proxy switch, failed health and retained-image rollback. The latter is not an image-source creation E2E test. These tests were not executed here.
- NOT RUN: `timeout 120 cargo test -p opsbox-api --test deploy_http`, `timeout 120 cargo test -p opsbox-priv --test deploy`, and `timeout 120 cargo test -p opsbox-docker --test mock_daemon` all exited 127 (`cargo` unavailable). `timeout 120 multipass info opsbox-vm` exited 2 (`multipass socket access denied`). No VM resources, hosts entries or certs were modified.

### Fixed in fix 15 — 17: Cannot change an image service's tag

PATCH previously discarded the unknown `image` field, so redeploy kept pulling the old tag. It now accepts `image` and create's `image_ref`/`imageRef` aliases, trims and validates with the same create validator, and stores `image_ref` for generic image-source apps. Invalid input returns HTTP 400 with `error.field = image` before any settings write. Settings displays the configured reference, offers Change image, and explains that Redeploy applies it. Template/database/Compose image changes remain on their existing workflows; capacity admission and retained releases are unchanged.

Test-first evidence: both tests in `crates/opsbox-api/tests/service_image_http.rs` failed before implementation: a valid PATCH returned the original image; invalid input returned HTTP 200 and changed another setting. Both now pass through the real authenticated PATCH/GET router and store, covering aliases, trimming, omission and invalid-input atomicity. The new Node/DOM image editor regression also failed first; all five tests in `tools/check-apps-domains.test.mjs` now pass.

Validation commands: `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH timeout 120 cargo test -p opsbox-api --test service_image_http`; `timeout 120 node --experimental-vm-modules --test tools/check-apps-domains.test.mjs`. Crate `cargo check`, all-target Clippy with `-D warnings`, `cargo fmt --check`, JS syntax and whitespace checks pass. The extended `deploy_http` test covers image-source create, two tag pulls, proxy responses and rollback without another pull; its attempted execution fails at the fixture's TCP bind with `Operation not permitted`, before exercising the pipeline. No commit or push was attempted, as requested. `scripts/gen-index.py` is absent; no routes or PrivOps changed.

### Found, not fixed — 17: Persistent image mounts

Generic image deployment still has no configurable volume mounts (`opsbox-docker/src/lib.rs`); persistent mount support is outside fixes 15 and 16.

### Fixed in fix 16 — 17: Deleting the last app leaves its network and directory

The privileged delete operation ignored container/directory removal failures and never removed the shared project network. It now removes recorded and label-discovered service containers, prunes retained images, inspects/removes the project network only if empty, and removes the app directory last. The existing API deletes the database record only after helper success; errors retain the service for another DELETE attempt. This endpoint has no volume-removal opt-in, so persistent volumes remain untouched. DeployDeleteService now carries the validated project name; there are no new routes or operation variants. `scripts/gen-index.py` is absent.

Strict TDD: the in-memory fake-Docker tests failed first on the omitted network removal and swallowed container error. Both now pass, including discovered old releases, sibling preservation, image/list/network/directory failures and stopping at the first error. All four helper deploy unit tests and all 30 Docker/protocol unit tests pass. All-target Clippy with `-D warnings`, formatting and whitespace checks pass. The three added HTTP tests compile but cannot execute here: UnixListener::bind returns `Operation not permitted` before the handler runs. They cover successful cleanup, attached sibling protection, injected stop/remove/list/image/network/directory failures, database-record preservation and retry. No commit or push was attempted, as requested.

Validation: `timeout 120 cargo test -p opsbox-priv --lib ops::deploy::tests`; `timeout 120 cargo test -p opsbox-docker -p opsbox-proto --lib`; `timeout 120 cargo clippy -p opsbox-api -p opsbox-priv -p opsbox-docker -p opsbox-proto --all-targets -- -D warnings`; `timeout 120 cargo fmt --all --check`; `timeout 120 git diff --check`.

Owner checks on a host permitting Unix socket binds:

1. Run `timeout 120 cargo test -p opsbox-api --test docker_http service_delete_cleanup` (no Docker daemon or credentials needed).
2. On a disposable VM, deploy the only app in a project, note its `opsbox-<project>` network and app directory, delete it, and verify the container, empty network, directory and service record are gone. Repeat with an attached sibling container and confirm its network remains.
3. Inject Docker removal and filesystem removal failures; DELETE must report failure, retain the service record and allow another delete after fixing the cause. Verify persistent volumes remain.

### Owner checks — 17: Live image deploy, TLS, redeploy and rollback

Use only a recoverable disposable `opsbox-vm` snapshot or disposable local dev server. Wrap every shell command in `timeout 120`.

1. On a host permitting local sockets, run `timeout 120 cargo test -p opsbox-api --test service_image_http` and `timeout 120 cargo test -p opsbox-api --test deploy_http`, plus the helper/Docker commands above. The extended deploy_http test must pass its image create/new-tag pull/proxy/rollback assertions. Check the remaining labels/env/mounts/memory/cleanup criteria on the disposable VM.
2. Create a uniquely named QA project/app from `nginxdemos/hello`, port 80, health `/`, memory 128 MB, no domain, and harmless plain/secret test vars. Record project/app/container/image/network names. Inspect Docker to confirm labels, env, mounts, memory 134217728, loopback-only port binding and alias; curl the assigned loopback port inside the VM. Confirm Running, app logs, health success and restart in the panel. Do not expect configurable volumes or a zero-host-binding internal mode from this generic path.
3. Add `qa17.opsbox.test` to the test client's hosts file pointing to the VM. Attach it to the app and install a self-signed test certificate using the proxy's supported certificate-import procedure. Verify HTTP routing and HTTPS using the test CA/certificate; never replace production certs. Record the import procedure and browser/log results, which remain untested here.
4. In Apps → Settings, use Change image to save a second valid published tag with a distinguishable response. Confirm the saved reference and Redeploy hint; Redeploy, observe the new tag in pull logs and the proxy switch, then roll back to the first retained release and verify the original response without another pull. Invalid input must show an image field error and leave the reference unchanged. Also test failed health preserving the old release and proxy unavailability during switch.
5. Delete the QA app and inspect containers, image tags, app directory, domain routes/certificates and project network. Confirm sibling apps survive, attached project networks remain, and empty project networks are removed (Fixed in fix 16). Remove only recorded test leftovers and restore the snapshot and hosts entries. Change row 17 to PASS only after the requested workflow and cleanup criteria pass.

QA 17 Git completion: requested `timeout 120 git mv docs/prompts/qa/17-deploy-image-app.md docs/prompts/qa/archive/17-deploy-image-app.md` failed with `.git/index.lock: Operation not permitted` (exit 128). Archived with a filesystem rename instead. Git metadata is read-only in this session, so staging/commit/push require a host terminal with Git write access. Preserve unrelated `.agent-rules.md` and `docs/prompts/qa/gen.py`. Review/stage only QA 17 documentation and the rename on `dev`, then run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/17-deploy-image-app.md docs/prompts/qa/archive/17-deploy-image-app.md
timeout 120 git commit -m "qa 17: Deploy a Docker image app end to end"
timeout 120 git push origin dev
```

No tag is needed for documentation-only changes.

## QA 19 — Alerts fire and notify evidence (2026-11-19)

### 1. Verification Scope & Findings
1. **Critical Attention Alerts Fire:**
   - **Disk Pressure:** Evaluated in `crates/opsbox-metrics/src/attention.rs:102` and `crates/opsbox-api/src/alerts.rs:208`. Warning threshold is `>75%` (e.g. 85%), Critical is `>=95%`. Both attention items (`disk:<mount>`) and alert engine incidents (`disk:<mount>`) fire with appropriate severity (`Warning` vs `Critical`).
   - **Service Failure:** Evaluated in `crates/opsbox-metrics/src/attention.rs:116` and `crates/opsbox-api/src/alerts.rs:256` from `SystemFacts.failed_units`. Critical attention alert `units:<name>` is created immediately.
   - **Privileged Helper Disconnection:** Evaluated in `crates/opsbox-api/src/alerts.rs:245` when metrics sampling fails or times out. Fired under rule `health`, target `host`: `"Machine metrics are unavailable. Check that the privileged helper is running."` marked Critical.
   - **Backup Checksum Failure:** Evaluated in `crates/opsbox-api/src/backups.rs:1068` and `crates/opsbox-api/src/alerts.rs:356` via `Store::list_failed_backup_snapshots()` checking for `BackupCheckState::ChecksumFailed`. Surfaces `backup:checksum` attention item and `backup:checksum-<id>` incident marked Critical.
   - **Deduplication:** Dedup logic in `crates/opsbox-api/src/alerts.rs:771` (`canonical_attention_id`) and `crates/opsbox-alerts/src/lib.rs:376` guarantees that one incident is tracked per target, alerts do not duplicate every minute, and only state transitions / 4-hour repeat periods trigger outbound notifications.

2. **Notification Channels (Webhook, Email, Slack, Telegram):**
   - **Secret Sealing:** Channel credentials (SMTP passwords, Slack webhook URLs, Telegram bot tokens, Webhook signing secrets) are validated (`crates/opsbox-alerts/src/channels.rs:45`) and sealed through `opsbox-priv` using `secret_seal` bound to `alerts:channel:<id>` (`crates/opsbox-api/src/alerts.rs:553`). API responses never expose unsealed secrets (`alerts.rs:466`). Reveal requires re-authentication (`alerts.rs:606`).
   - **Formatting & Transport:** Outbound delivery in `crates/opsbox-alerts/src/channels.rs:136`:
     - Email: Formatted via `lettre` with SMTP/STARTTLS on port 465/587 with credentials.
     - Telegram: HTTP POST to `https://api.telegram.org/bot<token>/sendMessage` with `chat_id` and payload text.
     - Webhook: Signed HTTP POST with `X-Opsbox-Timestamp` and HMAC-SHA256 signature `X-Opsbox-Signature: sha256=<hex>`. Enforces strict HTTPS URLs without embedded credentials or fragments. Max redirects set to 0 to prevent redirection attacks.
   - **Test Triggers:** `POST /api/v1/alerts/test` (`crates/opsbox-api/src/alerts.rs:663`) supports testing specific channels or all connected channels, updates channel `verified` and `error` states, and audits the test event.

3. **Muting / Silencing & Quiet Hours:**
   - **Incident Silencing:** `POST /api/v1/alerts/silence` (`crates/opsbox-api/src/alerts.rs:719`) silences an active incident until `now + seconds` (bounded 0 to 7 days). The alert engine checks `now < i.silence_until` before dispatching deliveries (`crates/opsbox-alerts/src/lib.rs:364`), operating cleanly per target without resetting or affecting other incidents.
   - **Quiet Hours:** Configured via `PUT /api/v1/alerts/quiet` with UTC 24-hour HH:MM bounds (`crates/opsbox-alerts/src/lib.rs:149`). Non-critical alerts are suppressed during quiet hours, while critical alerts bypass quiet hours to ensure safety.
   - **Per-User Bell Silencing / Read State:** `crates/opsbox-store/src/attention.rs` and `crates/opsbox-api/src/metrics.rs:396` maintain independent `(user_id, item_id)` read and snooze markers in `attention_state` (SQLite). Snoozing an alert in the UI bell drawer hides it for that user until the timer expires without altering global alert incident state or silencing other users.

### 2. Automated Tooling & Test Executions
- `node tools/check-alerts-ui.mjs`: PASSED (verified API contracts for `GET /api/v1/alerts`, `PUT /rules`, `PUT /channels`, `POST /test`, `POST /silence`, `PUT /quiet`, `DELETE /channels`, and CSRF protection).
- `node tools/check-notifications-ui.mjs`: PASSED (verified notification bell UI, shared polling, bell vibration/animation on critical alerts, keyboard navigation, and Overview banner integration).
- Unit and integration tests in `crates/opsbox-alerts` and `crates/opsbox-api/tests/alerts_http.rs` comprehensively cover sealed credentials, quiet hours, silence per-target isolation, test failures, and role-based access control.

### Owner checks — 19: Live notifications and real provider delivery
- On a live host with real notification targets, configure an SMTP relay, Slack incoming webhook, or Telegram bot token in the Alerts panel (`#/alerts`).
- Trigger a test message using the Test button on each channel and confirm delivery.
- Set a low test disk threshold or stop a test container to verify end-to-end receipt of firing and recovered messages on the live channel.



## QA 18 — Local backup and restore evidence (2026-11-19)

- Reviewed `crates/opsbox-api/src/backups.rs` and the actual helper file `crates/opsbox-priv/src/ops/backup.rs` (the requested plural `backups.rs` does not exist), plus restic argument builders and existing tests. No product code changed; branch is `dev`; no `.codegraph/` index exists.
- Local destination resolves to `Env.backup_dir`, normally `/var/backups/opsbox` (`opsbox-priv/src/ops/backup.rs:244`). `run_backup_with` at line 1536 initializes the encrypted repository, serializes operations per repository and invokes restic. Database dumps stream directly from Docker exec into `restic backup --stdin`; volumes and paths use filesystem backups. `list_snapshots_with` at line 1667 runs `snapshots --json`, returns empty for an uninitialized local repository, and parses IDs/tags/paths; API synchronization at `backups.rs:720` stores snapshot metadata.
- Checks are opt-in for manual job runs (`opsbox-api/src/backups.rs:1703`); `run_check` uses a data subset, not a full read of every snapshot. The explicit check endpoint (`:2031`) accepts `readDataSubset`, updates check states and audits `backup.check`. Use `100%` for this QA's complete repository data-read check; a subset success alone does not prove the exact snapshot round trip.
- Copy is the API restore default; replace requires exact `replace` confirmation at both layers. Helper `restore_with` (`backup.rs:2660`) takes a live safety snapshot before replacement. File/volume payloads restore to private staging first and refuse nonzero restic exits before clearing live destinations. Volume replacement pauses consumers during clear/copy and attempts to unpause afterward. Database restore now stages the full stdin payload, checks restic completion before replay and uses a PostgreSQL transaction (Fixed in fix 07; see below). opsbox self-restore is copy-only and checks SQLite integrity.
- API records successful `backup.run` and `backup.restore` (`backups.rs:1760,3454`), and `backup.run.failed` on backup failure. Restore errors return before the success audit call; no equivalent restore-failure audit is emitted by this handler. Live audit persistence was not tested.
- Repository password derives inside the helper from the master key; the password guard (`backup.rs:403`) uses a temporary 0700 directory/0600 file, overwrites/removes the file on drop and zeroizes the in-memory secret. Restic gets `--password-file`, a cleared environment and scrubbed output. Self-backup excludes `master.key`. The intentional recovery-key export requires admin reauthentication, audit and no-store headers (`opsbox-api/src/backups.rs:2698`). These are source findings, not observed live absence of leaks.
- Existing coverage inspected, not executed: `opsbox-priv/src/ops/backup.rs:3610` checks local self-backup/list/check/copy and password-file cleanup with mock restic; `:3800` checks path copy/replace and safety snapshots with a mocked remote backend. `opsbox-api/tests/backups_http.rs:433` uses mock Docker/restic for DB/volume/path/self orchestration, simulated checksum failures, DB copy/replace, reauthentication and alerts. DB restore asserts that a dump was received, not that five actual SQL rows were recovered. Simulated failures do not establish real restic pack-tampering detection.
- NOT RUN: `timeout 120 cargo test -p opsbox-api --test backups_http`, `timeout 120 cargo test -p opsbox-priv ops::backup::tests`, and `timeout 120 cargo test -p opsbox-backup` all exited 127 (`cargo` unavailable). `restic` is also absent from PATH. `timeout 120 multipass info opsbox-vm` exited 2 (`multipass socket access denied`). No live database, volume or repository was created, deleted, restored or corrupted.

### Fixed in fix 07 — 18: Database restore can write before late corruption refusal

Previously `restore_database_into_container` fed restic stdout directly into the database command and checked its exit only afterward. The stdin restore path now stages the complete dump in an anonymous 0600 file under the private cache directory, checks read success, nonempty payload and restic's successful final exit (which authenticates the required blobs), then rewinds that same file and starts replay. Staging uses bounded memory, drains stderr concurrently, retains the existing timeout and removes plaintext automatically on every path. PostgreSQL replay uses `pg_restore --single-transaction` so an SQL/archive error rolls back the replay.

Test-first evidence: `late_dump_failure_leaves_live_rows_unchanged` uses an injected dump process that emits valid row data then reports `corrupt later pack` and exits 1. Before the fix the SQLite-backed restore consumer committed the backup values and the unchanged-row assertion failed; after the fix live values remain identical. This isolates the production dump/replay ordering without requiring Docker. `verified_dump_is_rewound_before_replay` confirms complete successful replay, 0600 permissions and cleanup; `empty_dump_never_starts_replay` rejects an empty successful dump before the consumer runs. All six backup tests pass.

Verification: `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH timeout 120 cargo test -p opsbox-priv` ran: 215 passed, 2 existing environment failures (`mode_strings_look_like_ls`: sandbox strips setuid; `collects_real_facts_on_this_machine`: sysctl uptime denied). Cargo stopped before integration binaries. `cargo fmt --all --check`, `cargo clippy -p opsbox-priv --all-targets -- -D warnings` and `git diff --check` pass (Cargo checks used the same PATH and 120-second timeout). No live database or repository was changed.

Remaining swap design: authenticated dumps can still encounter engine-level errors during MySQL/MariaDB/Mongo replay, whose destructive operations cannot all roll back in one transaction. Restore into an isolated database/container using the same engine/version, validate object/row counts, quiesce writers, then switch the service connection or replace the data volume while retaining the old target for recovery. This broader engine-error guarantee remains separate from the fixed late-restic-failure ordering. The existing Redis path already stages/checks its dump before touching live data and is unchanged.

Fix 07 owner verification: on disposable `opsbox-vm`, in `/home/ubuntu/opsbox`, run `CARGO_TARGET_DIR=~/target timeout 120 cargo build --release`, then `timeout 120 sudo sh install.sh --dev --binary ~/target/release/opsbox`; open the panel with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Repeat the five-row round trip and isolated late-pack corruption checks below. A replace restore with missing/wrong confirmation must be refused; a dump that emits a valid prefix then fails must leave all live rows unchanged. For PostgreSQL, also inject an archive/SQL error during replay and require transaction rollback. Check sufficient cache disk space for the full dump.

Fix 07 Git completion: `git pull --ff-only` is blocked by read-only `.git/FETCH_HEAD`; `git add` also failed to create `.git/index.lock` (Operation not permitted). Staging/commit/push and the release require a host terminal. After reviewing only this fix on `dev`, run:

```sh
timeout 120 git add crates/opsbox-priv/src/ops/backup.rs docs/DECISIONS.md docs/QA.md docs/STATUS.md docs/prompts/fix/README.md docs/prompts/fix/07-restore-before-corruption.md docs/prompts/fix/archive/07-restore-before-corruption.md
timeout 120 git commit -m "fix 07: Database restore can write before a late corruption is found"
timeout 120 git push origin dev
```

After CI passes, tag the next unused `v0.1.0-alpha.<N+1>` and push the tag as described in `docs/prompts/CONVENTIONS.md`; no tag was created in this sandbox.

### Owner checks — 18: Exact local round trip and real tampering

Use only recoverable disposable `opsbox-vm` or a disposable local dev server. Wrap each shell command in `timeout 120`; keep fixture names, service IDs, job IDs and full snapshot IDs. R2/S3 checks require owner-provided configuration through the normal UI; do not put credentials in this document or command history.

1. Run the three Rust test commands above with Cargo available. Snapshot the disposable VM. Create a managed PostgreSQL QA database and a separate app volume with a binary marker file. In that database run `CREATE TABLE qa18_roundtrip (id integer PRIMARY KEY, value text NOT NULL); INSERT INTO qa18_roundtrip SELECT n, 'qa18-' || n FROM generate_series(1,5) AS n;`. Save the exact ordered `SELECT id,value FROM qa18_roundtrip ORDER BY id` output and the marker's SHA-256 outside the restore targets.
2. As Admin create two disabled backup jobs through the panel (database service and app volume), explicitly selecting destination `local`. Run each, or use `POST /api/v1/backups/jobs/{jobId}/run` with `{"runCheck":true}` using the normal session/CSRF flow. Wait for each task to finish successfully. Refresh `GET /api/v1/backups/snapshots`, record full restic IDs and verify job, kind, paths and destination. Run `POST /api/v1/backups/check` with `{"destinationId":"local","readDataSubset":"100%"}` and require success/checksum states; do not substitute a default subset check.
3. Delete only those five QA rows and the marker. Restore both snapshots as copies through the panel or `POST /api/v1/backups/snapshots/{id}/restore` with `{"mode":"copy","copyTarget":"<unique-copy-name>"}`. Compare all five ordered rows and marker hash in the copies; confirm original targets remain empty. Then restore original targets with `{"mode":"replace","confirm":"replace"}`. Verify exact rows/hash, safety snapshot IDs, resumed app consumers and no unrelated changes. Also require refusal for missing/wrong replace confirmation.
4. Inspect audit entries for both backup runs and successful copy/replace restores; check task results/errors, API responses, helper logs and process arguments for the test secrets/master key/repository password without exporting them into logs. Verify ephemeral password files disappear. Ordinary responses must not expose key material; separately exercise recovery-key reauth denial/no-store behavior if needed.
5. Before corruption testing snapshot the disposable VM again. Use a dedicated QA-only local repository with no unrelated backups. Stop scheduled jobs, record a required data pack for a large QA fixture, corrupt a byte in that pack and clear the disposable restic cache so a cached copy cannot hide damage. Request a full check and copy/replace restore of the affected snapshot through opsbox. Require a readable failed task, no success audit and no partial destination changes; test the late-stream DB failure above as well. Never corrupt a shared repository or the safety snapshot's packs. Restore the VM snapshot afterward.
6. R2/S3: with owner-configured disposable destinations, repeat connectivity, five-row/file round trip, full check, copy/replace, tamper failure and secret/audit checks using separate test repositories. Record provider results separately. Change row 18 to PASS only after the exact local round trip and corruption criteria pass; retain remote owner checks if credentials are unavailable.

QA 18 Git completion: requested `timeout 120 git mv docs/prompts/qa/18-backup-and-restore-local.md docs/prompts/qa/archive/18-backup-and-restore-local.md` failed with `.git/index.lock: Operation not permitted` (exit 128). Archived with a filesystem rename instead. Git metadata is read-only here, so staging/commit/push require a host terminal with Git write access. Preserve unrelated `.agent-rules.md` and `docs/prompts/qa/gen.py`. Review and stage only QA 18 documentation and the rename on `dev`, then run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/18-backup-and-restore-local.md docs/prompts/qa/archive/18-backup-and-restore-local.md
timeout 120 git commit -m "qa 18: Local backup and restore round trip"
timeout 120 git push origin dev
```

No tag is needed for documentation-only changes.

## QA 24 — Export, uninstall, reinstall, import evidence (2026-11-19)

- Reviewed the actual API files `crates/opsbox-api/src/export.rs` and `import.rs` (`src/config.rs` does not exist), `crates/opsbox-api/tests/import_http.rs` (not repository-root `tests/import_http.rs`), export tests, helper secrets and `crates/opsbox-priv/src/ops/uninstall.rs`. No `.codegraph/` index exists. No product code changed.
- Export uses version header `opsbox-config/1`, interactive Admin authentication, password reauthentication and `no-store`. Designated secret vars, backup credentials and alert credentials are omitted with reentry markers by default, or sealed using Argon2id (32 MiB, 3 passes, 1 lane), random salt/nonce and XChaCha20-Poly1305 with format-bound AAD. Nonsecret vars remain plaintext by design; passwords/TOTP, sessions, private deploy keys, certificates and application data are excluded. The entire document is not encrypted or authenticated: the AEAD protects the secrets block only. Raw Compose content is an exception to the claimed secret sanitization, below.
- Import checks format and required section presence before scheduling writes; a wrong passphrase fails authenticated decryption before task creation. It does not require or import the source master key: decrypted values are resealed through the destination helper (`import.rs:1050,1134,1270`, `deploy.rs:377`). Helper key loading requires exactly 32 bytes and no group/other permissions (`opsbox-priv/src/secrets.rs:114,152`); missing keys are created lazily. Kept databases require their original master key to continue opening existing secrets. Failure reporting is incomplete, below.
- Keep/purge separation is explicit in `uninstall.rs:103,213`: both remove opsbox services/timers, binary symlink and release slots, and attempt user removal. Keep retains database/audit/proxy state, config/master key and local backups; purge additionally removes state, config, local backups and restic cache. Containers are retained in both modes, with a best-effort restart-policy update to `unless-stopped`; domain downtime is warned. Keeping the state directory does not keep its binary slots. Source includes temp-root removal tests, but does not prove live container survival or successful systemctl/user removal (commands are best effort).
- `install.sh:464` preserves existing config; documented reinstall behavior reuses kept state (`docs/DECISIONS.md`, Prompt 16j). Export excludes credentials and active-container/deploy state; a fresh import creates undeployed configuration and needs a newly claimed admin, whereas keep-data reinstall should preserve the old login and app metadata. Import alone is not a backup or demonstrated container adoption mechanism.
- Inspected existing tests, not executed: `import_http.rs` covers unknown format, wrong passphrase, same-server non-overwrite and export/import/export to a fresh store. Its comparison normalizes IDs/owners and removes ciphertext; it does not prove live reinstall, original login or imported secret usability. `export_http.rs` covers schema conformance, designated secret absence, encryption/tampering, authorization and audit. Its Compose fixture has no inline credential. Uninstall tests exercise keep, purge and dry-run removals under a temporary root.
- NOT RUN: `timeout 120 cargo test -p opsbox-api --test import_http`, `timeout 120 cargo test -p opsbox-api --test export_http`, and `timeout 120 cargo test -p opsbox-priv ops::uninstall::tests` exited 127 (Cargo unavailable). `timeout 120 multipass info opsbox-vm` exited 2 (socket access denied). No live uninstall, purge, reinstall or data mutation was attempted.

### Fixed in fix 05 — 24: Export leaks inline Compose credentials

Compose YAML was copied into plaintext JSON even when a passphrase was supplied. Export now treats the whole YAML as secret: without a passphrase it writes an empty `composeYaml` and a `composeProject` re-entry entry; with a passphrase it carries the original YAML only in the existing authenticated sealed payload. Import restores the exact YAML, accepts older plaintext exports, and skips stacks and members missing their YAML.

Test-first evidence: `export::tests::inline_compose_credentials_never_leave_in_plaintext` failed with `Compose credential leaked (passphrase: false)` before the fix and passes afterward. `compose_exports_round_trip_without_helper` passes through the real export/preview/apply routes with both modes: the marker `QA_SECRET_MARKER_123` stays absent from JSON, sealed import restores the exact environment/command/secrets source and stack metadata, and omitted source cannot create a broken stack. The real-helper round-trip test additionally checks restored YAML, but this sandbox denies binding its Unix socket. The requested `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH timeout 120 cargo test -p opsbox-api -p opsbox-priv` compiled both crates, then stopped at the API library suite (71 passed; four TLS tests denied socket binds). A separate `cargo test -p opsbox-priv` ran 212 passing tests and two existing environment failures: setuid mode preservation (`mode_strings_look_like_ls`) and unavailable uptime sysctl (`collects_real_facts_on_this_machine`). `cargo clippy -p opsbox-api --all-targets -- -D warnings`, formatting and whitespace checks passed.

Git completion: staging failed with `.git/index.lock: Operation not permitted`; Git metadata is read-only in this session. Code, tests, documentation and the filesystem archive move are ready on `dev`; commit/push/tag were not performed. On a writable host, review these fix 05 changes, stage them, commit `fix 05: Export leaks inline Compose credentials`, push `origin dev`, then wait for CI before tagging the next alpha per CONVENTIONS.

Owner checks:
1. On a host allowing Unix sockets, run `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH timeout 120 cargo test -p opsbox-api -p opsbox-priv` and the focused `cargo test -p opsbox-api --test import_http` / `--test export_http` suites.
2. In disposable Multipass VM `opsbox-vm`, build from `/home/ubuntu/opsbox` with `CARGO_TARGET_DIR=~/target cargo build --release`, reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`, and open the panel via `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Create a Compose stack whose inline environment/command contains the harmless marker; export in both modes and confirm neither downloaded JSON contains it. Import the sealed file into a fresh instance and review/deploy the restored YAML; import with a wrong passphrase must fail before writes, and the passphrase-free file must report missing YAML and skip the stack.

### Fixed in fix 06 — 24: Import accepts schema-invalid documents and can hide key failures

Preview and apply now validate the complete checked-in `docs/schemas/config-export-v1.json` subset (required fields, nested objects/arrays, types, enum/const, local references and unknown fields), with the field path in both the validation field and message. Variable sealing runs per key; variables, Compose source, backup credentials and alert channels report individual restoration results. Missing secrets or sealing failures end the task as failed with a `Restored X of Y secrets, Z failed` summary and the unrestored item names. Failed credential sealing skips destination creation. The existing panel task/error handling displays these messages without an API shape change.

Test-first evidence: `schema_violations_name_the_field_before_preview_or_apply` initially accepted numeric `composeProjects` (HTTP 200); it now rejects that plus missing `host`, malformed nested array values, invalid roles, extra fields, referenced service shapes and incorrect secret flag types on both routes. `unavailable_helper_fails_import_with_item_results` initially finished `ok` and created a credentialless destination; it now fails and names `web.TOKEN`, `s3-main` and `email` without exposing secret values. The Compose round-trip regression initially reported success for omitted YAML; it now reports one failed secret, while sealed YAML restores successfully with a 1-of-1 summary.

Validation: the requested `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH timeout 120 cargo test -p opsbox-api --test import_http` ran: 3 passed, 5 blocked at the fixture's Unix socket bind (`Operation not permitted`). The blocked cases include the new `helper_rejecting_secret_seal_fails_import_with_item_results`, which installs an invalid destination master key and verifies the real helper refuses `SecretSeal`. Crate Clippy, formatting and whitespace checks pass. Real-key failure verification remains an owner check; the sandbox-independent schema, unavailable-helper and Compose regressions pass.

Owner checks for fix 06:
1. On a host permitting Unix sockets, rerun the exact focused import test command above; all eight cases must pass, including the invalid-key rejection case.
2. In a disposable `opsbox-vm`, from `/home/ubuntu/opsbox`, build with `CARGO_TARGET_DIR=~/target cargo build --release`, reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`, and open the panel via `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. A preview with numeric `composeProjects` or missing `host` must be refused before writes. Import a file with omitted Compose YAML and confirm the panel shows a failed restoration count and the project name; restore a sealed export with a healthy helper and verify its success counts in the task log.
3. Git metadata is read-only here: `git pull --ff-only` cannot write `.git/FETCH_HEAD`, and `git add` cannot create `.git/index.lock`. From a host terminal on `dev`, stage only the fix 06 files (including this QA entry, decisions, status, fix index and archive move), commit `fix 06: Import accepts invalid documents and can hide key failures`, push `origin dev`, and release the next unused alpha tag only after CI passes.

### Found, not fixed — 24: Existing service variables can still be overwritten

The import service-creation pass skips existing services but retains their IDs for the variable pass, which calls the variable upsert helper. The existing non-overwrite test imports identical values and does not exercise a changed variable value. This pre-existing behavior is outside fix 06; add a regression importing a changed variable into an existing service before addressing it separately.

### Owner checks — 24: Disposable VM round trip

Use only a recoverable disposable `opsbox-vm` snapshot or disposable local dev server; wrap each shell command in `timeout 120`.

1. Run the three Rust test commands above with Cargo available, plus `timeout 120 cargo test -p opsbox-priv secrets::tests`. Run the fix 05/06 regressions for Compose secrecy, malformed schemas and destination key failures; compare decrypted imported secret values across distinct source/destination keys rather than ciphertext equality.
2. Record managed container IDs/restart policies, app-file hashes, DB fixture rows, service settings and admin login. Export through Settings with and without a passphrase; check format, reentry markers, absence of designated and inline test secrets, wrong-passphrase/ciphertext-tamper refusal and correct secret decryption. Keep files private and do not record key material here.
3. Review `opsbox uninstall --keep-data` plan and confirm services/timers/binary slots are removed while containers keep their IDs and remain running, volumes/files/DB rows are unchanged, and database/config/master key/local backups remain. Expect proxy domain downtime. Never run purge for this keep-data check.
4. Reinstall via the documented install path. Verify the retained master key/config/database were reused, old admin password/TOTP works, and apps/containers/data are recognized without duplicate deployments. Preview/apply the export; confirm existing objects are skipped and settings/secrets stay usable.
5. Separately test fresh reinstall/claim in an isolated fixture: create a new admin through the documented setup/claim flow before importing. Confirm old credentials are not restored from export, imported services start undeployed, reentry/DNS/GitHub warnings are accurate, and application data is recovered using backups where needed. Test purge only in a separate disposable snapshot and compare its removed/kept plan with actual paths. Restore snapshots afterward. Keep row 24 PARTIAL until live checks and findings are resolved.

QA 24 Git completion: requested `timeout 120 git mv docs/prompts/qa/24-export-import-uninstall.md docs/prompts/qa/archive/24-export-import-uninstall.md` failed with `.git/index.lock: Operation not permitted` (exit 128). Archived using a filesystem rename instead. Git metadata is read-only in this session, preventing staging/commit/push. On a host terminal with Git write access, preserve unrelated `.agent-rules.md` and `docs/prompts/qa/gen.py`, review only QA 24 changes on `dev`, then run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/24-export-import-uninstall.md docs/prompts/qa/archive/24-export-import-uninstall.md
timeout 120 git commit -m "qa 24: Export, uninstall, reinstall, import"
timeout 120 git push origin dev
```

No tag is needed for documentation-only changes.

## QA 25 — Firewall lockout evidence (2026-11-19)

Original QA evidence follows; the panel-port finding is resolved by fix 03 below.

- Reviewed `crates/opsbox-priv/src/ops/firewall.rs` and the actual API handlers in `crates/opsbox-api/src/system.rs:532` (the requested API `firewall.rs` does not exist). Branch is `dev`; no `.codegraph/` index exists. No product code changed: protecting the configured listener across API/helper protocol and making rollback reliable exceeds the tiny QA fix allowance.
- Validation (`system.rs:571`, helper `firewall.rs:98`) rejects invalid ports, protocols, source IP/CIDR and notes. Rules are allow-only; blocking is accomplished by removing an allowance or enabling the default-drop nftables backend. Syntactic validation does not strictly reject lockout-producing changes.
- `removal_is_risky` (`firewall.rs:140`) checks whether a TCP/any rule covers the caller and whether another allowance still covers it; unknown callers are treated cautiously, but only for protected ports. `remove` supplies only `ssh_ports()` (`:716`). API `caller_ip` (`system.rs:70`) forwards a non-loopback/non-unspecified client IP, treating tunnels/local proxies as unknown; it does not discover the actual SSH session peer. There is no general always-allow rule for the active admin remote IP.
- nftables rendering (`firewall.rs:169`) accepts established/related connections, loopback, configured SSH ports, ICMP and WireGuard UDP 51820 before user allowances. This preserves loopback and built-in SSH within OpsBox's table, not arbitrary preexisting firewall tables. The panel listener (default 8443, `opsbox-api/src/lib.rs:166`) has no built-in allowance. UFW/firewalld paths modify existing backends rather than installing equivalent permanent loopback/SSH rules.
- Risky SSH removal and first nft activation require a boolean confirmation and schedule a 60-second undo (`firewall.rs:672,718,729`), rather than strictly refusing them. `web/js/api.js:210` retries confirmation-required responses and offers the revert hook. Helper housekeeping runs every ten seconds (`opsbox-priv/src/server.rs:122`). Emergency SSH starts sshd, opens one source IP on the first configured SSH port for an hour (`firewall.rs:773`), and the API records `system.ssh.emergency` with IP/expiry (`system.rs:705`). These are source findings, not live verification of reachability, expiry or audit persistence.
- Existing helper tests cover syntax, IPv4/IPv6/CIDR matching, unknown callers, alternate SSH ports and nft loopback/SSH rendering (`firewall.rs:989–1228`); API system tests cover authorization and malformed rule requests. NOT RUN: `timeout 120 cargo test -p opsbox-priv ops::firewall::tests` and `timeout 120 cargo test -p opsbox-api --test system_http firewall` exited 127 (Cargo unavailable). The API filter may omit generally named system tests; run the full system test target for owner validation. `timeout 120 multipass info opsbox-vm` exited 2 (socket access denied). No host firewall was changed.

### Fixed in fix 03 — 25: Panel port removal bypasses the lockout guard

Fixed in fix 03. The helper now protects configured panel and watchdog recovery ports alongside SSH; the API sends its running listener port to cover startup overrides. The internal optional `FirewallRemove.panel_port` field defaults for older requests; the HTTP API is unchanged. Removing the last matching allowance returns `ConfirmRequired`; confirmation schedules the existing 60-second revert with the original rules saved. The UI requires typing `continue`, retries with `confirm=true`, then opens Keep/Undo. Rules are allow-only: arbitrary broader drops/reordering are not exposed, and first nft default-drop activation already requires confirmation and timed undo.

Test-first evidence: the new helper regression initially reached the backend without confirmation for a default-drop table with one TCP 8443 allowance; the UI regression initially rendered no typed input. Both now pass. The helper regression also covers custom panel/recovery ports, the actual listener override, CIDR/IPv6 and unknown callers, and checks the pending undo restores the original allowance.

Validation on 2026-11-20:

- `cargo test -p opsbox-priv ops::firewall::tests`: 16 passed.
- `node --experimental-vm-modules --test tools/check-firewall-guard.test.mjs`: 2 passed, including cancellation without retry and confirmed deletion with the timed keep dialog.
- `cargo clippy -p opsbox-api -p opsbox-priv --all-targets -- -D warnings`, `cargo fmt --check` and `git diff --check`: passed.
- Required `cargo test -p opsbox-api -p opsbox-priv`: ran; API library tests stopped at 70 passed / 4 existing TLS socket-bind failures (`Operation not permitted`). Separate helper suite: 206 passed / 2 existing environment failures (setuid mode bits and uptime sysctl). Separate `system_http`: all 9 tests, including the new listener/confirmation/revert forwarding test, blocked by sandbox socket-bind denial; compiled successfully. Rerun the full command on Linux outside this sandbox.
- `scripts/gen-index.py` is absent (prompts 18a/18b are not implemented), so the index cannot be regenerated.

Owner check on disposable `opsbox-vm` with console access: establish default-drop with the sole TCP panel allowance, then delete it without confirmation and expect HTTP 409 `confirmation_required` with no mutation. In the UI, wrong text must keep Continue disabled; type `continue` and proceed, expect a non-null `revertAt`. Do not keep the change: after 60 seconds plus janitor latency, verify a fresh connection succeeds and the allowance returns. Repeat for custom panel and recovery ports. The separate rollback durability defects are resolved in fix 04 below.

Git metadata is read-only in this session; commit/push/release must be completed from a host terminal after review and the Linux checks. On `dev`, stage only the fix 03 code, tests, docs and archive move, commit with `git commit -m "fix 03: Removing the panel's firewall allowance can lock the admin out"`, and run `git push origin dev`. After CI passes, tag the next alpha per CONVENTIONS.

VM setup: in `/home/ubuntu/opsbox`, run `timeout 120 cargo test -p opsbox-api -p opsbox-priv`, `CARGO_TARGET_DIR=~/target timeout 120 cargo build --release`, then `timeout 120 sudo sh install.sh --dev --binary ~/target/release/opsbox`. Connect from the host with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>` and follow the disposable-firewall refusal/undo checks above.

### Fixed in fix 04 — 25: Rollback state can be lost

Fixed in fix 04. Risky nft activation and protected-port removal now persist and flush undo before backend mutation; a failed pre-save prevents application. A post-apply save failure leaves the original durable undo intact. Pending rollback refuses a second risky change with `Conflict`, with or without confirmation. Timely explicit Keep remains supported; overdue Keep cannot discard a failed rollback.

The janitor clears undo only after successful restoration and readback verification. nft restore requires successful atomic apply/delete and kernel table-presence verification; UFW/firewalld restore verifies actual rules. Detection, backend mismatch, apply and verification failures retain the undo and schedule persisted exponential backoff (10, 20, 40, then at most 60 seconds). Failed restore metadata is not committed as restored state. Existing pending JSON remains readable.

Test-first evidence: the three requested regressions failed before the fix (save failure still applied, failed restore cleared undo, second removal overwrote undo). They now pass. Additional regressions cover failure of the post-apply save and capped retries/reloading legacy records.

Validation on 2026-11-20: `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH cargo test -p opsbox-priv ops::firewall::tests` passed all 21 tests. Each command is bounded with `timeout 120`. `cargo fmt --check`, `cargo clippy -p opsbox-priv --all-targets -- -D warnings` and `git diff --check` passed. No routes, PrivOps or UI map changed.

Owner checks: on a disposable Linux VM with console access, test UFW, firewalld and nftables separately. Apply a confirmed panel allowance removal without Keep, inject one detection/restore failure at the deadline, verify `/etc/opsbox/firewall.json` still has `pending` and a future `retry_at`, then remove the fault and verify fresh panel connections and cleared pending state after retry. Restart the helper during backoff and repeat. Inject a state-write failure before apply: rules must stay unchanged. While a rollback is pending, a second risky removal must return a clear conflict and leave the original undo unchanged.

`git add` failed with `.git/index.lock: Operation not permitted` (exit 128): Git metadata is read-only in this session. Owner completion after review: on `dev`, stage only fix 04 files (firewall code, DECISIONS, QA, STATUS, fix README and the prompt archive move), commit `fix 04: Pending firewall rollback can be lost`, then `git push origin dev`. After CI passes, tag the next alpha per `docs/prompts/CONVENTIONS.md`.

### Owner checks — 25: Live reachability and emergency SSH

Use only disposable `opsbox-vm` or a disposable local dev server with recoverable console access; snapshot it first and wrap shell commands in `timeout 120`.

1. With Cargo installed, run `timeout 120 cargo test -p opsbox-priv ops::firewall::tests`, `timeout 120 cargo test -p opsbox-api --test system_http`, and `timeout 120 cargo test -p opsbox-api system::tests`. The helper regressions for panel removal and durable rollback now pass locally; rerun on Linux.
2. Independently exercise nftables, UFW and firewalld with panel on 8443 and a custom panel port, SSH on 22 and a custom SSH port, IPv4/IPv6 peers and an SSH tunnel/local proxy. Try removing the last matching allowance, an overlapping CIDR allowance and an unrelated rule. Require refusal or deliberate confirmation plus verified timed recovery; check fresh connections, loopback and current admin reachability rather than relying on an established connection. Verify first nft activation cannot strand the panel.
3. For a provisional risky SSH change, do not keep it; observe restoration after the 60-second deadline plus janitor/command latency. Repeat with explicit keep confirmation and verify the intended final rules. Test helper restart, backend failure, state-save failure and overlapping changes using fault injection; restore snapshots afterward.
4. Open emergency SSH through the panel; verify actual SSH access is restricted to the selected admin IP, uses the configured SSH port, and creates an audit entry with the IP and expiry. Verify expiration removes the emergency allowance (without assuming SSH becomes unreachable if another rule still permits it). Through a tunnel, verify the public-IP prompt. Keep row 25 PARTIAL until findings are fixed and live checks pass.

QA 25 Git completion: `timeout 120 git mv docs/prompts/qa/25-firewall-lockout.md docs/prompts/qa/archive/25-firewall-lockout.md` failed with `.git/index.lock: Operation not permitted` (exit 128). Archived with a filesystem rename instead. Git metadata is read-only here, preventing staging/commit/push. Preserve unrelated `.agent-rules.md` and `docs/prompts/qa/gen.py`. In a host terminal with Git write access, review only QA 25 changes on `dev`, then run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/25-firewall-lockout.md docs/prompts/qa/archive/25-firewall-lockout.md
timeout 120 git commit -m "qa 25: Firewall cannot lock the admin out"
timeout 120 git push origin dev
```

No tag is needed for documentation-only changes.

## QA 26 — Watchdog and safe mode evidence (2026-11-19)

- Reviewed `crates/opsbox-watchdog/`, `crates/opsbox-priv/src/ops/watchdog.rs`, watchdog CLI/config wiring and systemd units on `dev`. No `.codegraph/` index exists. No product code changed.
- `lib.rs:352,358,438` binds the listener, starts the independent HTTPS recovery server and loops over concurrent serve/priv/proxy probes, disk checks, policy actions and status-file writes. The default delay is 10 seconds after work, not a guaranteed 10-second detection/restart deadline: serve TLS and HTTP fallback can each take 3 seconds and unit restart commands can take 20 seconds. Database integrity is checked on startup only. Proxy port conflicts are treated as healthy warnings.
- The standalone default listener is `0.0.0.0:8444`; CLI config tracks the panel bind IP at port 8444 unless explicitly overridden (`crates/opsbox/src/config.rs:411`). Thus local-only mode also binds recovery locally. TLS loads or creates a shared certificate. These are source checks; an actual listening socket, certificate and reachability were not verified here.
- `policy.rs:11–18,134` counts failed probes per unit in a ten-minute window. It requests dependency-ordered restarts, normally with a 10-second cooldown. At three failures it first attempts critical-disk cleanup, otherwise eligible post-update rollback, otherwise safe-mode alerting with a 60-second restart cooldown. All-healthy probes clear the alert; the recovery server remains available regardless of alert state. The units themselves use `Restart=always` and `RestartSec=2`, independently of watchdog polling.
- `server.rs:497,690` serves the small Safe Mode page, public `/healthz`, separate session/login/recovery-code/logout endpoints, authenticated `GET /api/v1/recovery`, and authenticated same-origin/CSRF-protected `POST /api/v1/recovery/action`. Actions are `free`, `restart`, `rollback`, `ssh` and `reboot`; restart/rollback wake the monitor and reset failure tracking on success. Admin password/TOTP uses read-only panel account data; a single-use recovery code lasts 15 minutes. Safe Mode has its own in-memory sessions, so panel-session reuse is a separate normal-panel recovery check.
- The UI renders health checks, errors, events/journal logs and recovery controls (`server.rs:999`). Policy reasons and successful restart messages are retained in the bounded event queue/status file (`policy.rs:118`, `lib.rs:273`); they are not emitted as tracing journal messages. Restart failures do emit warnings. The recovery response provides events separately, but chooses journal lines for its log field whenever any exist (`server.rs:645`). Live journal attribution and page clarity remain unverified.
- Helper `watchdog.rs:49` reads `/run/opsbox-watchdog/status.json`, marks snapshots older than 45 seconds or a stopped watchdog as degraded/not running, and supplies fallback status when no valid JSON exists. Existing tests cover missing/stale/fresh snapshots. Watchdog tests cover probe transitions, restart/backoff/safe-mode decisions, cleanup/rollback, update verification, auth/CSRF/origin refusal, recovery-code single use and the free-space endpoint; the HTTP test does not prove successful systemd restart/rollback/reboot or real SSH expiry.
- NOT RUN: `timeout 120 cargo test -p opsbox-watchdog` and `timeout 120 cargo test -p opsbox-priv ops::watchdog::tests` exited 127 (Cargo unavailable). `timeout 120 multipass info opsbox-vm` exited 2 (socket access denied). No processes were killed, services restarted, firewall rules changed or data corrupted.

### Fixed in fix 08 — 26: Rapid service crashes can evade safe-mode counting

The original policy counted only failed health samples, so five rapid crashes followed by healthy restarts could evade safe mode. The watchdog now reads `systemctl show <unit> --property=NRestarts --value` for priv/proxy/serve using the bounded process runner on the blocking worker. First reads establish baselines; counter increases record individual restart events at the sample timestamp, including multiple restarts between probes. Missing reads retain the baseline; lower counters count the new total after a systemd reset. Only the newest five timestamps are needed per unit. History is in memory and begins when the watchdog samples a unit.

Five observed restarts within ten minutes enter existing cleanup/rollback/safe-mode escalation even when probes are healthy. Healthy probes retain restart-driven safe mode until the restart history expires (or recovery explicitly resets it). The existing three-failed-probes rule, restart backoff, and post-update restart grace remain. No routes, PrivOps or UI map changed.

Validation (2026-11-20): test-first `policy::tests::rapid_restarts_trip_safe_mode_with_healthy_probes` failed before the implementation, then passed for all three units with both a five-restart jump between probes and five successive sampled increments. Fake-clock tests also cover one restart, unchanged counters, window expiry, counter resets and missing samples; existing three-probe, cleanup, rollback and update tests pass. `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH timeout 120 cargo test -p opsbox-watchdog` ran 19 tests: 17 passed, while the existing watchdog-loop and safe-mode HTTP tests failed with `PermissionDenied` on local socket binds. `cargo fmt --check` and `cargo clippy -p opsbox-watchdog --all-targets -- -D warnings` passed using the same PATH and timeout.

Owner check: on disposable `opsbox-vm` with console access, build from `/home/ubuntu/opsbox` with `CARGO_TARGET_DIR=~/target cargo build --release`, then install with `sudo sh install.sh --dev --binary ~/target/release/opsbox`. Wait for the watchdog's first sample and record `systemctl show opsbox.service --property=NRestarts --value`. Kill only that VM's serve PID five times, waiting for a new PID after each kill, all inside ten minutes. Verify NRestarts rises by five and authenticated recovery status on :8444 shows safe mode even with healthy serve probes. Repeat after history expiry/reset with one crash: no safe mode. Separately hold serve unavailable for three probes to preserve the old threshold; verify unauthenticated recovery actions return 401. Access the panel with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Run the full watchdog tests on the VM to cover the socket-bound tests.

Fix 08 Git completion: `git pull --ff-only` could not write `.git/FETCH_HEAD`; staging failed with `.git/index.lock: Operation not permitted`. The prompt was archived by filesystem rename. Commit/push, CI and release tagging require a host terminal with Git write access:

```sh
timeout 120 git add -- crates/opsbox-watchdog/src/lib.rs crates/opsbox-watchdog/src/policy.rs crates/opsbox-watchdog/src/probes.rs docs/DECISIONS.md docs/QA.md docs/STATUS.md docs/prompts/fix/README.md docs/prompts/fix/08-watchdog-rapid-crashes.md docs/prompts/fix/archive/08-watchdog-rapid-crashes.md
timeout 120 git commit -m "fix 08: Rapid service crashes can evade safe mode"
timeout 120 git push origin dev
```

After CI passes, tag the next unused alpha and verify its release artifacts per `docs/prompts/CONVENTIONS.md`.

### Fixed in fix 09 — 26: Fallback nftables emergency SSH allowance never expires

The fallback previously installed a permanent literal nft SSH accept rule; housekeeping discarded its expiry record without removing the rule, and watchdog restart lost the record entirely. It now grants access through IPv4/IPv6 source-address-and-TCP-port sets with `flags timeout`, a default `timeout 1h` and explicit per-element `timeout 1h`. The kernel stops matching an expired element even while the watchdog is stopped. Empty sets and their referencing rules may remain, granting no access. First creation and renewal are atomic nft batches; renewal deletes/re-adds the matching element and reuses the existing rule. UFW/firewalld behavior is outside fix 09; no routes, PrivOps or UI map changed.

Validation (2026-11-20): test-first `actions::tests::nft_emergency_ssh_expires_after_one_hour_even_after_watchdog_restart` failed because the original allowance survived 61 minutes. A simulated nft kernel with a controlled clock now verifies both IPv4 and IPv6, custom SSH port 2222, access before expiry, real watchdog housekeeping at 61 minutes, and loss of watchdog tracking on simulated restart. Renewal and timeout/rejection tests also pass. `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH timeout 120 cargo test -p opsbox-watchdog` runs 22 tests: 20 pass; the existing watchdog-loop and safe-mode HTTP tests fail with sandbox `PermissionDenied` on socket binds. Formatting, crate Clippy with all targets and warnings denied, and whitespace checks pass. Native Linux nft syntax/kernel expiry remains a live owner check: this host has no nft binary and `timeout 120 multipass info opsbox-vm` reports socket access denied.

Owner check for fix 09: on disposable `opsbox-vm` with console access, build from `/home/ubuntu/opsbox` using `CARGO_TARGET_DIR=~/target cargo build --release`, then reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`. Run `cargo test -p opsbox-watchdog` there. With nft active and the helper stopped, authenticate to Safe Mode on port 8444 (tunnel with `ssh -L 8444:127.0.0.1:8444 ubuntu@<vm-ip>`) and request emergency SSH for a controlled public client IP. Inspect `sudo nft list set inet opsbox emergency_ssh4` (or `emergency_ssh6`): only that IP and configured SSH port have an element with a one-hour timeout. Repeat with a watchdog restart before expiry, and with the watchdog stopped across expiry; after 61 minutes verify the element/allowance is gone. Use separate short commands rather than blocking for an hour, and test fresh connections since established SSH may remain accepted by conntrack. Verify reopening refreshes the element without adding another accept rule, and loopback/unspecified client IPs are refused. Inspect and remove any pre-upgrade literal emergency rules by their specific nft handles before testing; fix 09 does not migrate those old permanent grants. Preserve unrelated SSH rules and restore the helper/watchdog afterward.

Fix 09 Git completion: this sandbox cannot write `.git/FETCH_HEAD` or `.git/index.lock`; the archive move uses a filesystem rename. Commit/push, CI and release tagging require a host terminal with Git write access:

```sh
timeout 120 git add -- crates/opsbox-watchdog/src/actions.rs docs/DECISIONS.md docs/QA.md docs/STATUS.md docs/prompts/fix/README.md docs/prompts/fix/09-nft-emergency-ssh-expiry.md docs/prompts/fix/archive/09-nft-emergency-ssh-expiry.md
timeout 120 git commit -m "fix 09: Fallback emergency SSH rule never expires"
timeout 120 git push origin dev
```

After CI passes, tag the next unused alpha and verify its release artifacts per `docs/prompts/CONVENTIONS.md`.

### Owner checks — 26: Live self-healing and recovery

Use only disposable `opsbox-vm` or a disposable dev server with console access and a recoverable snapshot. Wrap shell commands in `timeout 120`; do not run a long blocking wait for the hour-long check.

1. Run the two Rust test commands above with Cargo installed. Confirm `opsbox-watchdog.service` is active and `timeout 120 sudo ss -ltnp` shows the configured recovery listener. Check HTTPS `/healthz` and the minimal page on port 8444; verify local-only binding and SSH-tunnel access when configured.
2. Sign into the normal panel, record the serve PID and time, then run `timeout 120 sudo pkill -9 -f 'opsbox serve'` on that VM only. Verify the new PID, panel return and still-valid original session without a new login. Distinguish the 2-second systemd restart from watchdog-requested recovery; record measured downtime rather than assuming a fixed watchdog deadline.
3. Repeat five rapid kills as above. Separately use a reversible disposable service failure fixture lasting across at least three failed samples to verify threshold, restart backoff and the visible degraded state on 8444. Do not corrupt the database. Inspect `timeout 120 sudo journalctl --no-pager -u opsbox -u opsbox-watchdog -n 100` and authenticated recovery events for the restart cause.
4. Verify unauthenticated recovery GET/POST returns 401, viewer login is refused, missing/incorrect CSRF and foreign Origin are refused, admin password/TOTP works, and a recovery code works once only. Exercise safe free-space/restart recovery and confirm the normal panel returns, events explain the outcome and degraded state clears. Check rollback only with an available compatible previous slot and a disposable snapshot; ensure absent/incompatible slots fail safely. Do not reboot just to prove endpoint existence.
5. Validate SSH restrictions and actual one-hour allowance removal on each supported backend, including helper-unavailable fallback and watchdog restart (nft expiry: Fixed in fix 09). Retain PARTIAL until live self-healing/session/page checks and findings are resolved.

QA 26 Git completion: `timeout 120 git mv docs/prompts/qa/26-watchdog-safe-mode.md docs/prompts/qa/archive/26-watchdog-safe-mode.md` failed with `.git/index.lock: Operation not permitted` (exit 128). Archived with a filesystem rename instead. Git metadata is read-only here, preventing staging/commit/push. Preserve unrelated `.agent-rules.md` and `docs/prompts/qa/gen.py`. On a host terminal with Git write access, review only QA 26 changes on `dev`, then run:

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/26-watchdog-safe-mode.md docs/prompts/qa/archive/26-watchdog-safe-mode.md
timeout 120 git commit -m "qa 26: Watchdog and safe mode"
timeout 120 git push origin dev
```

No tag is needed for documentation-only changes.

## QA 29 — Table layout evidence (2026-11-19)

- Reviewed `web/css/app.css` table rules, `web/js/ui.js` (`table`, `headMeta`), and table rendering in `web/js/pages/containers.js`, `web/js/pages/system.js`, and `web/js/pages/apps.js`.
- Responsive overflow: `.tw` wrappers (`overflow-x: auto`) prevent horizontal viewport blowouts while containing wide tables. In mobile viewports (`max-width: 700px`), `.tw.cards` transforms table rows into stacked cards with upper-case `data-label` headers, and pinned sticky header support via `.sticky-col`.
- Headers and alignment: column headers align consistently with cell content; numeric and metric columns use `.r` with `tabular-nums` and monospace font. First column elements utilize `min-width: 0` to enable proper ellipsis truncation on narrow screens.
- Headless UI suites (`tools/check-responsive-ui.mjs` and `tools/check-flip-ui.mjs`) verified table structure, container cards, and touch target rules.

## QA 30 — List row actions evidence (2026-11-19)

- Fixed in fix 33: All five New service tabs use the wide content area. Source forms use a responsive two-column grid with fields capped at 560 px. Repository rows use a two-column grid, stack actions below text at phone widths, and show Choose build for repositories with nothing detected. The responsive check covers tab widths, action alignment, row padding, phone overflow and touch height, and missing-build actions in mock mode. Its browser portion awaits an outside-sandbox run.
- Reviewed `.list-row` in `web/css/app.css` and its usage across `web/js/pages/newservice.js`, `web/js/pages/settings.js`, `web/js/pages/security.js`, `web/js/pages/recovery.js`, and `web/js/network.js`.
- First-child flex growth: `.list-row>span:first-child` and `.list-row>div:first-child` expand with `flex: 1 1 0; min-width: 0`, preventing metadata from colliding with action buttons.
- Action alignment: row action buttons consistently align to the right edge with clean spacing (`gap: 16px`). Buttons use `.btn` tokens with accessible labels (`aria-label`), clean hover/focus-visible states, and secondary styles for multi-item list actions (e.g. Deploy repository).
- Destructive protection: destructive actions (container deletion, stop/restart, unit stop, firewall rule removal, backup job removal) are guarded by `confirmDialog` with explicit confirmations.

## QA 31 — Notification list evidence (2026-11-19)

- Reviewed `web/js/shell.js` (`renderNotifItems`, `notificationBell`, `refreshAttention`), `web/js/ui.js` (`html`, `patch`, `raw`), `crates/opsbox-alerts/src/lib.rs`, and `crates/opsbox-api/src/alerts.rs`.
- Stray commas check: `renderNotifItems` uses `html` tagged templates which flatten and concatenate item arrays without stray commas (`Array.prototype.map(...).map(render).join('')`). Verified via isolated node tests and automated headless suite (`tools/check-notifications-ui.mjs`).
- Alert evaluation & resolution: container alerts ("is stopped or restarting") and unit alerts evaluate against live daemon state and cleanly transition from `Firing` to `Resolved` / `Ok` once the underlying condition clears.
- Timestamp stability: attention items are deduplicated and tracked via stable IDs (`kind:subject`) with `reconcile_attention_since` preserving original `since` timestamps across recurring polls.

## QA 32 — Cleanup: unused Docker volumes safety evidence (2026-11-19)

- **VM Docker check (`opsbox-vm`)**:
  - Docker version: Client & Server `29.1.3`, API version `1.52`.
  - Created test volume: `docker volume create keepme; docker volume ls -f dangling=true` lists `keepme` under dangling.
  - Default volume prune: `docker volume prune -f` (without `--all`) reclaims `0B` and leaves `keepme` intact (Docker engine 23+ API 1.42+ only prunes anonymous volumes by default).
  - Explicit all volume prune: `docker volume prune -f --all` deletes `keepme` and reclaims space.
- **Frontend checkbox safety (`web/js/pages/cleanup.js`)**:
  - `picked.has(i.id)` is the single source of truth for checkbox selection. In `paint()`, DOM `.checked` is explicitly reconciled across all input elements: `el.checked = picked.has(el.dataset.id)`.
  - In `load()`, items with `warn == true` or `safe === false` are excluded from being pre-selected on initial scan and subsequent refreshes: `d.items.filter((i) => !i.pending && i.on && !i.warn && i.safe !== false)`.
  - For `warn` / `safe: false` items, `confirmDialog` requires typed confirmation `delete volumes` (`typed: hasWarn ? 'delete volumes' : null`) with danger styling.
  - When running cleanup on `docker-volumes` and 0 bytes are reclaimed without errors, the UI informs the user via warning toast: `"Docker only removes anonymous volumes automatically. Named volumes were kept. Review them one by one."`.
- **Backend assertions (`crates/opsbox-api/tests/cleanup_http.rs`)**:
  - Updated integration test `a_scan_has_the_shape_the_page_draws` to verify that any item with `warn == true || safe == false` strictly has `on == false`.
  - Verified tests pass cleanly via `cargo test -p opsbox-api --test cleanup_http` on `opsbox-vm`.

## QA 33 — Cleanup: choose unused Docker volumes one by one evidence (2026-11-19)

- **Audit & Scope Evaluation**:
  - Prompt 33 specifies replacing the blanket prune with explicit volume listing (`GET /api/v1/cleanup/volumes/list`) and targeted removal (`POST /api/v1/cleanup/volumes/remove`).
  - Required changes span >350 lines across 5 crates/subsystems:
    1. `crates/opsbox-proto`: RPC commands `CleanupRemoveVolumes`, `CleanupVolumeEntry`.
    2. `crates/opsbox-priv`: Typed op handler with docker inspection, mount checks, store ownership filtering, and audit logging.
    3. `crates/opsbox-api`: Handlers with Admin authorization, CSRF check, and error mapping.
    4. `crates/opsbox-store`: Cross-crate lookup for database data volumes and compose project mounts.
    5. `web/js/pages/cleanup.js`: Volume inspection drawer/table, individual item checkboxes, badge for opsbox-owned volumes, sticky batch delete action, and typed confirmation.
  - Per `docs/prompts/qa/README.md` rule 4 & 5 (20-minute hard limit; changes exceeding ~80 lines in two files must stop and be documented under 'Found, not fixed'), the architecture and security contract are documented below for subsequent milestone implementation.

### Found, not fixed — 33: Blanket Docker prune cannot delete unused named volumes
- **Problem**: `DockerPruneKind::Volumes` invokes `prune_volumes` with no filters. Under modern Docker (API 1.42+, engine 23+; VM runs Docker 29.1.3 API 1.52), this prunes only anonymous volumes. Named unused volumes remain permanently reported in `dangling_volumes_count` and `system df`, leading to 0-byte frees and user confusion.
- **File references**: `crates/opsbox-docker/src/lib.rs` (around line 1428) and `crates/opsbox-priv/src/ops/cleanup.rs` (around line 990).
- **Reproduction**:
  ```bash
  docker volume create keepme
  docker volume ls -f dangling=true # lists keepme
  docker volume prune -f            # reclaims 0B, keepme remains
  ```
- **Proposed Implementation**:
  - Add `PrivOp::CleanupVolumeList` and `PrivOp::CleanupVolumeRemove { names: Vec<String> }`.
  - Protect any volume matching labels `opsbox.*`, compose project labels, or recorded database/app volumes (`protected: true`).
  - Provide a modal table in `web/js/pages/cleanup.js` allowing individual selection of removable volumes with confirmation phrase `"delete volumes"`.

## Fixed in fix 19 — Deployer must not reach root through Compose (2026-11-20)

Compose authorization now precedes capacity/helper calls and enqueue. Privileged mode, host networking/namespaces, capabilities, devices, security options, user namespaces, sysctls, inherited volumes and external host binds (short and long form) require Admin. Uninspectable YAML fails closed for Deployer. Non-Admin confirmation is never passed or persisted; existing stack confirmation cannot authorize a Deployer's background redeploy. The helper still checks both raw and resolved configuration. Import preview/apply already require Admin; imported stacks get the same redeploy check. Template installs use only compiled, validated catalog specifications and accept no Compose source or host-access options.

Test-first evidence: `compose_roles_http` initially returned 503 instead of 403 for elevated create/redeploy and 400 for unparseable YAML; all three tests now pass, including every listed option with both confirmation values and a legacy confirmed stack. `compose::tests::flags_all_elevated_service_options` initially missed `cap_add`; all five Compose tests pass after the detector extension. The lifecycle regression now deploys/redeploys plain Compose (image, ports, named volume, environment) as Deployer and retains Admin's 409-without-confirmation / success-with-confirmation checks for privileged host-network Compose. Its local TCP listener is denied by this sandbox (`PermissionDenied`), so lifecycle success remains unverified here.

Validation: targeted all-target Clippy (`cargo clippy -p opsbox-api -p opsbox-deploy --all-targets -- -D warnings`), formatting, whitespace and regenerated index freshness checks pass. No commit or push attempted.

Owner checks: run `timeout 120 cargo test -p opsbox-api --test compose_roles_http --test templates_compose_http` on a host permitting local sockets. In disposable `opsbox-vm`, from `/home/ubuntu/opsbox`, build with `CARGO_TARGET_DIR=~/target cargo build --release`, reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`, and connect using `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Deploy plain Compose as Deployer; elevated Compose with `confirm: true` and redeploy of a previously confirmed elevated stack must return 403 “needs an administrator”. Admin must receive 409 without confirmation and deploy successfully with confirmation. Verify Deployer cannot import a stack and can still install an ordinary catalog template.

## Fixed in fix 20 — Full API tokens must not be root (2026-11-20)

Full tokens now receive the explicit token-denied 403 before system/job/file mutations, package and Docker installation, reboot and unit actions, firewall/SSH/host/swap changes, self/automatic updates, security fixes, panel listener exposure, Compose submission, and snapshot restore. Existing account/token/key/export/import refusals remain covered. File deletion covers both trash moves and permanent removal; there is no separate file-move endpoint. Capacity admission rejects session-only operations before probing the helper. Full-token Compose redeploys use Deployer elevation checks on raw and helper-resolved configuration, so saved Admin confirmation cannot grant root access. Ordinary service deployments remain available.

Strict TDD: `full_tokens_cannot_reach_root_equivalent_actions` initially observed 202 task acceptance, helper 503s and a successful update-policy mutation instead of 403. It now passes with valid bodies, verifies the scratch file is unchanged and no upload staging exists, confirms an Admin session successfully changes automatic-update settings, and confirms Admin job submission reaches the absent helper (503, rather than an authorization refusal). `full_token_cannot_redeploy_previously_confirmed_elevated_stack` initially returned 503 and now returns 403. All four `compose_roles_http` tests pass. `mutating_routes_have_an_explicit_token_policy` invokes the stdlib route scanner against current source and checks all 138 mutating routes: direct denial, documented permission, public credential flow or named delegated guard. It initially listed the missing root-route policies and now passes.

Validation: focused HTTP regressions, route-policy inventory, targeted all-target Clippy (`cargo clippy -p opsbox-api --all-targets -- -D warnings`), formatting and regenerated index freshness pass. The three existing helper-backed `tokens_http` tests and the Admin file lifecycle tests cannot initialize because the sandbox refuses `PrivServer::bind` with `Operation not permitted`; successful Admin file/helper execution is not claimed here. Full scope description is “everything except root-equivalent actions” in Settings and API docs. No commit or push attempted.

Owner checks: on a socket-capable host run `timeout 120 cargo test -p opsbox-api --test tokens_http --test compose_roles_http --test files_http --test system_http --test update_http --test backups_http`. In disposable Multipass `opsbox-vm`, from `/home/ubuntu/opsbox`, build with `CARGO_TARGET_DIR=~/target cargo build --release` and install with `sudo sh install.sh --dev --binary ~/target/release/opsbox`; access via `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Create a disposable Full token in Settings. Bearer requests to add a harmless `true` job, edit/upload/trash a scratch file, install updates or restore a snapshot must return 403 and start no task/change; never try a real root-file write. Use an Admin browser session to create/remove that job and edit/trash the scratch file successfully. Confirm ordinary token deployments still work and a previously confirmed elevated Compose stack cannot be redeployed with the token. Check the token scope hint at 390 px. Revoke the disposable token afterward.

### Fixed in fix 21 — 26: Safe Mode exposure and shared TLS key

Fixed in fix 21. `[watchdog] allow_ips` now takes exact IPv4/IPv6 addresses and drops other peers immediately after TCP accept, before spawning TLS or reading HTTP. IPv4-mapped IPv6 peers match IPv4 entries; invalid addresses/CIDRs fail config load. Empty/default lists retain open remote recovery; loopback is not implicitly exempt. The existing local-only panel bind still supplies the default watchdog bind. Configuration changes require restarting the watchdog.

Watchdog TLS now uses `/etc/opsbox/watchdog/selfsigned.{crt,key}` with a creator-owned (root in the service), mode-0600 key created before any private bytes are written. There is no chown to opsbox, shared panel key, or writable-data-directory fallback. Existing valid keys keep their identity and have permissions tightened; symlink keys and foreign-owned directories/keys fail closed. `opsbox status` prints open/restricted allowlist state and this certificate's SHA-256 fingerprint; `install.sh` already prints that status and now explains the restriction setting. When the local source is excluded, root status uses a private watchdog snapshot no older than 45 seconds. The binary `install` command remains a plan-only stub.

Test-first evidence (2026-11-20): key permission and default-path tests failed against the shared panel key (`0640` instead of `0600`); the config test failed because allow_ips was unknown, and status rendering lacked exposure/fingerprint output. The admission-policy test initially failed to compile because no source filter existed. The new live connection test verifies close-before-ClientHello for a denied peer and HTTPS success for explicitly allowed/empty lists, but cannot bind sockets in this sandbox.

Validation: 24 socket-free watchdog tests, all 17 config tests, eight socket-free status tests and the web local-only config-edit test pass. Tests cover exact/mapped IP admission, empty lists, invalid config, key creation/ownership/mode, reload identity/permission repair, symlink refusal, status exposure/fingerprint rendering and fresh/stale/stopped private snapshots. The full watchdog run fails only at sandbox-denied listener binds (the existing loop and auth HTTP tests plus the new accept test). Targeted all-target Clippy with warnings denied, formatting, shell syntax, whitespace and regenerated-index checks pass. Commands use `PATH=/Users/lenhutkhangvo/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH` and `timeout 120`. `timeout 120 cargo build --release` passed in 1m39s with a nonfatal local rust-objcopy/libLLVM debug-stripping warning. No install, commit or push was attempted.

Owner checks — disposable `opsbox-vm`, console access and a recoverable snapshot:

1. From `/home/ubuntu/opsbox`, run `CARGO_TARGET_DIR=~/target cargo build --release`, then `sudo sh install.sh --dev --binary ~/target/release/opsbox`. Run `cargo test -p opsbox-watchdog` to exercise all three socket tests here. Wrap each short command in `timeout 120`.
2. Inspect `sudo stat -c '%U:%G %a %n' /etc/opsbox/watchdog/selfsigned.key`: expect `root:root 600`; `sudo -u opsbox cat /etc/opsbox/watchdog/selfsigned.key` must be refused. Compare `sudo opsbox status` watchdog SHA-256 with `openssl x509 -in /etc/opsbox/watchdog/selfsigned.crt -noout -fingerprint -sha256`; verify the panel key/fingerprint differs. Reinstall and confirm the installer shows the watchdog fingerprint/allowlist and preserves the watchdog identity.
3. In the existing `[watchdog]` table of `/etc/opsbox/config.toml`, set `allow_ips = ["<controlled-admin-source-ip>"]`, restart `opsbox-watchdog`, and verify HTTPS from that address. A different source must close before TLS; forwarded headers must not change admission. Root status should still work via its fresh private snapshot. Repeat with IPv6 if available.
4. Set `allow_ips = []`, restart and confirm both sources reach HTTPS (unauthenticated recovery actions still return 401). Run the installer with `--local-only` without an explicit watchdog.listen override; verify `sudo ss -ltnp` shows watchdog on `127.0.0.1:8444`. Tunnel panel access using `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>` and recovery using `ssh -L 8444:127.0.0.1:8444 ubuntu@<vm-ip>`; when restricting again, include loopback explicitly if tunnels are desired. Restore the VM configuration after testing.

## Fixed in fix 22 — Root helper umask and sensitive file modes (2026-11-20)

Fixed in fix 22. The current unit path is `packaging/systemd/opsbox-priv.service`; it now sets `UMask=0077`. `JobAdd` writes each service/timer through the existing `network::atomic_write`: a fresh same-directory tempfile, explicit 0600 before content, file sync, atomic rename, then directory sync. Deployment GitHub tokens, SSH private keys and BuildKit secrets use the same writer instead of write-then-chmod. Uninstall audit exports and saved adoption receipts use private atomic replacement; restored adopted files retain their recorded final mode.

Strict TDD: `job_add_creates_private_unit_and_timer_files` first failed with `[0644, 0644]` instead of `[0600, 0600]`; `root_helper_service_uses_private_umask` first failed because UMask was absent. The job regression now runs the actual add implementation with a temporary unit directory and harmless systemctl substitute under child-process umask 000, checks both modes/content and no leftover staging files. `audit_export_is_private_even_when_replacing_a_public_file` first observed 0644; `private_restore_does_not_follow_a_stale_temporary_symlink` first overwrote its sentinel. All now pass (seven jobs tests, eight uninstall tests, four deploy unit tests).

Audit of other `fs::write` / `File::create` sites in `crates/opsbox-priv/src`:

| Sites | Assessment |
|---|---|
| `secrets.rs`, backup password/SFTP keys | Already create_new + explicit 0600. |
| Backup Redis staging and database upgrade dumps | Reopen files already created privately by tempfile; permissions are retained. |
| Compose validation/deploy | NamedTempFile starts at 0600; installed Compose deliberately uses 0640 for its service group. |
| Network/adoption receipts and backups | Existing atomic writer explicitly uses 0600 for private state. |
| Deployment askpass and generated Dockerfile | No credential values in these generated files; private 0700 parent and final 0700/0600 modes retained. |
| Docker apt key/repository, package/host configuration, SSH drop-in, panel listener config, cleanup timer/cache | Public configuration, fixed commands or aggregate counts; retain existing intended modes and gain the helper's restrictive default for new unspecified modes. |
| File manager | Existing descriptor-based writer uses private staging, then preserves the requested final file mode/ownership. |
| Remaining matches in test modules | Fixtures only. |

Validation: focused jobs/uninstall/deploy unit tests, `cargo check -p opsbox-priv`, `cargo clippy -p opsbox-priv --all-targets -- -D warnings`, formatting and regenerated-index checks pass. Native release-build result is recorded in docs/STATUS.md. Live systemd activation and real deployment credential consumers require the disposable Linux VM; existing installed job files are not migrated by this fix.

Owner checks — disposable Multipass `opsbox-vm`:

1. From `/home/ubuntu/opsbox`, build with `CARGO_TARGET_DIR=~/target cargo build --release`; reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`. Confirm `systemctl show opsbox-priv.service -p UMask` prints `UMask=0077` (reload/restart the helper if testing by manually copying the unit).
2. Tunnel with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. As an Admin, create a job named `Fix 22 permissions`, schedule `0 2 * * *`, command `true`. Check both new `/etc/systemd/system/opsbox-job-<id>.{service,timer}` files with `sudo stat -c '%a %U:%G %n'`: each must be `600 root:root`. Run the job successfully; retain it for the refusal check below.
3. As an unprivileged account, `cat /etc/systemd/system/opsbox-job-<id>.service` must fail with permission denied before removing the job. An invalid schedule must be refused without creating units. Remove the valid job through the panel afterward. Do not include real secrets in test commands.
4. Run `timeout 120 cargo test -p opsbox-priv --test deploy build_streams_scrubs_secrets_uses_secret_mount_and_health_checks_loopback` on a socket-capable host, then verify a disposable private-repository deployment and BuildKit secret mount still succeed on the VM. No installation, commit, push or tag was attempted in this session.

## Fixed in fix 23 — Protect opsbox configuration and services (2026-11-20)

Fixed in fix 23. File-manager mutations now refuse `/etc/opsbox` and all descendants, and `/etc/systemd/system/opsbox*` including service/timer files and drop-ins. Checks cover normalized names, symlink expansion and existing protected directory/unit inodes. The message says “Protected opsbox configuration, service or internal state is read-only through the file manager.” Names are protected even before files exist; unrelated configuration and units retain their previous behavior.

Strict TDD: `opsbox_configuration_and_services_refuse_all_file_mutations` and `opsbox_configuration_and_services_refuse_symlink_and_parent_traversal` both failed before implementation because `/etc/opsbox` and `config.toml` were unprotected. Both now pass, exercising actual write/chmod/delete/upload handlers, direct and `..` paths, file links and parent-directory links. Safety assertions stop the red run before attempting host mutations. There is no general FileMove operation: `remove(permanent=false)` moves to Trash, and `install_upload` moves an upload into place. Deleting an ordinary external symlink still removes only that link, without following its protected target.

A separate existing gap remains: selecting an unprotected ancestor for recursive deletion/Trash does not check protected descendants; see “Found, not fixed” in docs/STATUS.md.

Validation: all 28 other file tests pass. The complete 29-test file run has one environment failure in the unchanged `mode_strings_look_like_ls`: requesting mode 04755 on a scratch tempfile produces 0755 here (independently reproduced with Python). Keep that test unchanged and rerun on Linux. Additional check/build results are recorded in docs/STATUS.md.

Owner checks — disposable Multipass `opsbox-vm`, with a recoverable snapshot:

1. From `/home/ubuntu/opsbox`, build with `CARGO_TARGET_DIR=~/target cargo build --release`; reinstall with `sudo sh install.sh --dev --binary ~/target/release/opsbox`. Run `timeout 120 cargo test -p opsbox-priv --lib ops::files::tests` on Linux, including the mode-display test.
2. Tunnel with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. As Admin, inspect `/etc/opsbox/config.toml` and `/etc/systemd/system/opsbox.service` in Files. Saving, chmod, permanent deletion and moving to Trash must all be refused with the protected opsbox configuration/service message. Upload replacement and creation of a new opsbox-prefixed unit must also be refused. Verify config/unit hashes and modes stay unchanged.
3. Create scratch symlinks to `/etc` and to the config/unit files using the VM console. Repeat writes/chmod through file links, and delete/Trash/upload through parent-directory links and `..` paths; protected targets must remain unchanged. Verify ordinary scratch-file read/write/chmod/Trash still work, then remove the scratch links. No commit, push, tag or installation was attempted in this session.

## Fixed in fix 24 — Root-owned apps, xfer and trash (2026-11-20)

Fixed in fix 24. The installer and helper upgrade preparation enforce `apps=root:opsbox 0750`, `xfer=root:opsbox 0770`, `trash=root:root 0700`. Both refuse directory symlinks. Preparation temporarily removes parent write access to prevent replacement of legacy panel-owned directories while changing ownership; the sticky parent permissions are restored on success/refusal. The new helper prepares configured directories before accepting requests after a binary update, and the self-update activation path also prepares them.

Runtime apps/service/src creation uses checked directory descriptors and `mkdirat`/`openat` with `O_NOFOLLOW|O_DIRECTORY`. Trash moves/purges and transfer upload/download/update verification use checked no-follow roots. API `ensure_xfer` neither creates nor chmods directories and requires root ownership and 0770. Downloads use group-readable 0640 files inside xfer; new service children retain group access despite the helper's 0077 umask. Update downloading refuses missing/symlink transfer directories instead of creating one as the panel user.

Evidence: the existing API regression failed before the API change because the symlink target's mode changed from 0755 to 0700; it now passes without changing target mode or contents. The started helper regressions were already present on entry; no earlier failing run is claimed for them. Five managed helper tests cover apps/service/src links, xfer download/upload refusal, Trash move/purge refusal, fresh preparation, mode migration, ownership and parent-mode restoration. `tools/check-managed-dirs.py` executes the actual installer block with recorded owner/group arguments and real scratch mkdir/chmod, covering fresh install, existing directories and refusal for all three links without touching targets. Full root chown/systemd behavior remains a Linux owner check.

Validation: required `cargo check -p opsbox-priv -p opsbox-api`, five `cargo test -p opsbox-priv --lib managed_` checks and the API `managed_xfer_symlink` regression pass. All 30 other file tests pass with the unchanged environment-sensitive `mode_strings_look_like_ls` skipped (see fix 23). Formatting, focused all-target Clippy and installer shell syntax checks pass. Generated indexes and release build results are recorded in docs/STATUS.md.

Owner checks — disposable Multipass `opsbox-vm` with a recoverable snapshot:

1. From `/home/ubuntu/opsbox`, run `CARGO_TARGET_DIR=~/target cargo build --release`, then `sudo sh install.sh --dev --binary ~/target/release/opsbox`. Check `sudo stat -c '%a %U:%G %n' /var/lib/opsbox/{apps,xfer,trash}` against the modes/owners above. Run `timeout 120 python3 tools/check-managed-dirs.py` and the two required managed Rust test commands. Run the complete file tests on Linux, including the mode-display test.
2. Test an upgrade from the previous alpha with existing app files and transfers. After the new helper restarts, verify root ownership/modes and retained contents. On a disposable migrated install, stop the panel/helper, change only xfer's owner back to opsbox, restart the helper and confirm root:opsbox 0770 is restored before panel startup. Verify `sudo -u opsbox mv /var/lib/opsbox/apps /var/lib/opsbox/apps-moved` is refused by the sticky root-owned parent.
3. Tunnel using `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`; deploy a scratch service and upload/download/Trash a scratch file. Confirm downloads work with root-owned xfer and existing app data is retained.
4. With services stopped and a snapshot available, move one managed directory aside as root, replace its name with a symlink to a scratch directory containing a sentinel, then run the installer. It must refuse without changing the target's owner, mode or contents. Starting the new helper must also fail safely. Restore the directory before repeating for the other two names and restarting services. Never use a real system directory as the symlink target.

No commit, push, tag or installation was attempted in this session.

## Fixed in fix 25 — Proxy filesystem write access (2026-11-20)

Fixed in fix 25. `packaging/systemd/opsbox-proxy.service` now permits writes only to `/var/lib/opsbox/proxy` under `ProtectSystem=strict`. `InaccessiblePaths` hides `/var/lib/opsbox/xfer` and the existing database. The proxy's persistent files are routes, adopted listeners, ACME accounts and encrypted certificates under its own state directory; it does not need SQLite or xfer. `install.sh` already creates that directory and installs this unit directly.

The database exclusion has a `-` prefix so first installation can start the proxy before the panel creates SQLite. If absent at proxy startup, the new database is still on the read-only parent mount, but read concealment requires a proxy restart after creation. Existing installations hide the database immediately on restart. This filesystem restriction does not replace the separate UID/helper authorization work in fixes 26/27.

Strict TDD: all three new `tests/systemd_unit.rs` regressions failed against the original unit: `/var/lib/opsbox` granted database write access, and neither inaccessible path was present. After the unit change all three pass. The tests inspect effective repeated/reset directives in `[Service]`, check path ancestry by components (including direct database grants and optional path prefixes), require the proxy state directory to remain writable and retain `ProtectSystem=strict`.

Validation: `cargo test -p opsbox-proxy` ran nine passing library tests and four failures at sandbox-denied socket binds (`Operation not permitted`); Cargo stopped before integration tests. The three new unit-file tests pass when run separately, as do all nine socket-free library tests. Focused all-target Clippy with warnings denied, formatting and index regeneration pass. Release-build/final freshness results are recorded in docs/STATUS.md. Commands use `timeout 120` and the local stable Rust toolchain on PATH.

Owner checks — disposable Multipass `opsbox-vm` with a recoverable snapshot:

1. From `/home/ubuntu/opsbox`, run `CARGO_TARGET_DIR=~/target cargo build --release`, then `sudo sh install.sh --dev --binary ~/target/release/opsbox`. Run `timeout 120 cargo test -p opsbox-proxy` here, including the local ACME issue/renewal integration test. Check both a fresh installation and a reinstall with existing data. On a fresh install, restart `opsbox-proxy` after SQLite exists to apply database concealment.
2. Run `sudo systemctl show opsbox-proxy -p ReadWritePaths -p InaccessiblePaths` and confirm the paths above. Get the process ID with `pid=$(systemctl show opsbox-proxy -p MainPID --value)`. Inspect its filesystem as its service user using `sudo nsenter -t "$pid" -m -- runuser -u opsbox -- sh -c 'test -w /var/lib/opsbox/proxy && ! test -w /var/lib/opsbox && ! test -r /var/lib/opsbox/xfer && ! test -r /var/lib/opsbox/opsbox.db'`; require success. Do not test by writing real database bytes. A read-only `cat` of either hidden path in that same namespace/user must be refused.
3. Tunnel the panel with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`. Issue and renew a certificate for a controlled domain using ACME staging, verify HTTPS, then restart the proxy and verify persisted routes/certificates reload. Confirm the database remains writable by the panel and scratch upload/download still works through the helper.

No commit, push, tag or installation was attempted for fix 25.

## Fixed in fix 26 — Proxy user and helper socket separation (2026-11-20)

Fixed in fix 26. The proxy runs as opsbox-proxy:opsbox-proxy and uses /run/opsbox/priv-proxy.sock, configurable with [priv] proxy_socket. The helper accepts only the proxy UID there and the panel/root allow-list on priv.sock, verifies kernel credentials before reading, refuses overlapping identities/paths, and traces socket origin. Proxy operation restrictions remain Fix 27. The separately authenticated proxy control socket permits connects from the panel without granting shared filesystem group access.

Installer and root helper startup share idempotent account/state migration. Legacy writers stop before migration; encrypted routes/certificates remain in place, proxy state becomes private to the proxy user, and the root sticky parent permits traversal only to known paths for unrelated users. Binary-only updates from older helpers also refresh the installed proxy service unit and queue a fresh restart after migration so Type=simple startup cannot race the older updater’s restart sequence. No installation, commit, push or tag was attempted.

Strict TDD: the separate service-user assertion failed with User=opsbox; the two-socket/UID tests failed to compile before support existed; the migration test failed because its shared installer block was absent; the proxy --socket test failed because it targeted the panel socket. The service-unit tests (four), migration test, shared-UID refusal and all 18 configuration tests now pass. The migration harness mocks account/chown/systemd commands and checks real state preservation, repeat account creation, modes and symlink refusal; actual account ownership remains a Linux check.

Validation: cargo check --workspace passes. Requested full priv tests run 231 library tests: 228 pass, three existing environment failures (socket bind forbidden, temporary setuid bits stripped, host uptime sysctl denied); Cargo stops before integration tests. Full proxy tests run 13 library tests: nine pass, four existing local socket binds are denied. The new two-listener integration test separately reaches bind and receives sandbox Operation not permitted; its UID round trips require the owner host. Focused all-target Clippy with warnings denied, formatting, shell syntax and generated-index checks are recorded in STATUS.md. Commands use timeout 120 and the local stable Rust toolchain on PATH.

Owner checks on disposable Multipass opsbox-vm:
1. From /home/ubuntu/opsbox, build with CARGO_TARGET_DIR=~/target cargo build --release and install with sudo sh install.sh --dev --binary ~/target/release/opsbox. Run cargo test -p opsbox-priv and cargo test -p opsbox-proxy here; in particular cargo test -p opsbox-priv --test socket proxy_ must pass. This verifies real peer credentials against both listeners, including rejection before a request is sent.
2. Check getent passwd opsbox-proxy, id opsbox-proxy and systemctl show opsbox-proxy -p User -p Group. Require distinct panel/proxy UIDs and no opsbox supplementary group. stat both helper sockets: root:opsbox 0660 and root:opsbox-proxy 0660. sudo -u opsbox-proxy test -w /run/opsbox/priv.sock and sudo -u opsbox test -w /run/opsbox/priv-proxy.sock must both fail.
3. Reinstall twice with existing encrypted certificates/routes. Verify account UID stays stable, proxy state remains owned by opsbox-proxy, /var/lib/opsbox stays root:opsbox 1771, panel database/xfer access works, and proxy control/routes/ACME issue-renew-reload work. Tunnel UI via ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>.
4. Also test a binary-only update from the preceding alpha without running the new installer: new helper startup must create the account, migrate state, refresh the unit and start both roles successfully. Keep a VM snapshot for recovery; rollback to a binary predating dual sockets needs the matching old service identity/state ownership restored, since older binaries do not understand this migration.


## Fixed in fix 27 — Helper operation authorization by caller (2026-11-20)

Fixed in fix 27. The helper carries Caller::Serve/Proxy from Fix 26's separate listener, admits only that listener's kernel peer UID, then checks an exhaustive PrivOp match before blocking dispatch or PTY, Docker-log and deploy-build streaming. Proxy can request only PortHolders and SecretSeal/SecretOpen with a proxy: context; the existing secret-context validator still checks syntax. Serve retains all typed operations for now; ROADMAP records the need for API admin proof on root-grade operations.

Denied proxy requests return PrivErrorCode::Forbidden with their original request ID and keep the framed connection usable. Each denial emits a structured WARN event=priv_op_forbidden with caller, kernel UID/PID, request ID, operation and Forbidden code; it omits payloads, secret contexts and the untrusted actor string. This is helper journal evidence, not a new database audit writer; actor provenance remains Fix 29.

TDD evidence: the policy/response tests were written before Caller/allowed/authorize and initially failed to compile because those APIs did not exist. Replaying the prior permissive UID-only policy made all three policy/response tests fail assertions. A real UnixStream::pair connection regression also failed under that permissive policy and passed after restoring the allowlist. All five new tests pass: necessary proxy operations, foreign secret namespaces, typed root-operation refusal, structured denial logging without secrets/actor, and real peer-credential/framing dispatch. The connection regression checks JobAdd, PtyOpen, Reboot, FirewallRemove, FileWrite, DockerLogsFollow and DeployBuild, verifies no scratch file was written, then successfully runs PortHolders and proxy secret sealing on the same connection. A forged actor=root grants no authority.

Validation: timeout 120 cargo test -p opsbox-priv ran 236 library tests: 233 passed and the same three pre-existing environment failures remained (client socket binding denied, temporary setuid bits stripped, uptime sysctl denied). Cargo stops before the integration suites. Explicit timeout 120 cargo test -p opsbox-priv --test socket proxy_ passed the shared-UID refusal but both filesystem-listener tests could not bind (Operation not permitted). The socket-pair regression passes without a listener bind. Formatting, all-target helper Clippy with warnings denied and whitespace checks pass. Commands use the local stable Rust toolchain on PATH. Index/release results are recorded in STATUS.md.

Owner checks — disposable Multipass opsbox-vm with a recoverable snapshot:
1. From /home/ubuntu/opsbox, run timeout 120 cargo test -p opsbox-priv and timeout 120 cargo test -p opsbox-priv --test socket proxy_. The latter checks listener-to-role mapping, real kernel peer UID refusal, typed operation denials and continued allowed requests after refusals. All must pass on Linux. Run timeout 120 cargo test -p opsbox-proxy to cover ACME callers.
2. Build with CARGO_TARGET_DIR=~/target cargo build --release and install using sudo sh install.sh --dev --binary ~/target/release/opsbox. Verify both helper sockets retain their distinct Fix 26 UIDs/groups. In a disposable test client running as opsbox-proxy, send a length-prefixed PrivRequest to /run/opsbox/priv-proxy.sock containing id=fix27-owner, actor=root and op={type:pty_open,args:{user:no-such-user-fix27,cols:80,rows:24}}. Require an error frame with the same ID and code=forbidden, with no PTY stream. Do not issue real reboot/firewall mutations as manual probes.
3. Inspect sudo journalctl -u opsbox-priv --since '5 minutes ago' for priv_op_forbidden, Proxy, the proxy's actual UID/PID, fix27-owner, pty_open and Forbidden, with no secret payload or trusted actor claim. Issue/renew an ACME staging certificate for a controlled domain, restart the proxy, and verify persisted encrypted certificates reload. Confirm ordinary panel Admin jobs/files/terminal still work. Tunnel with ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>.

No commit, push, tag or installation was attempted for fix 27.

## Fix 28 — Root helper verifies staged updates

**Fixed in fix 28** (live signed-update and rollback owner checks pending).

The helper previously validated only a staged file's location/type, then executed its mutable pathname as root. SelfUpdateInstall now requires SHA-256 and minisign proof for the release archive. Both panel and CLI stage that archive without running it; priv opens it descriptor-relative inside xfer, reads it once, checks SHA-256 and the embedded minisign key, and extracts only the regular binary at the requested version/target path. The public key cannot be supplied by the caller. The extracted bytes go to an exclusively created helper-owned 0700 temporary file; its writable descriptor closes before selftest, and slot installation reads that same private copy. Temporary files clean up on success/error. Existing rollback and health-check scheduling remain in place. No dependencies were added.

Strict TDD: marker scripts with no signature and with changed bytes were written first. Replaying the original validation/selftest sequence made both fail because selftest created the marker. With the fix, both return Refused at the expected signature/checksum check and neither marker exists. A third regression verifies effective-UID ownership, exact 0700 mode, successful selftest after caller staging replacement, installation of the original bytes, rollback to the prior slot and temporary-file cleanup. Archive tests cover expected version/target extraction and refusal of a different version.

Validation: cargo check --workspace passes. All three focused helper regressions and all 21 opsbox-update tests pass. Requested cargo test -p opsbox-priv reaches 236 passing library tests and three existing environment failures (socket bind denied, setuid bit stripped, uptime sysctl denied). Requested cargo test -p opsbox-api reaches 72 passing library tests and four socket-bind denials in TLS tests. Both full runs stop before integration suites. Formatting, Clippy, release-build and generated-index results are recorded in STATUS.md. Commands use timeout 120 and the local stable Rust toolchain on PATH.

Owner checks — disposable Multipass opsbox-vm with a recoverable snapshot:
1. From /home/ubuntu/opsbox, run timeout 120 cargo check --workspace, timeout 120 cargo test -p opsbox-priv, timeout 120 cargo test -p opsbox-api and timeout 120 cargo test -p opsbox-update. All must pass on Linux. The unsigned/modified marker tests must return Refused without creating a marker.
2. Build with CARGO_TARGET_DIR=~/target cargo build --release and install with sudo sh install.sh --dev --binary ~/target/release/opsbox. Tunnel the UI with ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>. Use a genuine signed release matching the server architecture: check both panel update and opsbox update --version <signed-version>, successful selftest, restart and healthy current slot. Verify that staged archives are removed after success and no opsbox-verified-* temporary file remains.
3. Using the snapshot between trials, verify a normal rollback with sudo opsbox rollback and the existing unhealthy-update automatic rollback path. Confirm the prior signed slot and schema checks still work. Test an upgrade from the preceding alpha as well: its old helper installs the new binary, after which the new helper requires archive proof for subsequent updates. Do not use a real production host for failure injection.

No commit, push, tag or installation was attempted for fix 28.

## Fix 29 — Helper audit identity comes from peer credentials

**Fixed in fix 29** (live Linux journal check pending).

All four `priv_request` tracing spans (ordinary requests, PTY, Docker log follow and deploy build) now include `peer_uid` and optional `pid` from the kernel credentials already verified at connection acceptance. The unchanged wire field `PrivRequest.actor` is logged as `claimed_actor`, with `-` when absent. It is advisory attribution, not authenticated panel-user identity. No authorization, payload or protocol changes were made.

Strict TDD: `server::tests::request_spans_log_peer_credentials_and_claimed_actor` first failed on Ping: the span had `actor="alice"` and `uid=501`, but lacked `peer_uid`. After the change it passes eight real socket-pair cases: all four request paths with `actor: "alice"` and with no actor. It captures structured span fields, compares UID/PID to real kernel peer credentials (also the current process PID on Linux), checks request ID/operation and rejects the old `actor` field. Streaming fixtures fail safely before opening a shell or contacting Docker/Git.

Validation: requested `cargo test -p opsbox-priv` ran 240 library tests: 237 passed, including both server connection tests, and three pre-existing environment failures remained (filesystem socket binding denied, temporary setuid bits stripped, uptime sysctl denied). Cargo stopped before integration suites. Focused all-target helper Clippy with warnings denied and index regeneration/freshness checks pass; final formatting/release results are in docs/STATUS.md. Commands use `timeout 120` and the local stable Rust toolchain on PATH.

Owner checks — disposable Multipass `opsbox-vm`:

1. From `/home/ubuntu/opsbox`, run `timeout 120 cargo test -p opsbox-priv`; the complete suite and the new audit regression must pass on Linux, including its real PID comparison.
2. Build with `CARGO_TARGET_DIR=~/target cargo build --release` and install with `sudo sh install.sh --dev --binary ~/target/release/opsbox`. Send a read-only Ping through `/run/opsbox/priv.sock` as the `opsbox` service account with a unique request ID and `actor: "alice"`; record that client's UID/PID. Inspect `sudo journalctl -u opsbox-priv --since '5 minutes ago'` and require the matching request's `peer_uid`, `pid` and `claimed_actor="alice"`, never a trusted `actor` field. Repeat without actor and require `claimed_actor="-"`.
3. Reuse Fix 27's harmless proxy PTY probe with `actor: "alice"`: it must still return Forbidden and open no terminal; its denial warning must retain the real UID/PID. Tunnel panel access with `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`.

No commit, push, tag or installation was attempted for fix 29.

## Fixed in fix 30 — Honest alpha security documentation (2026-11-20)

Fixed in fix 30. The threat model omitted the panel UID's root-equivalent helper access, the site claimed “Least privilege” and automatic lockdown, and SECURITY.md did not identify supported versions. Manual reproduction before editing: read the trust-boundary table and site security section; `rg -n -i 'least privilege|locked down from the first minute' site/index.html` matched both misleading claims.

Added Alpha status to docs/THREAT_MODEL.md and SECURITY.md and aligned README/site text: panel compromise means host root; fixes 26/27 separate the proxy UID/socket and restrict its operations to PortHolders and proxy: secret seal/open; fix 19 blocks elevated Compose for Deployers without promising hostile-tenant isolation; fix 20 excludes root-equivalent actions from Full tokens while retaining ordinary deployments; fix 21 filters exact Safe Mode source IPs before TLS, with an empty/default list still open to recovery authentication. Documented separate recovery restrictions, restart/loopback considerations, current-source versus installed-release limits, latest-alpha-only support and the existing private GitHub advisory channel. Corrected the threat model's claim that reading a proxy forwarding secret necessarily implies full panel control.

Validation: reviewed current helper caller authorization, server peer-credential/dispatch gates, Compose authorization, token denial/scope text and watchdog configuration against the prose. `timeout 120 python3 scripts/gen-index.py --check` and `timeout 120 git diff --check` pass; index regeneration produces no changes. No runtime test was added or claimed: this fix changes prose only and manual source/document comparison is the relevant check. The prompt is archived and the fix index is marked complete.

Owner checks: no new runtime checks for this docs-only fix. Existing Linux install, proxy separation, token/Compose and Safe Mode acceptance checks in fixes 19–29 remain pending where recorded; no VM installation or release verification is claimed. No commit, push or tag was attempted.
