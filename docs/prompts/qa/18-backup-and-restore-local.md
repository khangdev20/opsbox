# QA 18. Local backup and restore round trip

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Create a database with 5 rows and an app volume with a file. Run a backup to the local destination.
2. Delete the rows and the file, then restore as a copy and in place.
3. Check: data is back, the snapshot is checksum-verified, the audit log has both actions, nothing leaks the encryption key.
4. Fail safely: restore a tampered snapshot; it must be refused with a clear message.

## Pass when
Round trip is exact; tampering is detected. R2 and S3 are owner checks (rule 7).

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 18: Local backup and restore round trip`, push `dev`, archive this file.
