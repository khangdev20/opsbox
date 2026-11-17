# opsbox

Self-hosted all-in-one server panel in the spirit of Railway / Render, installed once on any Linux instance. Goal: never SSH into the instance again.

**Status: alpha, UI complete on mock data, backend not started.**

## Layout
- `web/` the whole interface: plain HTML, CSS and ES modules, no build step. `web/js/api.js` is the mock backend; each export maps to one REST endpoint (see `docs/API.md`).
- `web/vendor/monaco/` the file editor (VS Code's editor core), self-hosted, loaded on first use.
- `src/` Rust backend, placeholder. Phase 1 embeds `web/` into one binary and implements the API.
- `docs/ROADMAP.md` what is built and what comes next.
- `docs/ARCHITECTURE.md` the decisions; `docs/BACKEND_SPEC.md` the full backend spec and implementation plan.

## Try the UI
`cd web && python3 serve.py` (caching off, so a normal reload shows edits), open http://localhost:8099. Demo 2FA code: any 6 digits. Setup token: `7QK2-9XMD-4TFA`.

## Principles
Binds to localhost by default, no telemetry, single static binary, UI embedded, panel meant to sit behind a VPN or IP allowlist while alpha.

License: Apache-2.0. Monaco is MIT, IBM Plex is OFL (see `web/fonts`, `web/vendor/monaco`).
