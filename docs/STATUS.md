# Status

Last prompt done: `02-release-builds.md` (archived).
Next prompt: `03-self-update.md`.

## What works
- `crates/opsbox-priv/src/ops/files.rs` and `crates/opsbox-api/src/files.rs` use `rustix` dirfd/`*at` calls, `openat2` (Linux 5.6+) with `O_NOFOLLOW | O_DIRECTORY` fallback, and dev/ino deny checks.
- `.github/workflows/ci.yml`: `cargo-deny` (`deny.toml`), static `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` builds via `cargo-zigbuild`, binary size reporting.
- `.github/workflows/release.yml`: builds both musl targets on `v*` tags, signs with minisign, attaches `.sha256`, `.minisig`, CycloneDX SBOM, and `install.sh`; marks `-alpha`/`-beta`/`-rc` as pre-release.
- `install.sh`: embeds `MINISIGN_PUBKEY`, supports `--channel beta`, auto-installs `minisign` if missing, preserves data/config on in-place upgrade.
- `README.md`, `site/index.html`, and `docs/RELEASING.md` updated.

## What is partial
- None for prompt 02.

## Found, not fixed
- Unused dependency warning for `futures-util` in `crates/opsbox-priv/Cargo.toml`.
