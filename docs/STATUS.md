# Status

Last prompt done: `01-file-access-race.md` (archived).
Current prompt (partial): `02-release-builds.md`.
Next prompt: `02-release-builds.md` (finish after owner sets minisign key).

## What works
- `crates/opsbox-priv/src/ops/files.rs` and `crates/opsbox-api/src/files.rs` use `rustix` dirfd/`*at` calls, `openat2` (Linux 5.6+) with `O_NOFOLLOW | O_DIRECTORY` fallback, and dev/ino deny checks.
- `.github/workflows/ci.yml`: added `cargo-deny` (`deny.toml`), static `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` builds via `cargo-zigbuild`, and binary size reporting.
- `.github/workflows/release.yml`: builds both musl targets on `v*` tags, packages `.tar.gz`, `.sha256`, `.minisig`, CycloneDX SBOM, and `install.sh`; marks `-alpha`/`-beta`/`-rc` as pre-release.
- `install.sh`: supports `--channel beta` (fetches pre-releases), auto-installs `minisign` via `apt-get`/`dnf` if missing, preserves data/config on in-place upgrade.
- `README.md`, `site/index.html`, and `docs/RELEASING.md` updated with canonical release install command and releasing guide.

## What is partial
- `02-release-builds.md`: `MINISIGN_PUBKEY` in `install.sh` is still empty and GitHub `release` environment secret `MINISIGN_SECRET_KEY` needs to be created by the owner before tagging `v0.1.0-alpha.1` and verifying on the VM.

## Found, not fixed
- Unused dependency warning for `futures-util` in `crates/opsbox-priv/Cargo.toml`.
