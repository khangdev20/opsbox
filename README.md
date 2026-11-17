# opsbox

Self-hosted all-in-one server panel in the spirit of Railway / Render, installed once on any Linux instance. Goal: never SSH into the instance again.

**Status: alpha. Phase 1 server management is implemented and tested on Ubuntu 24.04 arm64; later-phase screens retain labelled sample data.** See [verification](docs/VERIFY.md) and [current status](docs/PHASE1_STATUS.md).

## Install

```sh
curl -fsSL https://github.com/khangdev20/opsbox/releases/latest/download/install.sh | sh
```

Re-running the installer upgrades an existing installation in place while keeping data and configuration. Pass `--channel beta` for pre-releases or `--version vX.Y.Z` for a specific release. See [docs/RELEASING.md](docs/RELEASING.md) for release signing and verification details.

## Layout
- `web/` the whole interface: plain HTML, CSS and ES modules, no build step. `web/js/api.js` selects real or sample data; each export maps to one REST endpoint (see `docs/API.md`).
- `web/vendor/monaco/` the file editor (VS Code's editor core), self-hosted, loaded on first use.
- `crates/` Rust workspace: server, privileged helper, auth, metrics, storage and shared protocol. The release binary embeds `web/`.
- `docs/ROADMAP.md` what is built and what comes next.
- `docs/ARCHITECTURE.md` the decisions; `docs/BACKEND_SPEC.md` the full backend spec and implementation plan.

## Try the UI
`cd web && python3 serve.py` (caching off, so a normal reload shows edits), open http://localhost:8099. Demo 2FA code: any 6 digits. Setup token: `7QK2-9XMD-4TFA`.

## Principles
Binds to localhost by default, no telemetry, single static binary, UI embedded, panel meant to sit behind a VPN or IP allowlist while alpha.

License: Apache-2.0. Bundled libraries and fonts retain their licences under `web/vendor` and `web/fonts`.
