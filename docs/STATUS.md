# Status

Last completed prompt: `docs/prompts/archive/03c1-site-logo.md` (landing site OpsBox mark + wordmark, PNG favicons; no release tag — `site/` only). Prior: `16f` (tag `v0.1.0-alpha.20`).

## What works (03c1)
- Landing nav/footer logo matches the panel: `img/logo-mark-128.png` + `Ops<b>Box</b>`, `Box` via `--brand` (`#FC5801`); accessible name on the home link.
- Favicons use PNG marks (`logo-mark-64` / `256`); unused `site/favicon.svg` removed.
- Relative `img/` paths work for GitHub Pages project URL `/opsbox/`.

## Verified
- Prompt 03c1: logo/favicon markup and `--brand` token aligned with `web/css/app.css` `.brand`; `favicon.svg` unreferenced and removed.
- Earlier: prompt 16f notification bell / attention UI (`tools/check-notifications-ui.mjs` passed).

## Partial / Found, not fixed
- Screenshots use demo data (`?mode=mock`); real VM shots need Owner + 2FA session state.
- No display names in the backend; "2FA on" is true for every browser session.
- Earlier open items remain: live DB/R2/MinIO/SFTP/alert-channel checks, pgweb path prefix, Capacity override click-through, idle browser CPU profiling, Caddy/Traefik live cutovers.
- `capacity_http::queued_workloads_block_database_resize_pooler_and_memory_adding_extensions` fixture issue (see DECISIONS) unchanged.

## Next
- `docs/prompts/16e-transport-security.md` (lowest remaining), then `16g-capacity-page.md` and the rest of the 16* release-prep series, then `17-alpha-release.md`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`.
- Check: open `site/index.html` (or `python3 -m http.server` in `site/`) — logo and favicon in both themes at ~1440px and ~390px; unused `favicon.svg` gone.
