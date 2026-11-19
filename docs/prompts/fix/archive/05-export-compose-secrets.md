# Fix 05. Export leaks inline Compose credentials

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 24: exporting configuration writes the raw Compose YAML into the JSON; an inline `environment:` password is in plaintext, with or without the passphrase.

## Steps
1. Test: create a Compose stack with an inline credential containing the marker `QA_SECRET_MARKER_123`, export once with and once without a passphrase, and assert the marker is absent from both files (or only present inside the sealed block).
2. Fix in the export code (find it with `rg -n 'composeProjects' crates`): either replace values of env keys that look secret (`PASSWORD`, `SECRET`, `TOKEN`, `KEY`, `URL` with credentials) with `${SECRET:name}` placeholders and store the real value in the sealed `secrets` list, or seal the whole YAML. Import must restore it.
3. Round trip test: export then import restores the same Compose behaviour.

## Done when
The marker never appears in plaintext in either export, and import restores the stack.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 05`. Commit `fix 05: Export leaks inline Compose credentials`, push `dev`, archive this file.
