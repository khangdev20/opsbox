# Releasing opsbox

Releases are built, signed, and published to GitHub Releases by `.github/workflows/release.yml` whenever a `v*` tag is pushed.

## One-time signing key setup

Release archives are signed with [minisign](https://jedisct1.github.io/minisign/).

1. Generate a key pair on a trusted local machine (pass `-W` for an unencrypted secret key if you want unattended CI signing without a separate password secret, or omit `-W` and set `MINISIGN_PASSWORD` as well):

   ```sh
   minisign -G -W -p opsbox.pub -s opsbox.key
   ```

   **Never commit `opsbox.key` to git.** Store a backup in your password manager and delete the local copy once uploaded.

2. In the GitHub repository, open **Settings → Environments** and create a protected environment named `release` (optionally require reviewer approval or restrict to `v*` tags).

3. Under the `release` environment, add a secret named `MINISIGN_SECRET_KEY` and paste the entire contents of `opsbox.key`. (If you encrypted the key with a passphrase, also add `MINISIGN_PASSWORD`.)

4. Copy the public key string (`RW...`, the second line of `opsbox.pub`) into `MINISIGN_PUBKEY="..."` in `install.sh` and commit that change.

## Cutting a release

1. Ensure `dev` is green in CI (`cargo fmt`, `cargo clippy`, `cargo test`, `cargo deny check`, musl builds).
2. Update the version in `Cargo.toml` if needed, commit, and push to `origin/dev`.
3. Create and push a tag:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

   Tags containing `-alpha`, `-beta`, or `-rc` (for example `v0.1.0-alpha.1`) are automatically marked as **pre-release** on GitHub.

The release workflow builds static `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` binaries, packages `opsbox-<version>-<target>.tar.gz`, generates `.sha256` checksums, signs each archive with `minisign` (`.minisig`), attaches a CycloneDX SBOM (`opsbox-<version>.cdx.json`), and uploads `install.sh` as a release asset.

## Yanking a bad release

If a published release has a critical bug:

1. **Do not** re-use the same version tag.
2. In GitHub Releases, either:
   - Edit the release and mark it **Pre-release** (so `/releases/latest` no longer points to it and `install.sh` stops serving it on the default channel), or
   - Delete the GitHub Release and delete the remote tag (`git push origin :refs/tags/vX.Y.Z`) if it was just cut.
3. Fix the issue on `dev` and cut a new patch tag (`vX.Y.(Z+1)`).
