# 18b. Project index for agents, part 2: routes, privileged operations, database and UI maps

Follow `docs/prompts/CONVENTIONS.md`: branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done, commit as `prompt NN: <title>` and `git push origin dev`. **Time box 25 minutes.** Requires 18a (`scripts/gen-index.py`). No release tag.

## Goal
The questions agents ask most are: "which handler serves this URL", "which privileged operation does it call", "which table stores this", "which UI page calls it". Generate those tables so no search is needed. Extend `scripts/gen-index.py` with four more sections, each written to its own file under `docs/index/`, same rules as 18a (deterministic, no line numbers, standard library only).

## 1. `docs/index/routes.md`
One row per HTTP or WebSocket route: `METHOD  /api/v1/path  handler  file  auth`.
- Find them where the router is built (`crates/opsbox-api/src/lib.rs` and the per-module `router()` functions; inspect how `.route(...)` calls are written and parse that form). Include nested `.nest()` prefixes and the capacity middleware's matched routes as a note.
- `auth` = the extractor the handler takes (`Admin`, `Deployer`, `Client`, public, token) so permissions are visible.
- Group by module. Report routes that exist in code but not in `docs/API.md`, and the reverse, at the end under "Drift" (informational, does not fail `--check`).

## 2. `docs/index/privops.md`
One row per `PrivOp` variant from `opsbox-proto`: `Variant  request type  handler fn  file  client method`.
- Variant to handler: the `match` in `crates/opsbox-priv` (server dispatch). Client method: the matching function in `crates/opsbox-priv/src/client.rs` with its timeout if it overrides the default (`with_timeout`).
- List variants that have no client method or no handler as "Unwired".

## 3. `docs/index/store.md`
Tables from the migrations (`CREATE TABLE` statements in `crates/opsbox-store`): table, columns with types, indexes, the migration that created it, and the `Store` methods that touch it (by name, found by scanning for the table name in `crates/opsbox-store/src`). Mark settings keys (`get_setting("...")` string literals) in a "Settings keys" list.

## 4. `docs/index/ui.md`
- Pages: each `web/js/pages/*.js` with its route hash, exported `view`, the `api.*` functions it calls and the CSS files it relies on.
- API layer: every exported function in `web/js/api.js` with `REAL` or `mock-only`, its HTTP method and path when real (parse the `http(...)` calls), and which pages use it.
- Shared UI helpers exported by `web/js/ui.js` (name and one-line summary).
- Mock-only functions are listed first under a "Still mock" heading: that list is the honest backlog.

## Checks
- Run twice: no diff. `--check` fails when you add a dummy route and passes after regenerating (undo the dummy).
- Spot check five rows of each table against the source by hand and fix the parser until all five are right.
- Pick a real task ("add a field to the database create form") and trace it using only the index files: you must reach the route, the handler, the `PrivOp`, the table and the UI page without a single search. Write the trace into `docs/STATUS.md` as the acceptance example.
- Total size of `docs/index/` stays under 500 KB.

## Done when
Four new files regenerate identically and the trace works. Note anything the parser could not handle under "Found, not fixed".
