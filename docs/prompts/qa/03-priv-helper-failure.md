# QA 03. Helper down: messages, banner and no half-made things

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. With the panel open, run `sudo systemctl stop opsbox-priv`.
2. Check: the banner from prompt 03f appears, the message names the restart command, read-only pages still load, Add database and Add app fail with the new text (not the old 'not running' for a timeout).
3. Press Add database with Override ticked: the form keeps every value after the error.
4. Start the helper again: the banner clears without a page reload, and no database or container was left behind (`docker ps -a`, Apps list).
5. Look at the log line `privileged helper call failed`: it has op name, elapsed time and request id.

## Pass when
All five checks hold; any miss is a tiny fix or a `Found, not fixed` entry.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 03: Helper down: messages, banner and no half-made things`, push `dev`, archive this file.
