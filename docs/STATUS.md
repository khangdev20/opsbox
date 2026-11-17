# Status

Last prompt done: `03-self-update.md` (archived).
Next prompt: `04-public-first-run.md`.

## What works
- `crates/opsbox-priv/src/ops/files.rs` and `crates/opsbox-api/src/files.rs`: `rustix` dirfd/`*at` calls, `openat2` (Linux 5.6+) with `O_NOFOLLOW | O_DIRECTORY` fallback, dev/ino deny checks.
- CI & Release: `cargo-deny`, `cargo-zigbuild` musl builds, minisign-signed releases, CycloneDX SBOM, `install.sh` with embedded `MINISIGN_PUBKEY` and `--channel beta`.
- Self-update (`opsbox-update`, `opsbox-priv/src/ops/update.rs`, `opsbox-api/src/update.rs`, `opsbox/src/cmd/update.rs`):
  - Version slots (`/var/lib/opsbox/bin/opsbox-<version>`), `/usr/local/bin/opsbox` symlink, single-binary migration, keep current + previous.
  - Daily GitHub Releases check, SHA-256 + minisign verification, `selftest`, `systemd-run` 2-minute health check & automatic rollback, SQLite pre-migration backup & incompatible rollback refusal.
  - Real Settings "opsbox updates" UI, Overview `/attention` update item, CLI `opsbox update` & `opsbox rollback`.

## What is partial
- None.

## Found, not fixed
- Unused dependency warning for `futures-util` in `crates/opsbox-priv/Cargo.toml`.
