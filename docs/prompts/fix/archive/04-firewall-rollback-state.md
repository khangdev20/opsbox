# Fix 04. Pending firewall rollback can be lost

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 25: after a confirmed risky change, if detection or restoration fails at the 60 second deadline, or saving the pending state fails, or two risky removals overlap, the undo information can be cleared or lost, so the original reachable state is not restored.

## Steps
1. In the firewall op, find where the pending revert is stored and cleared. Write tests with an injected failure: (a) state save fails after apply, (b) restore fails once, (c) two risky changes in a row.
2. Fixes: persist the undo record BEFORE applying; clear it only after a verified restore; on restore failure keep it and retry with backoff; serialize risky changes (the second is refused with a clear message while one is pending).

## Done when
All three tests pass; a failed restore leaves the undo record and a retry.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 04`. Commit `fix 04: Pending firewall rollback can be lost`, push `dev`, archive this file.
