# Fix 29. Audit trusts the caller's actor string

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`PrivRequest.actor` is free text (`privilege.rs` ~34) and `priv` logs it as given (`server.rs` ~376).

## Steps
1. Test first: a request with `actor: "alice"` is logged with the real `peer_uid`, `pid` and `claimed_actor=alice`.
2. Change the log line only; do not change the proto. Note in `docs/THREAT_MODEL.md` that actor is advisory.

## Done when
Every helper log line carries the real uid and pid.

Commit `fix 29: Audit trusts the caller's actor string`, push `dev`, archive this file.
