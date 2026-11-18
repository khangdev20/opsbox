# 16r. Default security headers on every response

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-api/src/hardening.rs`, `crates/opsbox-api/src/lib.rs` (middleware order), `crates/opsbox-api/src/tools.rs` (pgweb and Adminer responses), `crates/opsbox-proxy` (panel domain responses), `web/` (only if CSP needs code changes), `docs/API.md`, `docs/THREAT_MODEL.md`. Builds on 16e (the server knows reliably whether a request came over TLS and through the proxy).

Today: `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: same-origin` and a minimal CSP are set; `no-store` is set per route only; there is no HSTS.

## Do
Set in one middleware for every response from `serve` (API, UI, downloads, errors, 404s), with per-route exceptions only where needed:
- `Cache-Control: no-store` for all `/api/*` responses and HTML; long-lived `immutable` caching only for hashed static assets, if any.
- `Strict-Transport-Security: max-age=31536000` when served over HTTPS on a real domain (not on the self-signed IP mode, where HSTS would lock users out after a certificate change); no `includeSubDomains` or preload by default.
- `Content-Security-Policy`: `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'` (only if inline styles remain after 16d; otherwise drop it); `img-src 'self' data:`; `connect-src 'self'`; `frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'`. Check the terminal (xterm) and any vendor code still work; fix the code, not the policy, where possible.
- Keep `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: same-origin`; add `Permissions-Policy: camera=(), microphone=(), geolocation=()` and `Cross-Origin-Opener-Policy: same-origin`.
- No `Server` version header; error bodies never contain stack traces, paths or secrets.


Data tools (pgweb, Adminer under `/tools/...`) may need a looser CSP: give them their own policy on their path prefix only, never relax the panel's.

Tests:
- Header test over a route table: every registered route (walk the router) returns the required headers, including error and 404 paths; `/api/*` all have `no-store`.
- HSTS present only on HTTPS with a real domain, absent in self-signed IP mode and on loopback HTTP.
- The panel loads with the new CSP: run the existing Playwright smoke over all pages and the terminal with no CSP violations in the console.

Verify on `opsbox-vm`:
- Through the panel domain from prompt 05: HSTS present, HTTP on port 80 redirects.
- `scripts/check-transport.sh` (from 16e) lists every header; DevTools console shows no CSP violations on every page, the terminal and pgweb.
Record the results in `docs/STATUS.md`.
