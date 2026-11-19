# QA 02. Update and rollback between two alphas

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Install an older alpha (`--version v0.1.0-alpha.30`) on the VM, create one app or one database.
2. From the panel (System > Updates) check for updates: it must offer the newest alpha and, if it offers nothing, say why (03e).
3. Apply the update, wait for the health check, confirm `opsbox status` shows the new full version (for example `0.1.0-alpha.39`, not `0.1.0`).
4. Run `sudo opsbox rollback`; confirm the old version returns and the app or database is still there.
5. Fail safely: stop `opsbox-priv` right after applying an update and confirm the automatic rollback happens within the documented time.

## Pass when
Both directions work, data survives, and the failure case rolls back by itself.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 02: Update and rollback between two alphas`, push `dev`, archive this file.
