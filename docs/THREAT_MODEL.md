# Threat model

Short, one section per area. Extend it when a prompt changes what is trusted. Reporting channel and response targets: [SECURITY.md](../SECURITY.md) (summary in [docs/SECURITY.md](SECURITY.md)).

## Alpha status

- **Panel compromise means host-root compromise.** `serve` runs as the `opsbox` UID and delegates system actions to the root helper. That UID can request every typed `PrivOp`, including root jobs and terminals; the helper does not require independent proof of an authenticated panel administrator. Typed requests and parameter validation do not contain a compromised panel process. Keep panel access behind a VPN, SSH tunnel (`--local-only`), or IP allowlist.
- **Proxy separation is implemented (fixes 26/27).** The proxy runs as the distinct `opsbox-proxy` UID and connects through `/run/opsbox/priv-proxy.sock`. Kernel peer credentials and an operation allowlist restrict it to `PortHolders` and `SecretSeal`/`SecretOpen` in `proxy:` contexts. It cannot use the panel helper socket or request jobs, terminals or arbitrary file writes. A compromised proxy still controls routed traffic, its forwarding secret and proxy certificates; this separation does not protect the panel if the attacker also compromises `serve`.
- **Deployers must be trusted operators.** Fix 19 requires Admin approval for elevated Compose, checks raw and helper-resolved configuration, and refuses unsafe parsing. Before that fix, Deployer access could grant host root through Compose. These checks reduce known escalation paths; they are not a sandbox for hostile tenants or untrusted build/deploy code. Do not grant Deployer access to untrusted people on an alpha host.
- **Full API tokens remain sensitive (fix 20).** Full means “everything except root-equivalent actions”. Root-equivalent and sensitive account/secret routes require an interactive session; ordinary deployments remain available. Token redeploys apply Deployer checks to elevated Compose even if an Admin previously confirmed it. Protect and revoke leaked tokens: they still grant broad application access and can change deployed code.
- **Restrict Safe Mode separately (fix 21).** The watchdog is a root recovery service, normally on port 8444. Its default bind follows the panel IP, and an empty/default `[watchdog] allow_ips = []` permits any source to reach recovery authentication. Set exact allowed IPv4/IPv6 addresses (no CIDRs) and restart `opsbox-watchdog`; other peers are dropped before TLS. Loopback is not implicitly allowed, so include it for SSH tunnels. `--local-only` makes the default watchdog bind loopback unless `[watchdog] listen` overrides it. The watchdog uses its own root-only TLS key; check exposure and its certificate fingerprint with `opsbox status`.

These describe the current source and packaged configuration. Existing hosts need the fixes installed and services restarted; outstanding Linux installation and recovery checks are tracked in [QA.md](QA.md).

## Trust boundaries (alpha)

| Boundary | Trust | Notes |
|---|---|---|
| Browser → `serve` | Untrusted until claimed; then authenticated session + TOTP | Claim needs the one-time setup token; after claim, setup routes 404 |
| `serve` → `priv` | Panel/root kernel peer UID on the panel socket; typed `PrivOp` only | Panel UID can request all operations; treat `serve` compromise as root |
| Proxy → `priv` | Separate proxy UID and socket (fixes 26/27) | Only `PortHolders` and `proxy:` secret seal/open operations |
| Proxy → `serve` | Loopback + shared secret | Forwarded headers ignored unless both match |
| Installer → host | Root for install only | Verifies SHA-256 + minisign before replacing the binary |
| Watchdog / Safe Mode | Root recovery path with separate TLS key | Separate listener/allowlist; empty allowlist permits remote access to authentication |

In scope for no one yet: a public bug bounty, formal third-party audit, or hardening against a malicious admin who already owns the panel.

## Helper audit identity (fix 29)

`PrivRequest.actor` is caller-supplied advisory text, not authenticated identity or proof of a panel user. Request tracing labels it `claimed_actor` (or `-` when absent). Kernel Unix-socket peer credentials are authoritative for the connecting process: `peer_uid` and `pid` accompany ordinary requests, PTY sessions, Docker log streams and deploy builds. PID is optional on platforms where the kernel API does not provide it. These credentials identify the local process/account, not the human using the panel; a compromised caller can forge its claimed actor but cannot replace the kernel identity in these fields.

## Transport (prompt 16e)

What we defend against: someone on the network reading or changing traffic, a cookie that leaks or is planted, and a client that lies about where it came from.

| Threat | Control | Where it is pinned |
|---|---|---|
| Passwords and cookies on the wire | `serve` never listens on a non-loopback address without TLS: config load refuses `tls = false` there, in every build. TLS 1.2 minimum, 1.3 preferred, rustls default suites, ALPN `h2` and `http/1.1`. | `config.rs` tests; `tls.rs` tests; `scripts/check-transport.sh` |
| Fake certificate on first connect | `opsbox status` prints the SHA-256 fingerprint of the certificate the listener really presents (a handshake, not a file). | `status.rs` tests |
| Cookie sent over HTTP | `Secure` is set whenever the request arrived over TLS, decided by our listener or the trusted proxy, never by a client header. There is no "never" setting. | `session.rs`, `auth_http.rs` |
| Cookie planted from a sibling site | Over HTTPS the session cookie is `__Host-opsbox_session`; a plain-named cookie is ignored there. | `auth_http.rs` |
| Spoofed `X-Forwarded-For` / `-Proto` (bypass of the allowed-address list, rate limits, audit, `Secure`) | Honoured only from a loopback peer that sends the proxy's shared secret; otherwise removed before handlers run. | `transport.rs`, `transport_http.rs`, `security_http.rs` |
| Another program on the same machine posing as the proxy | Loopback alone is not enough: it must also know the secret, which is stored in the database (mode 0600) and added by the proxy to the panel route only. | `transport.rs` tests |
| Session kept after sign-out | Sign-out deletes the session on the server; a copied cookie stops working. | `auth_http.rs` |

