# Status

Last prompt done: `04-public-first-run.md` (archived).
Next prompt: `05-proxy-panel-domain.md`.

## What works
- `crates/opsbox-priv/src/ops/files.rs` & `crates/opsbox-api/src/files.rs`: `rustix` dirfd/`*at` calls, `openat2` (Linux 5.6+) with `O_NOFOLLOW | O_DIRECTORY` fallback, dev/ino deny checks.
- CI & Release: `cargo-deny`, `cargo-zigbuild` musl builds, minisign-signed releases, CycloneDX SBOM, `install.sh` (`--channel beta`, `--local-only`).
- Self-update (`opsbox-update`, `opsbox-priv/src/ops/update.rs`, `opsbox-api/src/update.rs`, `opsbox/src/cmd/update.rs`): version slots, daily check, SHA-256 + minisign verify, `systemd-run` 2-min rollback, SQLite pre-migration backup, real Settings UI & `/attention`.
- Public first run (`opsbox-api/src/tls.rs`, `opsbox/src/cmd/serve.rs`, `opsbox/src/cmd/web.rs`):
  - Default `0.0.0.0:8443` with self-signed TLS (`rcgen` + `tokio-rustls`), public IP detection (AWS IMDSv2, GCP, Azure, DigitalOcean, Hetzner, interface fallback).
  - Pre-claim `setup_guard` returns 403 for all API routes except `/api/v1/setup/*`, `/healthz`, `/readyz`, and static assets.
  - `Secure` cookies on native TLS, `panel-tls` security check warning while self-signed/no domain, `opsbox web local-only on|off`, `--local-only` in `install.sh`, auto `http` mode in `web/js/api.js` on HTTPS/8443.

## What is partial
- None.

## Found, not fixed
- Unused dependency warning for `futures-util` in `crates/opsbox-priv/Cargo.toml`.
