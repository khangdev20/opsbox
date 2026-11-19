# QA 25. Firewall cannot lock the admin out

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. In System > Firewall try to block the panel port and the SSH port (or the port of the current session).
2. Check: the guard refuses or asks for an explicit, typed confirmation and offers a timed rollback; applying a rule that would cut the session auto-reverts after the timer.
3. Emergency SSH access toggle works and is audited.

## Pass when
You cannot cut yourself off by accident.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 25: Firewall cannot lock the admin out`, push `dev`, archive this file.
