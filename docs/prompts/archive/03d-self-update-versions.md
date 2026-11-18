# 03d. Fix self-update: the panel always says "up to date"

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `Cargo.toml` (workspace version), `crates/opsbox/src/main.rs` (`VERSION`), `.github/workflows/release.yml`, `crates/opsbox-update/src/release.rs` (`select_release`), `crates/opsbox-api/src/update.rs` (`Updater::channel`, `apply`), `crates/opsbox/src/cmd/update.rs` (CLI), `install.sh`, `docs/RELEASING.md`. Spec: 7.15 (self-update).

## What is wrong (found 2026-11-19)
On a real server installed from `v0.1.0-alpha.13` with `--channel beta`, Settings > opsbox updates always shows "opsbox 0.1.0 · up to date", even with `v0.1.0-alpha.20` published. Three causes, all need fixing:

1. **The binary does not know its release version.** `VERSION = env!("CARGO_PKG_VERSION")` and the workspace version is `0.1.0` for every tag. The release workflow builds tags `v0.1.0-alpha.N` without changing it, so every installed build reports `0.1.0`.
2. **Semver then says nothing is newer.** `select_release` compares with `semver::Version`, where `0.1.0-alpha.20 < 0.1.0`. So even on the beta channel no alpha can ever be "newer" than the running `0.1.0`.
3. **The installer's channel is not kept.** `install.sh --channel beta` uses beta only to pick the download. The panel's `opsbox.update.channel` setting is never written, so it defaults to `stable`. Every release is a pre-release, so the stable channel finds no release at all.

## Fix
1. **Version from the tag**
   - The release workflow sets the crate version from the tag before building (`VERSION="${TAG#v}"`, then `cargo set-version` via cargo-edit, or a `sed` on the workspace `version`, then `cargo build --locked`; keep `Cargo.lock` consistent). It fails the job if the built binary's `opsbox --version` does not equal the tag.
   - Local and dev builds get a clear dev version, for example `0.1.0-dev+<short sha>` from a `build.rs` (`OPSBOX_BUILD_VERSION` env overrides). A dev build never offers updates unless the user picks a version explicitly.
   - `opsbox --version`, `opsbox status`, the API (`/health`, `/updates/self`) and the UI all show the full version (`0.1.0-alpha.21`), not just `0.1.0`.
2. **Channel kept from install**
   - `install.sh --channel beta` (and an install of a pre-release by `--version`) writes the channel to the config file (`[update] channel = "beta"`). The updater reads the setting first, then config, then a default.
   - The default channel is `beta` while the running version is a pre-release, and `stable` otherwise. The UI shows which source the channel came from.
3. **Migration for servers already installed**
   - They report `0.1.0` and cannot see alphas as newer. When the running version has no pre-release part and the newest published release is `0.1.0-alpha.N` with the same major.minor.patch, treat the running build as "unknown alpha" and offer the newest alpha on the beta channel, once, with a note ("Your install predates version tagging; update to get accurate versions").
   - Document the manual path too: `sudo opsbox update --version v0.1.0-alpha.<N>`. Check that it works today (`apply` with an explicit version skips the comparison) and keep it working.
Making the reason visible in the UI ("no stable release yet", rate limits, last checked) is prompt 03e.

## Tests
- `select_release`: alpha.13 on beta sees alpha.20; stable with only pre-releases returns "no release on channel"; `0.1.0` (old build) with alphas present returns the migration offer; dev builds return none.
- Version stamping: a CI step builds with tag `v9.9.9-alpha.1` in a dry run and asserts `opsbox --version` prints it.
- Installer: `--channel beta` writes the channel; a stable install does not.

## Verify
- On `opsbox-vm` and `almanac-sensor` (currently `alpha.13`, reporting `0.1.0`): after this release is tagged (`v0.1.0-alpha.21`), the panel offers it (migration path). Update from the panel, then confirm the panel shows `0.1.0-alpha.21` and offers nothing newer; tag `alpha.22` with a trivial change and confirm the normal path offers it; roll back works.
- Record the results in `docs/STATUS.md`. Update `docs/RELEASING.md` with the version stamping step.

Out of scope: changing the release cadence, signing, or the rollback logic (03).
