# QA 28. README, install page and links

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Extract every URL and command from `README.md`, `site/index.html` and `docs/*.md`. `curl -sI` each URL (timeout 10): none returns 404.
2. Run the one-line install command's first lines in a dry way (`curl -fsSL <url> | head -20`) and confirm the script, checksum and signature files exist on the latest GitHub release.
3. Screenshots in the README show the current design (logo `#FC5801` Box, sidebar groups).
4. Fix broken links and stale text.

## Pass when
No dead link, no stale screenshot, the install URL serves the newest signed script.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 28: README, install page and links`, push `dev`, archive this file.
