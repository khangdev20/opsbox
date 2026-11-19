# QA 16. Containers: start, stop, restart, logs, remove

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Run three containers (one fails on start). In the panel: start, stop, restart, view logs (live and stop), remove, each on one container.
2. Check: state changes in under 3 seconds, errors say why, logs stream and stop when the dialog closes (no stuck websocket: `ss -tnp | grep opsbox` count returns to normal).
3. Remove asks for confirmation and does not remove volumes unless asked.

## Pass when
Every action works and cleans up after itself.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 16: Containers: start, stop, restart, logs, remove`, push `dev`, archive this file.
