# 02. Release builds, signing and one-line install

Spec: 10, 14. Goal: anyone can install the latest opsbox with one command, and every new version is published automatically from a git tag.

Do:
- CI builds static `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` (cargo-zigbuild or cross) on every push; report binary sizes.
- Release workflow on tags `v*` (tags with `-alpha`, `-beta`, `-rc` are marked pre-release): build both targets, sign with minisign (secret key from a protected GitHub environment secret), upload `opsbox-<version>-<target>.tar.gz`, `.sha256`, `.minisig`, and `install.sh` itself as release assets. Attach an SBOM. Add `cargo deny` to CI.
- `install.sh` embeds the minisign public key. Normal path: detect arch, download the archive for the latest release (or `--version vX.Y.Z`, or `--channel beta` to include pre-releases), verify SHA-256 and signature, abort on mismatch, then install as today. `--dev --binary` stays for local builds. Works unattended from cloud-init user data.
- Canonical install command, independent of the `main` branch:
  `curl -fsSL https://github.com/khangdev20/opsbox/releases/latest/download/install.sh | sh`
  Update README and `site/index.html` (both install boxes and the "How it works" step) to use it.
- Re-running the installer on a machine that has opsbox upgrades it in place (keeps data and config), so manual updates work before prompt 03.
- `docs/RELEASING.md`: generate the minisign key pair once (never commit the secret key), store it in the GitHub environment secret, put the public key in `install.sh`, cut a release by pushing a tag, how to yank a bad release.

Owner action needed: the agent cannot create the signing key secret in GitHub. Stop and tell the owner exactly what to run and where to paste it, then continue.

Verify: tag `v0.1.0-alpha.1` from `dev`, wait for the release, then on the VM (fresh snapshot) run the one-line install with `--channel beta`; run it again to confirm an in-place upgrade keeps the admin account. Refusal check: tamper with the downloaded archive and confirm the installer aborts.
