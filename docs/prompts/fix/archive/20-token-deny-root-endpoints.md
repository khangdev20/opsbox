# Fix 20. Full API tokens must not be root

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`deny_token` is missing on jobs add (`system.rs` ~462), file write (`files.rs` ~263) and updates install (`system.rs` ~115), so a leaked `full` token gets root.

## Steps
1. Test first: a `Full` token gets 403 on every root-equivalent route (jobs, file write/delete/move, packages, updates install, reboot, firewall, SSH keys, users/tokens, restore/import); an Admin session still works.
2. Add `deny_token` to those routes, plus a route-table test that fails when a new mutating route has no token decision.
3. Fix the scope description in UI and docs: 'everything except root-equivalent actions'.

## Done when
A `Full` token cannot create a job, write a file or install packages.

Commit `fix 20: Full API tokens must not be root`, push `dev`, archive this file.
