# QA 22. Cleanup page speed

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Fill the VM with junk (old images, build cache, logs). Open Cleanup and time the first paint and the scan with `performance.now()` and the network panel.
2. Check: first paint under 1 second with a skeleton, the scan runs in the background, requests do not pile up when the page is reopened, deleting shows progress.
3. Reclaim something and confirm the numbers drop.

## Pass when
Page is usable in under 1 s; no request runs longer than 30 s without progress.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 22: Cleanup page speed`, push `dev`, archive this file.
