# QA 15. Ports 80/443 already taken

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. On the VM start a container on 80 and 443 (`docker run -d -p 80:80 -p 443:443 nginx`), then install opsbox.
2. Check: opsbox does not crash-loop; it falls back to other ports and says so in the panel.
3. Use 'take over', then undo; use 'move behind opsbox'; each leaves the old site reachable and reversible.
4. `systemctl status opsbox` shows no restart loop (`NRestarts` stays low).

## Pass when
No crash-loop, no lost site, clear choices.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 15: Ports 80/443 already taken`, push `dev`, archive this file.
