# QA 20. Terminal behaves like a real shell

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Open Terminal as the default user: `whoami`, `pwd`, `top` then `q`, `vim` then `:q`, resize the window and check `stty size`, paste a long line, run `exit` (the session closes cleanly).
2. Open 5 terminals at once: the limit message is clear; closing the tab ends the shell (`ps` shows no orphan).
3. Root requires the confirmation; a deployer cannot open one.
4. Idle for the documented timeout and confirm it is closed.

## Pass when
No orphan shells, no stuck sessions.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 20: Terminal behaves like a real shell`, push `dev`, archive this file.
