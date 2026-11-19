# QA 33. Cleanup: choose unused Docker volumes one by one

Follow `docs/prompts/qa/README.md` (time box 20 minutes; if the change grows past about 80 lines in two files, stop after the API part and write the page part up under "Found, not fixed"). Follows QA 32, which found that the blanket volume prune does nothing for named volumes.

## Goal
Replace "prune all unused volumes" by an explicit list: the user sees each unused volume and removes only the ones they choose. Nothing owned by opsbox is ever offered.

## API
- `GET /api/v1/cleanup/volumes/list`: unused volumes from Docker (`volume ls` with `dangling=true`), each `{ name, driver, created, size_bytes (from system df, may be null), anonymous, owner }`. `owner` is `opsbox` when the volume is labelled `opsbox.*`, compose-project-labelled by an opsbox stack, or named in the store for an app or database; those are returned with `protected: true` and cannot be removed here.
- `POST /api/v1/cleanup/volumes/remove` body `{ names: [..], confirm: "delete volumes" }`: Admin only; each name re-validated in `priv` as a typed op (`CleanupRemoveVolumes`), refused if protected, in use at that moment, or not currently unused. Uses `volume remove` without `force`. Returns per-name `{ name, removed, error }` and the audit log gets one entry per removed volume. No change to the existing `docker-volumes` prune id except: it now means "anonymous volumes only" and its row text says so.

## UI (`web/js/pages/cleanup.js`)
- The volumes row opens "View volumes": a table (name, size, created, type anonymous/named, a "kept by opsbox" badge for protected ones, disabled checkbox). Nothing pre-selected, ever.
- A single sticky action "Delete N volumes" opens a confirmation that lists the names and needs the typed phrase `delete volumes`.
- After removal rescan; rows disappear (FLIP list); errors per volume stay visible.
- Mobile: card rows at 390 px (see 16s).

## Tests
- Rust: protected volume refused; in-use volume refused; two removable volumes removed and audited; invalid name (`../x`, empty, too long) refused.
- Browser: nothing pre-selected; confirmation needs the phrase; removed rows vanish.

## Pass when
On `opsbox-vm`: create `keepme` and an unused anonymous volume plus a database volume; the list shows all three, the database one as kept; removing `keepme` works; the database volume cannot be selected. A forged request for the database volume's name is refused with 403 and audited. Screenshot in `docs/screenshots/qa33/`, row in `docs/QA.md`.
