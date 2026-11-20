# Fix 26. Give the proxy its own system user and socket

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`serve` and `proxy` both run as `opsbox`, so a compromise of the internet-facing proxy has everything `serve` has. Step 1 of 2 (fix 27 is step 2).

## Steps
1. Test first: a client on the proxy socket with the `opsbox` uid is refused, and the reverse.
2. Create system user `opsbox-proxy`; run `opsbox-proxy.service` as it; add a second socket `/run/opsbox/priv-proxy.sock` (root, group `opsbox-proxy`, 0660). `priv` records which socket a connection used plus `SO_PEERCRED`.
3. Make `install.sh` and `opsbox update` create the user and migrate ownership idempotently.

## Done when
Two sockets and two uids; migration is idempotent; behaviour otherwise unchanged.

Commit `fix 26: Give the proxy its own system user and socket`, push `dev`, archive this file.
