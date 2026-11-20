# Fix 16. Deleting the last app leaves its network and directory

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 17: deleting the only app in a project leaves the Docker network and the app directory; injected removal failures can also leave containers or files while the delete reports success.

## Steps
1. Test with a fake Docker client: delete the last app of a project, expect the project network removed when it has no other containers; a removal failure must make the delete task report failure and keep the record.
2. Fix in the delete op: remove container, then volumes only if the user asked, then the empty network, then the app directory; report the first failure and do not delete the database record until all steps succeed (or mark it `delete_failed` with a Retry).

## Done when
Fake-client tests pass for success and for each injected failure.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 16`. Commit `fix 16: Deleting the last app leaves its network and directory`, push `dev`, archive this file.
