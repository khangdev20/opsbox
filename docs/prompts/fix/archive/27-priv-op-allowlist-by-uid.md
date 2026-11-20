# Fix 27. Helper authorises by UID only, not by operation

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`dispatch` (`ops/mod.rs` ~69) never asks who may run which op, so any `opsbox`-uid process can send `pty_open` or `job_add` as root. Needs fix 26.

## Steps
1. Add `allowed(caller, op) -> bool` with `Caller = Serve | Proxy`. Proxy gets only what it calls (`rg -n 'PrivOp::' crates/opsbox-proxy`: expected `PortHolders` and `proxy:*` secrets).
2. Test: table-driven with an exhaustive `match` (no wildcard) so a new `PrivOp` forces a decision; denied ops return typed `Forbidden` and are logged.
3. Serve stays allowed for now; add a ROADMAP line: root-grade ops need an admin proof from the API.

## Done when
Proxy can run only its short op list.

Commit `fix 27: Helper authorises by UID only, not by operation`, push `dev`, archive this file.
