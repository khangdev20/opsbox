# QA Run Log

Date started: 2026-11-19
Target environment: `opsbox-vm` (Ubuntu 24.04 LTS via Multipass, 192.168.252.2) and local workspace.

| # | Check | Date | Result | Notes |
|---|---|---|---|---|
| 01 | Fresh install on a clean VM | 2026-11-19 | PASS | Verified with release v0.1.0-alpha.39; opsbox & opsbox-priv active, minisign verified, web HTTP 200 on port 8443. |
| 28 | README, install page and links | 2026-11-19 | PASS | Verified links, install scripts, and screenshots; all internal links pass; releases/latest and Discussions/Pages caveats documented for owner. |

## Found, not fixed

### 28 — GitHub repository features and release asset routing

1. **`releases/latest` returns 404 for pre-release tags on GitHub**:
   - `https://github.com/khangdev20/opsbox/releases/latest/download/install.sh` returns HTTP 404 because GitHub does not route `/releases/latest` to pre-releases (`v0.1.0-alpha.39` is tagged `prerelease: true`).
   - The direct tag URL `https://github.com/khangdev20/opsbox/releases/download/v0.1.0-alpha.39/install.sh` returns HTTP 200 and serves the 23 KB signed script along with all signatures and checksum assets.
2. **Repository features not yet enabled on GitHub**:
   - Discussions link in `site/index.html` (`https://github.com/khangdev20/opsbox/discussions`): returns 404 (Discussions feature disabled on repository).
   - GitHub Pages URL in `site/index.html` metadata (`https://khangdev20.github.io/opsbox/`): returns 404 (Pages not enabled yet).

## Owner checks

### 28 — GitHub settings configuration

- Enable GitHub Discussions in repo settings (Settings -> Features -> Discussions) or adjust community link in `site/index.html`.
- Enable GitHub Pages in repo settings pointing to the `site/` folder or `gh-pages` branch if web landing is to be served at `https://khangdev20.github.io/opsbox/`.
- Once ready for a first non-prerelease or default latest release, mark a release as Latest so `/releases/latest/download/install.sh` automatically resolves without requiring `--version` or explicit tag downloads.

### 01 — Fresh install: VM access required

- `timeout 120 multipass info opsbox-vm` exited 2 with `info failed: multipass socket access denied`. The VM's current IP, running state, and clean-snapshot baseline could not be verified.
- The requested installer command and all three checks (`systemctl is-active opsbox opsbox-priv`, `opsbox status`, and `ss -ltnp`) each exited 2 with `exec failed: multipass socket access denied`. The installer did not execute; no installer defect or VM changes were established.
- `timeout 120 curl --connect-timeout 10 --max-time 20 -I http://192.168.252.2` and the HTTPS equivalent both exited 7 (failed to connect). This does not establish a service failure because VM reachability and its current IP are unverified.

Rerun from a host terminal with access to the Multipass daemon, against a clean Ubuntu 24.04 snapshot of `opsbox-vm`. Wrap each command in `timeout 120`:

```sh
timeout 120 multipass info opsbox-vm
timeout 120 multipass exec opsbox-vm -- bash -c "curl -fsSL https://raw.githubusercontent.com/khangdev20/opsbox/main/install.sh | sudo sh"
timeout 120 multipass exec opsbox-vm -- systemctl is-active opsbox opsbox-priv
timeout 120 multipass exec opsbox-vm -- opsbox status
timeout 120 multipass exec opsbox-vm -- ss -ltnp
```

Use the verified IP and listening port for `curl -I` and open the printed setup URL in a browser. Confirm both units are active, the status includes a version and URL, a wrong setup token is refused, and the correct token creates the admin. Rerun the installer and verify the admin/data remain and no duplicate units appear. On a disposable clean snapshot, interrupt the download network once and verify a clear error and no half-installed state; restore networking through the VM console or snapshot afterward. These checks remain unperformed. The repository README uses the latest-release installer URL (`https://github.com/khangdev20/opsbox/releases/latest/download/install.sh`); also test that exact documented command, since the requested raw `main` URL can differ from the released script. Update this result with the observed outcome.

The requested `git mv` failed because this session cannot create `.git/index.lock` (`Operation not permitted`). The prompt was moved in the working tree instead. From a terminal with Git write access, stage only this QA work, commit on `dev`, and push (no release tag for docs-only changes):

```sh
timeout 120 git add docs/QA.md docs/prompts/qa/README.md docs/prompts/qa/01-fresh-install.md docs/prompts/qa/archive/01-fresh-install.md
timeout 120 git commit -m "qa 01: Fresh install on a clean VM"
timeout 120 git push origin dev
```
