# QA 08. Roles: deployer and viewer cannot do admin things

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Create a deployer and a viewer (invite flow) on the VM.
2. With each session, call from `curl`: terminal WebSocket, files outside app dirs, system updates, firewall, user management, backups restore, reveal secrets.
3. Expected per spec section 6: deployer has no terminal, no system, no files outside app dirs; viewer is read only.
4. Check each refusal is 403 with a sentence, and each is in the audit log.

## Pass when
No privilege escalation; every handler enforces its permission, not only the UI.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 08: Roles: deployer and viewer cannot do admin things`, push `dev`, archive this file.
