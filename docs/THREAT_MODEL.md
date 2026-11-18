# Threat model

Short, one section per area. Extend it when a prompt changes what is trusted.

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

Not covered here: response caching and default headers (16r). In scope for no one: encrypting JSON inside TLS, mutual TLS, a WAF.

Known limit: the proxy secret is in the database and in the proxy's route table in memory. Anyone who can read either already controls opsbox.

## Secrets in responses and logs (prompt 16o)

What we defend against: secrets appearing in list/detail JSON when they are not needed, lingering in DevTools or caches, being written to deploy/task logs, or being revealed without a fresh confirmation.

### Inventory

| Secret | Stored as | List / detail | Full value |
|---|---|---|---|
| Service / database env values (`DATABASE_URL`, passwords, tokens, …) | Sealed in SQLite via `priv` (`deploy:*`) | `••••••••` (or reference syntax for `${{ … }}`); never opened | `POST /services/{id}/vars/{key}/reveal`; DB URLs also `POST /databases/{id}/connection-string` |
| API tokens (`ops_…`) | Hash only | Metadata on `GET /settings` | Shown once on `POST /tokens`; not recoverable |
| Backup S3 access keys / SFTP private keys | Sealed (`backup:dest:<id>`) | `hasCredentials` / `hasValue: true` on destinations | `POST /backups/destinations/{id}/reveal` |
| Restic recovery password | Derived from master key (not stored) | Not listed | `POST /backups/recovery-key` after re-auth |
| Alert channel credentials (SMTP password, Slack/Telegram/webhook secrets) | Sealed (`alerts:channel:<id>`) | `hasValue: true`, `to: "Configured"` | `POST /alerts/channels/{id}/reveal` |
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
