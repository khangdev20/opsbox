# QA 32. Cleanup: unused Docker volumes must never be selected by accident

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

Seen on the Cleanup page: the row "Docker volumes not used by any container" shows `4 unused volumes · may contain data · unchecked by default`, yet its checkbox is **checked** (91.4 MB). The text says one thing and the box does the other. Deleting a volume deletes data for good, so this is a safety bug until proven otherwise.

What the code says today: `crates/opsbox-api/src/cleanup.rs` sends `"on": !pending && safe && !warn`, and the volumes item has `safe:false, warn:true`, so the API default is unchecked. `web/js/pages/cleanup.js` fills `picked` from `i.on` on first load and when `fresh`, and `paint()` re-renders rows keyed by `data-key`. So something else checked it: either the user, or the page.

## Check
1. Reload the page (hard reload) with 4 unused volumes: is the box checked on first paint? Record `GET /cleanup` JSON for `docker-volumes` (`on`, `safe`, `warn`).
2. Click Refresh/rescan, wait for the background volume size to arrive (`getCleanupVolumes` sets `bytes`), and watch the box. Does `paint()` or the keyed morph change its state?
3. Morph bug test: tick another row, then let the list refresh while the volumes row changes `pending` or size. Look at the `checked` DOM **property** against the `checked` attribute; keyed morphing that copies attributes only can leave a reused row checked.
4. Run path: with only volumes selected, press Clean. Does the confirmation say which volumes will be deleted? Does it need more than one click?
5. What exactly runs: find the op in `crates/opsbox-priv/src/ops/cleanup.rs` (`docker-volumes`, around line 990) and note whether it prunes every unreferenced volume, named ones included.
6. On `opsbox-vm`: create a database, then remove it choosing "keep data" (or stop and remove its container by hand). Does its volume show up in the unused list? It must not.

## Fix (tiny parts only)
- Make state come from one place: render `checked` from `picked` and set the DOM `.checked` property after every patch (or mark the input so the morph never reuses it). Add a test: first paint with `on:false` is unchecked; after a refresh and after the size arrives it is still unchecked; the user's own tick survives a refresh.
- Items with `warn` can never be pre-selected, in the API or the page, even if a future backend sends `on:true`.
- Volumes confirmation (in `run`): list the volume names and sizes, and for `warn` items require typing `delete volumes` (the same pattern as the capacity override). Disabled Clean button text says why.
- Exclude volumes that opsbox knows (database data, app volumes recorded in the store, anything labelled `opsbox.*`) from the unused list, with a note "N volumes belong to opsbox apps and are kept". If that needs a store lookup across crates, write it up instead.
- Show the list of volumes (name, size, created, last used if known) behind a "View volumes" link so the user can see what "4 unused volumes" are before ticking.

Anything beyond this goes to `docs/QA.md` under "Found, not fixed".

## Second problem, reported by the owner: cleaning does nothing
The owner ran Clean many times and the same 4 volumes (91.4 MB) are still listed. Likely cause, found in code and to be confirmed first:
- `crates/opsbox-docker/src/lib.rs` (`prune`, `DockerPruneKind::Volumes`, about line 1428) calls `prune_volumes` with **no filters**. Since Docker 23 (API 1.42), the engine prunes only *anonymous* volumes unless the filter `all=true` is sent. Docker on the server is 29.10.
- The scan (`dangling_volumes_count`, `volume_sizes` via `system df`) counts every volume no container uses, **named ones too**.
- So the scan offers 4 named volumes, the prune removes none of them, reports 0 bytes freed with no error, and the next scan shows the same 4.

Check (5 minutes, on `opsbox-vm`): `docker volume create keepme; docker volume ls -f dangling=true` then run Cleanup with only volumes ticked; `docker volume ls` still lists `keepme`. Then `docker volume prune -f` (no `--all`) leaves it too, while `docker volume prune -f --all` removes it. Record the Docker version and these three outcomes in `docs/QA.md`.

Do NOT fix this by adding `all=true` to the prune: that deletes every unused named volume at once, including ones that hold a database. Tiny fix for this prompt:
- After `run`, compare the scan before and after. If a selected item freed 0 bytes and the same count remains, the toast and the row say plainly: "Docker only removes anonymous volumes automatically. These 4 volumes are named and were kept. Review them one by one." and link to the volume list (prompt 33).
- Never report success for an item that freed nothing and changed nothing.
The real fix (list, choose, remove by name, with protection for opsbox-owned volumes) is prompt 33.

## Pass when
After a reload and after the background size arrives, a run that removes nothing says so, and the volumes row is unchecked; ticking it requires a typed confirmation that names the volumes; opsbox-owned volumes are never offered. Add a screenshot pair to `docs/screenshots/qa32/` and one row to `docs/QA.md`.
