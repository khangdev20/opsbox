# Status

Last completed prompt: `docs/prompts/archive/05-proxy-panel-domain.md` (Proxy and panel domain)

## What works
- In-process Rust reverse proxy (`crates/opsbox-proxy`) on `:80`/`:443` with control socket `/run/opsbox-proxy/proxy.sock` and persisted `routes.json` (Hyper 1, `tokio-rustls`/`rustls` with `ring`, `instant-acme` HTTP-01, `hickory-resolver`, `x509-parser`).
- HTTP `:80` answers `/.well-known/acme-challenge/<token>` and redirects other traffic (`308`) to HTTPS; HTTPS preserves `Host`, forwards `X-Forwarded-Proto`/`X-Forwarded-For`, pins upstream `serve` TLS cert SHA-256, adds HSTS, enforces upload limits (`413`), and upgrades WebSockets (`101`).
- ACME account and domain private keys are encrypted at rest via `opsbox-priv` (`PrivOp::SecretSeal`/`SecretOpen`, restricted to `"proxy:"` contexts and refusing `"totp:*"`) and renewed 30 days before expiry.
- `opsbox-api`: `/api/v1/panel` (`GET`, `/dns-check`, async `/certificate` task, `DELETE /domain`, `/local-only`), `GET /api/v1/settings` `panel` snapshot, startup route sync, and `/readyz` requiring `proxy` when a panel domain is active.
- Settings UI (`web/js/pages/settings.js`, `web/js/api.js`): wired `REAL` Panel address section with Check DNS (`points here` / `waiting for DNS` + DNS records table), Issue/Renew certificate, Remove, and Close raw port 8443 offer (`sample data` badge removed).
- CLI & packaging: `opsbox proxy`, `opsbox web domain <name> [--skip-dns-check]`, `opsbox status`/`selftest`/`doctor`/`update`, `packaging/systemd/opsbox-proxy.service`, `install.sh`, and `.github/workflows/release.yml`.

## Partial / Found, not fixed
- Pre-existing `cargo::unused_dependencies` warning for `futures-util` in `crates/opsbox-priv/Cargo.toml` on non-Linux hosts (`futures-util` is only used inside `#[cfg(target_os = "linux")]`).
- Linux-only checks (`tools/check-idle-memory.py`, `musl` target build, systemd service runtime, live Let's Encrypt staging in VM) are not runnable natively on macOS and remain for Linux CI / `opsbox-vm`.

## Next
- Verify `opsbox-proxy.service`, UI Panel address flow, ACME staging, and closing port `:8443` in `opsbox-vm`.
- Next prompt: `docs/prompts/06-watchdog-safe-mode.md`.
