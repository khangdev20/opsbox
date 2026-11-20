# Fix 25. Internet-facing proxy can write the database

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`opsbox-proxy.service` has `ReadWritePaths=/var/lib/opsbox`, which includes `opsbox.db` and `xfer`. The proxy needs only `/var/lib/opsbox/proxy`.

## Steps
1. Check what the proxy really opens: `rg -n 'var/lib/opsbox|state_dir' crates/opsbox-proxy`.
2. Narrow the unit to `ReadWritePaths=/var/lib/opsbox/proxy` and add `InaccessiblePaths` for `xfer` (and the DB if unused).
3. Unit-file test: no `ReadWritePaths` entry is a parent of the DB path. Owner check on the VM: certificate issue and renew still work.

## Done when
The proxy cannot open `xfer` or write the DB; certificates still work.

Commit `fix 25: Internet-facing proxy can write the database`, push `dev`, archive this file.
