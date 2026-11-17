# Status

Last completed prompt: `docs/prompts/archive/10-app-domains.md` (App domains)

## What works
- `crates/opsbox-store` (`0007_app_domains.sql`, `deploy.rs`): `service_domains` table with multi-domain support, primary election/sync with `services.domain`, `redirect_to`, `force_https`, `hsts`, and `dns_status` persistence.
- `crates/opsbox-proxy` (`proto.rs`, `proxy.rs`, `acme.rs`, `server.rs`, `domain.rs`): per-domain `redirect_to` HTTP/HTTPS permanent redirects, cert-gated `redirect_https` (plain HTTP until cert exists, `308` once active unless disabled), per-route `hsts`, and persisted ACME renewal failure/backoff tracking (`consecutive_failures`, `last_error`, `next_retry_at`).
- `crates/opsbox-api` (`domains.rs`, `panel.rs`, `deploy.rs`, `metrics.rs`) & `crates/opsbox` (`cmd/web.rs`): `GET/POST /services/{id}/domains`, `POST /domains/{name}/check`, `POST /domains/{name}/certificate` (`202 {task}`), `PATCH/DELETE /domains/{name}`, `POST /domains/{name}/primary`; strict RBAC/owner/`deploy:<svc>` token enforcement, panel/app domain collision checks, `"tls"` renewal-failure items in `GET /attention`, and shared `build_routes_with_panel`.
- `web/js` (`api.js`, `pages/apps.js`): `Domains & SSL` tab fully wired (`REAL`) with zero sample data badge, internal `<service>` project-network URL reference, live public IP DNS hints, www/redirect modes, Force HTTPS & HSTS switches, certificate issuance/renewal, and primary selection.

## Partial / Found, not fixed
- Linux-only runtime/CI checks (`tools/check-idle-memory.py`, `musl` static build, `cargo-deny`, real Docker Engine + public port 80/443 Let's Encrypt HTTP-01 on Ubuntu) cannot run natively on macOS and remain for Linux CI / `opsbox-vm`.
- Wildcard app domains (`*.example.com`) require DNS-01 provider tokens (not part of prompt 10) and are explicitly rejected with a clear message.

## Next
- Verify adding a public test domain, DNS check, Let's Encrypt certificate issuance, `www` redirect, Force HTTPS/HSTS toggles, and primary domain switching in `opsbox-vm`.
- Next prompt: `docs/prompts/11-templates-compose.md`.
