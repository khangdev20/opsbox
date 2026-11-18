# 16e. Transport security: TLS everywhere and safe cookies

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-api/src/session.rs` (`SecureCookies`, cookie attributes), `crates/opsbox-api/src/hardening.rs` (security headers), `crates/opsbox-api/src/lib.rs` (router, middleware order), `crates/opsbox-api/src/tls.rs`, `crates/opsbox/src/config.rs` and `cmd/serve.rs` (listen address, TLS mode), `crates/opsbox-proxy` (panel domain, forwarding headers), every API module that returns secrets (`deploy.rs`, `databases.rs`, `backups.rs`, `tokens.rs`, `github.rs`, `alerts.rs`, `settings`), `web/js/*` where secrets are shown, `docs/API.md`, `docs/THREAT_MODEL.md` (create or extend). Spec: 6 (auth and sessions), 6.3 (first run), 7.18 (panel domain).

Context: the browser's DevTools shows decrypted JSON; that is expected and not a risk. The risks are traffic that is not encrypted on the network, cookies that can leak, responses that get cached, and secrets returned where they are not needed. A review on 2026-11-19 found good foundations (`SameSite=Strict`, `HttpOnly`, `nosniff`, `X-Frame-Options: DENY`, `no-store` on auth and a few download routes) and these gaps to check:
- `opsbox status` on a real install printed `serve http://0.0.0.0:8443`: plain HTTP on every interface.
- `SecureCookies::Never` exists, and `Auto` trusts `X-Forwarded-Proto`, which any client can send if `serve` is reachable directly.
- No HSTS; CSP has only `frame-ancestors`, `base-uri`, `object-src` (handled in 16r).
- `no-store` is set per route, not by default for the API (handled in 16r).

Secrets in responses (masking, reveal, log redaction) are prompt 16o.

Goal: each property below holds by construction and is pinned by a test, so a later change cannot silently break it. Do not weaken any existing control.

## 1. Encrypted transport, always
- `serve` never listens on a non-loopback address without TLS. Allowed modes:
  - Behind the opsbox proxy (panel domain): `serve` listens on `127.0.0.1` only; the proxy terminates TLS.
  - Direct first run / no domain: `serve` terminates TLS itself with the self-signed certificate from prompt 04 on the public port. Show its SHA-256 fingerprint in `opsbox status`, the setup token output and the installer summary, so the user can compare it with what the browser shows.
  - `--local-only` or `listen = "127.0.0.1:..."`: plain HTTP allowed (SSH tunnel use).
- Config load refuses `listen` on a non-loopback address with TLS off, with a clear error. No flag to override it in release builds; tests may use loopback.
- Port 80 on the proxy only redirects to HTTPS (308) and serves ACME challenges; it never serves the panel or API over HTTP.
- `opsbox status` prints the scheme it really serves (`https://…` or `http://127.0.0.1…`); fix the misleading `http://0.0.0.0:8443` line.
- TLS settings: TLS 1.2 minimum, 1.3 preferred, rustls defaults (no legacy ciphers), ALPN `h2` and `http/1.1`.

## 2. Cookies
- Remove `SecureCookies::Never` from release behaviour. `Secure` is set whenever the request arrived over TLS, which is known from the listener itself, not from headers.
- `X-Forwarded-Proto` and `X-Forwarded-For` are trusted only when the TCP peer is the opsbox proxy (loopback plus the proxy's known socket or a shared secret header the proxy adds and `serve` checks). Otherwise they are ignored and stripped before handlers see them. The client IP used for rate limits, audit and IP allowlists comes from this same trusted path.
- Session cookie: `__Host-` prefix when served over HTTPS (`__Host-opsbox_session`, `Path=/`, no `Domain`), `HttpOnly`, `Secure`, `SameSite=Strict`, idle and absolute lifetimes as in the spec. Keep the existing tool-cookie scoping in `tools.rs`.
- Logout and password or 2FA change revoke the session server side, not only the cookie.

Default response headers (no-store, HSTS, CSP and the rest) are prompt 16r.

## 5. Tests that pin all of this
- Config: non-loopback listen without TLS is rejected.
- A test client hitting `serve` directly with `X-Forwarded-Proto: https` and `X-Forwarded-For` spoofed: headers ignored, cookie still correct for the real scheme, audit shows the real peer.
- Cookie attribute tests for HTTPS and loopback-HTTP modes.
- A small script `scripts/check-transport.sh <url>` (curl based) that prints scheme, TLS version, certificate fingerprint, the security headers, and cookie attributes of a login; used in verification.

## Verify on `opsbox-vm`
- `opsbox status` shows `https://` and the fingerprint; `curl http://VM:8443` fails or redirects; `scripts/check-transport.sh` passes.
- With `tcpdump -i any -A port 8443` running during a login and a page load: no readable JSON, cookie or password in the capture (do it once, keep the capture out of git).
Record the results in `docs/STATUS.md`.

Out of scope: application-level encryption of JSON (not useful: the key would live in the same browser), mutual TLS, a WAF.
