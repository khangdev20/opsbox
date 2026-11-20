# Fix 17. Docs say /health/ready but the route is /readyz

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 04: prompts and docs use `/health/ready`; the real route is `/readyz` (`crates/opsbox-api/src/lib.rs`). People following the docs get 404.

## Steps
1. `rg -n 'health/ready' .` over docs, scripts, prompts and web; list every hit.
2. Pick one: fix the docs to `/readyz`, and add a `/health/ready` alias only if something external already calls it. Update `docs/prompts/archive/03f-*.md` text only if it is referenced by tooling; otherwise leave archived prompts alone.

## Done when
No document tells people to call a route that does not exist; a one-line test hits the documented route and gets 200 or 503, never 404.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 17`. Commit `fix 17: Docs say /health/ready but the route is /readyz`, push `dev`, archive this file.
