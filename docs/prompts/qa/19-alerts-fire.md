# QA 19. Alerts fire and notify

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Create a rule that triggers on purpose (disk or memory threshold low, or stop a container).
2. Check: the alert appears in the bell (16f) within the check interval, resolves when fixed, is not duplicated every minute, and honours the quiet hours setting.
3. Email, Telegram and webhook channels: send a test from the panel; for real channels use a local webhook receiver (`python3 -m http.server`) and verify the payload; real services go to 'Owner checks'.

## Pass when
One alert per incident, one resolve, test sends work.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 19: Alerts fire and notify`, push `dev`, archive this file.
