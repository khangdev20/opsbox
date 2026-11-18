# Status

Last completed prompt: `docs/prompts/archive/17-alpha-release.md` (alpha release docs + landing). Not committed / not pushed (per session request). Prior: `16s`.

## What works (17)
- **SECURITY.md** at repo root: private reporting via GitHub advisories, response targets, alpha security properties. Pointer at `docs/SECURITY.md`.
- **Threat model**: trust-boundary summary added atop `docs/THREAT_MODEL.md`.
- **README**: pitch, alpha notice, Ubuntu 22.04/24.04 + Debian 12, one-line install, screenshots, links to `docs/SECURITY.md` and `docs/THREAT_MODEL.md`.
- **Landing (`site/index.html`)**: install command unchanged (`…/releases/latest/download/install.sh`); OS copy + FAQ aligned; Security / threat-model links; OpenGraph + Twitter card tags for `og.svg` (1200×630 placeholder). `site/og.svg` added.

## Partial / Found, not fixed
- Shell and file-delete unavailable: live `docs/prompts/17-alpha-release.md` is a stub; archive copy is at `docs/prompts/archive/17-alpha-release.md`. Owner: `git rm docs/prompts/17-alpha-release.md` then `git add docs/prompts/archive/17-alpha-release.md`.
- `site/og.png` not written (meta tags use `site/og.svg`). Generated card: `~/.cursor/projects/Users-lenhutkhangvo-Developer-opsbox/assets/og.jpg` — convert to `site/og.png` and retarget meta tags.
- 60-second screen recording (install → claim → deploy → domain) not produced.
- GitHub repo social preview: owner sets in repo settings.
- Merge `dev` → `main`, first public alpha tag/notes, pin release in README: **ask owner first** (not done).

## Next
- Owner: `git rm` live 17 stub; convert `og.jpg` → `site/og.png`; set GitHub social preview.
- Owner: approve merge `dev` into `main`, then tag next `v0.1.0-alpha.N`, mark release notes as first public alpha, pin in README.
- Optional: 60s demo recording for the landing / README.
- Commit when asked: `prompt 17: alpha release`. Do not push unless asked.
- VM check: one-line install from README on Ubuntu 22.04/24.04 or Debian 12; claim with setup token; refuse setup after claim (404).
