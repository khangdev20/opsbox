# Fix 08. Rapid service crashes can evade safe mode

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 26: the watchdog samples health at an interval; five quick crashes followed by quick restarts can all fall between samples, so safe mode never triggers.

## Steps
1. Find how the watchdog counts failures (`crates/opsbox-watchdog`). Test with a fake clock and a service that restarts quickly: five restarts inside the window must trip safe mode.
2. Fix by counting restarts, not only failed probes: read the unit's `NRestarts` or `InvocationID` change from systemd on each sample and treat each change as a failure event with its timestamp; keep the existing three-failed-probes rule.

## Done when
The fake-clock test trips safe mode on five restarts and does not on one.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 08`. Commit `fix 08: Rapid service crashes can evade safe mode`, push `dev`, archive this file.
