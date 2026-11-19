# QA 17. Deploy a Docker image app end to end

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Create an app from the image `nginxdemos/hello`, memory 128 MB, internal only; then attach a test domain via `/etc/hosts` and a self-signed cert.
2. Check: container up, app shows Running, logs open, health check passes, restart works, delete removes container, network and files.
3. Redeploy with a new tag and roll back; the previous version serves again.

## Pass when
Create, redeploy, rollback and delete all work, with no leftover container or directory.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 17: Deploy a Docker image app end to end`, push `dev`, archive this file.
