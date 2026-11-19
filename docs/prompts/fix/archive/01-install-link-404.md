# Fix 01. The one-line install command returns 404

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
README, `docs/INSTALL.md` and `site/index.html` tell people to run `curl -fsSL https://github.com/khangdev20/opsbox/releases/latest/download/install.sh | sh`. GitHub's `/releases/latest` ignores pre-releases and every alpha is published as a pre-release, so this URL returns 404 (checked: the same file under `/releases/download/<tag>/` returns 200). The command on the front page does not work.

## Steps
1. Read `.github/workflows/release.yml` and `crates/opsbox-update/src/release.rs` (`select_release`): find where the pre-release flag is set and how the updater uses it for the `stable` and `beta` channels.
2. Decide the smallest safe fix and write it in `docs/DECISIONS.md` (3 lines). Preferred: for tags matching `v0.1.0-alpha.*`, publish the GitHub release as a normal release titled `OpsBox alpha N` (not a pre-release), so `/releases/latest` works; the alpha period accepts that the default channel sees alphas. If this would make a server on the stable channel jump to a newer alpha unexpectedly, keep the flag as is and instead make the installer URL resolve the newest tag (a tiny `install.sh` redirect page is not available while Pages is off), and write that choice down.
3. Add a test or workflow check: after a release the job fetches `/releases/latest/download/install.sh` and fails if it is not 200.
4. Update `select_release` tests so the channel behaviour you chose is covered.

## Done when
`curl -fsSL -o /dev/null -w '%{http_code}' -L https://github.com/khangdev20/opsbox/releases/latest/download/install.sh` prints 200 after the next tag, and an existing server on alpha.43 still updates from the panel. Owner check: run it after the tag is built.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 01`. Commit `fix 01: The one-line install command returns 404`, push `dev`, archive this file.