In scope for no one: encrypting JSON inside TLS, mutual TLS, a WAF.

Known limit: the forwarding secret is in the panel database and the proxy's route table in memory. A compromised proxy can forge forwarded headers to `serve`, including client addresses used by its allowlist and rate limits. This secret authenticates proxy transport, not a panel user; panel session authentication still applies. Protect panel reachability at the VPN or firewall as well.

## Default response headers (prompt 16r)

What we defend against: a response that gets cached with secrets, a page framed or scripted by another origin, a browser that remembers HSTS for a disposable self-signed IP certificate, and error bodies that leak internals.

| Threat | Control | Where it is pinned |
|---|---|---|
| API / HTML cached by a proxy or browser | `Cache-Control: no-store` on every `/api/*` response and on HTML; only hashed `fonts/` / `vendor/` assets may use long-lived `immutable` | `hardening.rs`, `hardening_http.rs`, `scripts/check-transport.sh` |
| Clickjacking / hostile framing | `X-Frame-Options: DENY` and CSP `frame-ancestors 'none'` | `hardening.rs` |
| Inline or third-party script on the panel | CSP `default-src` / `script-src 'self'` (inline styles kept for the lit UI); `/tools/…` alone gets a looser CSP for pgweb/Adminer | `hardening.rs` |
| HSTS lock-out after self-signed cert rotation | `Strict-Transport-Security: max-age=31536000` only when the request is HTTPS on a real DNS name; absent for IP Host and loopback HTTP | `hardening::should_send_hsts`, `hardening_http.rs` |
| Fingerprinting via `Server` / stack traces | `Server` removed; `HttpError` messages never include paths, backtraces or secrets | `hardening.rs`, `error.rs` |
| Camera / mic / geolocation prompts; cross-origin window attacks | `Permissions-Policy: camera=(), microphone=(), geolocation=()`; `Cross-Origin-Opener-Policy: same-origin` | `hardening.rs` |

## Secrets in responses and logs (prompt 16o)

What we defend against: secrets appearing in list/detail JSON when they are not needed, lingering in DevTools or caches, being written to deploy/task logs, or being revealed without a fresh confirmation.

### Inventory

| Secret | Stored as | List / detail | Full value |
|---|---|---|---|
| Service / database env values (`DATABASE_URL`, passwords, tokens, …) | Sealed in SQLite via `priv` (`deploy:*`) | `••••••••` (or reference syntax for `${{ … }}`); never opened | `POST /services/{id}/vars/{key}/reveal`; DB URLs also `POST /databases/{id}/connection-string` |
| API tokens (`ops_…`) | Hash only | Metadata on `GET /settings` | Shown once on `POST /tokens`; not recoverable |
| Backup S3 access keys / SFTP private keys | Sealed (`backup:dest:<id>`) | `hasCredentials` / `hasValue: true` on destinations | `POST /backups/destinations/{id}/reveal` |
| Restic recovery password | Derived from master key (not stored) | Not listed | `POST /backups/recovery-key` after re-auth |
| Alert channel credentials (SMTP password, Slack/Discord/Telegram/webhook secrets) | Sealed (`alerts:channel:<id>`) | `hasValue: true`; Discord shows only the token's last four characters | `POST /alerts/channels/{id}/reveal` for other channels; Discord cannot be revealed |
| GitHub App webhook secret | Sealed (`deploy:github_app:webhook_secret`) | `hasWebhookSecret: true` | `POST /github/webhook-secret/reveal` |
| GitHub App private key / deploy-key private keys | Sealed | `hasPrivateKey: true` / public key + fingerprint only | Never over HTTP (used only inside `priv`) |
| TOTP secrets | Sealed on the user row | Not listed after enrolment | Shown once at enrol (`/auth/totp/enroll`); thereafter only verified in `priv` |
| Docker container env (inspect) | Live from Engine | `••••` + last 4 for secret-looking keys | Not revealed via the panel (rotate/redeploy instead) |
| Proxy forward secret | Settings row (mode 0600) | Never in API responses | Used only between proxy and `serve` |

### Controls

| Threat | Control | Where |
|---|---|---|
| Secret in ordinary GET JSON | Masked placeholder or `hasValue`; sealed values are not opened for list/detail | `deploy`, `databases`, `backups`, `alerts`, `github`, `tokens`, docker `mask_env` |
| Reveal without recent proof | Owner/admin (or service deployer for vars); interactive session; password or 2FA within 5 minutes (`POST /auth/reauth` or credentials on the reveal body) | `secrets::begin_reveal` |
| Reveal abused / scraped | Per-session reveal rate limit; `Cache-Control: no-store`; audit names the secret and target, never the value | `limits::reveal`, `secrets::audit_reveal` |
| Secret echoed in deploy/task logs | Known values scrubbed to `[REDACTED]` in `priv` build/start and again in serve-side deploy/task writers | `opsbox_deploy::scrub_secrets`, `secrets::redact_line`, `DeployHub::emit`, `Tasks` |
| UI leaves plaintext on screen | Reveal button; auto-hide after 30 s or navigation; Copy prefers the clipboard without keeping the value in the page | `web/js/secrets.js` |
