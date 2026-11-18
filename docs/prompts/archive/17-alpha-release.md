# 17. Alpha release

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Do:
- `SECURITY.md` (private reporting, response targets), short threat model in `docs/THREAT_MODEL.md`.
- README for newcomers: what it is, one-line install, screenshots, alpha warning, supported OS.
- Landing page (`site/`) to GitHub Pages; real install command and links.
- A 60-second screen recording: install, claim, deploy an app, add a domain.
- Social preview: a 1200×630 `site/og.png` (logo, tagline, one panel screenshot) referenced by `og:image` and `twitter:card` in `site/index.html`, also set as the GitHub repository social preview (owner does that step).
- Merge `dev` into `main` (ask the owner first). Do not create a tag named `v0.1.0-alpha`: alphas are already numbered (`v0.1.0-alpha.N`). Tag the merge commit with the next number per CONVENTIONS, mark that GitHub release as the first public alpha in its notes, and pin it in the README.

Verify: a fresh cloud VM, the one-line install from the README, an app live on a domain without SSH after the install step.
