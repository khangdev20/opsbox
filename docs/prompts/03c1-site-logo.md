# 03c1. Landing site: check and commit the new logo

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit and `git push origin dev`. This prompt changes only `site/`, which is not part of the binary: do not tag a release for it.

Context: on 2026-11-19 the landing page was updated by hand, outside the prompt flow, and left uncommitted. The working tree has:
- `site/index.html`: nav and footer logo changed from the text `opsbox<i>_</i>` to the OpsBox mark (`img/logo-mark-128.png`) plus the wordmark `Ops<b>Box</b>`; favicon changed from `favicon.svg` to PNG icons (`img/logo-mark-64.png`, `img/logo-mark-256.png`) and an `apple-touch-icon`.
- `site/styles.css`: `.logo` rules for the image and wordmark (`Box` in `#FC5801`), footer sizes.
- `site/img/` (new, untracked): `logo-mark-64.png`, `logo-mark-128.png`, `logo-mark-256.png`, copied from `web/img/`.
Other files in the working tree (for example under `web/` or `design/`) may belong to another task: do not stage, change or revert them.

Do:
1. Review the diff of `site/index.html` and `site/styles.css`: only the logo, favicon and related styles changed; markup is valid; the logo link keeps an accessible name ("OpsBox home") and the image has `alt=""` because the wordmark text follows it.
2. Make the logo consistent with the panel (`web/css/app.css` `.brand`, `web/js/ui.js` `logo()`): same image, wordmark, colour and spacing. Use a CSS variable for the brand orange instead of the literal if the site has a token file; check the wordmark in light and dark themes.
3. `git rm site/favicon.svg` (no longer referenced; confirm with a grep first).
4. Check the page at 1440 px and 390 px in both themes (nav, mobile menu, footer); no console errors; images load from `site/img/` with correct paths when the site is served from a sub-path (GitHub Pages project URL `/opsbox/`).
5. Commit only these paths: `git add site docs/prompts docs/STATUS.md` then `git commit -m "prompt 03c1: landing site logo" -- site docs/prompts docs/STATUS.md`, push to `origin dev`. `git status` may still show other people's files; it must show nothing under `site/`.

Verify: open `site/index.html` locally (or `python3 -m http.server` in `site/`) and confirm the new logo and favicon in both themes and on mobile width.
