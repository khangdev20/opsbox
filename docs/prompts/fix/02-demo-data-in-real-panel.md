# Fix 02. Demo values leak into the real panel

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 10 FAILED: in http mode the panel can still show fake data. `web/js/shell.js` starts `hostName` as `srv-sgn-01` and keeps it if the request fails; `web/js/pages/configure.js` falls back to the repo `acme/shop`; `web/js/pages/settings.js` builds a fake GitHub identity (`acme-co`, `opsbox-demo`, `panel.example.com`) for a truthy non-object setting; `web/js/api.js` still exports mock `buildLog` (listed in `REAL` by mistake), `termExec` and `rotateOutKey` that can answer in http mode.

## Steps
1. Write a DOM or Node check that loads these modules with `MODE='http'` and a failing API, and asserts none of the strings `srv-sgn-01`, `acme/shop`, `acme-co`, `opsbox-demo`, `panel.example.com` can appear, and that the three mock exports throw a clear 'needs a real server' error in http mode.
2. Fix each: neutral loading text (`This server`) until the host name arrives; no default repo; real GitHub state only; remove `buildLog` from `REAL` or point it to the real endpoint; guard `termExec` and `rotateOutKey` with the same pattern `openTerminal` uses.
3. Keep demo mode (`MODE !== 'http'`) working: its sample values stay.

## Done when
The check passes, demo mode still renders its samples, and a real server never shows a sample name even when a request fails.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 02`. Commit `fix 02: Demo values leak into the real panel`, push `dev`, archive this file.
