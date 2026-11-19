# Fix 03. Removing the panel's firewall allowance can lock the admin out

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 25: with a default-drop firewall, `DELETE /api/v1/firewall/rules/{id}` for the rule that allows the panel port (for example 8443) is not treated as risky, so no confirmation and no timed rollback (`revertAt` is absent). Only SSH and CIDR rules are guarded.

## Steps
1. In `crates/opsbox-priv/src/ops/` (firewall) and `crates/opsbox-api/src/` (firewall handlers) find the lockout guard. Write a test: default-drop table, one allowance for the panel port, delete it without `confirm`: expect `ConfirmRequired`, and with confirm expect a pending revert.
2. Extend the guard so the panel port (the current listener port, also the watchdog recovery port) counts as protected the same way SSH does. Include rules that would shadow it (a broader drop above it).
3. The UI already shows the typed confirmation for risky rules; check it appears for this case.

## Done when
The new test passes; deleting the panel-port rule needs the typed confirmation and reverts by itself after the timer.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 03`. Commit `fix 03: Removing the panel's firewall allowance can lock the admin out`, push `dev`, archive this file.
