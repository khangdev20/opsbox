# Status

Last completed prompt: `docs/prompts/archive/16r-security-headers.md` (default security headers). Not committed / not pushed (per session request). Prior: `16q`.

## What works (16r)
- **`hardening::middleware`**: every response gets `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: same-origin`, `Permissions-Policy: camera=(), microphone=(), geolocation=()`, `Cross-Origin-Opener-Policy: same-origin`, full panel CSP; `Server` stripped.
- **`Cache-Control: no-store`** on all `/api/*` and HTML; `fonts/` / `vendor/` keep long-lived `immutable`.
- **HSTS** `max-age=31536000` only when HTTPS + real DNS `Host`; absent on self-signed IP and loopback HTTP.
- **`/tools/…` CSP** looser (`unsafe-inline` / `unsafe-eval`) for pgweb/Adminer only.
- **Tests**: `hardening_http.rs` route table, 404/405 headers, API `no-store`, conditional HSTS.
- **Docs/scripts**: `API.md`, `THREAT_MODEL.md`, `DECISIONS` 16r, `scripts/check-transport.sh` lists new headers + HTML `no-store` + conditional HSTS.
- **Monaco**: `opsbox-editor.worker.js` same-origin bootstrap (replaces `data:` worker) so Files editor works under CSP.

## Partial / Found, not fixed
- **Shell unavailable this session** — could not run `cargo test` / `fmt` / `clippy`, or `git mv`. Archive copy is at `docs/prompts/archive/16r-security-headers.md`; remove the live `docs/prompts/16r-security-headers.md` with `git mv` (or `git rm` + add) before commit.
- **Not verified on `opsbox-vm`** (panel domain HSTS + DevTools CSP console). Run `scripts/check-transport.sh` there.
- Playwright CSP smoke across all pages/terminal/pgweb not run this session.
- Carried over: pending commit/release for 16o–16r (commit only when asked).

## Next
- Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (esp. `hardening_http`).
- Then commit (only when asked): `prompt 16r: default security headers`. Do not push unless asked.
- Then `16s-responsive-panel.md`.
- VM check: through panel domain — HSTS present, HTTP:80 redirects; `check-transport.sh` PASS; DevTools console clean on every page, terminal, pgweb. Must fail safely: self-signed IP URL must **not** send HSTS.
