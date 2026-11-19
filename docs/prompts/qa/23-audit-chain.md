# QA 23. Audit log integrity

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Perform ten actions (login, create app, change setting, failed login).
2. Check each is in the audit log with user, IP, time and result; filters and CSV export work and match the table.
3. Verify the hash chain with the built-in verify command or endpoint; then tamper with one row in the SQLite file on a copy and confirm verification fails and says where.

## Pass when
Chain verifies when clean and fails loudly when altered.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 23: Audit log integrity`, push `dev`, archive this file.
