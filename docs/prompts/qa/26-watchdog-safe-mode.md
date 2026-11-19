# QA 26. Watchdog and safe mode

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. `sudo pkill -9 -f 'opsbox serve'`: the watchdog restarts it within the documented time; the panel returns without a new login if the session is still valid.
2. Kill it repeatedly (5 times quickly): safe mode starts with a clear page and the recovery action works.
3. Corrupt nothing; just confirm logs say why it restarted.

## Pass when
Self-healing works; safe mode is understandable.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 26: Watchdog and safe mode`, push `dev`, archive this file.
