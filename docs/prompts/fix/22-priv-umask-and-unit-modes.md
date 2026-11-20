# Fix 22. Root helper writes files with a loose umask

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`opsbox-priv.service` has no `UMask`; `jobs.rs` (~128) writes unit files with `fs::write`, so they can be 0644 and expose job commands.

## Steps
1. Test first: after `JobAdd` the unit and timer files are 0600.
2. Write them with explicit mode 0600 and atomic rename; add `UMask=0077` to the service.
3. `rg -n 'fs::write|File::create' crates/opsbox-priv/src`: set explicit modes only where sensitive data is stored.

## Done when
New job unit files are 0600 and the service has `UMask=0077`.

Commit `fix 22: Root helper writes files with a loose umask`, push `dev`, archive this file.
