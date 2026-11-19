# Fix 09. Fallback emergency SSH rule never expires

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 26: when the helper is down and Safe Mode adds an SSH allowance through the nftables fallback, the rule has no expiry; it stays after the one hour deadline and after a watchdog restart.

## Steps
1. Test with a controlled clock: add the fallback rule, advance 61 minutes, run housekeeping: the rule must be gone; also after a simulated watchdog restart (store the expiry with the rule, for example as an nft `timeout 1h` element or a persisted deadline that is checked at start).
2. Prefer the native nft timeout (`add element ... { <ip> timeout 1h }`) so expiry does not depend on the watchdog being alive.

## Done when
The rule disappears after one hour with or without a restart.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 09`. Commit `fix 09: Fallback emergency SSH rule never expires`, push `dev`, archive this file.
