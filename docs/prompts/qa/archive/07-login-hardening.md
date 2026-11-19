# QA 07. Login, lockout, TOTP and session revoke

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Wrong password five times: lockout with `Retry-After`, and the message does not say whether the user exists.
2. Enable TOTP, log out and in: wrong code refused, right code accepted, the same code cannot be reused.
3. Two sessions: revoke one from the panel; the revoked one gets 401 on its next request (server side, not just a cookie delete).
4. Change password: other sessions end.
5. Setup claim after the server is claimed returns 404 and changes nothing.

## Pass when
All hold. Any bypass is a FAIL; fix at once if tiny, otherwise write it up at the top of `docs/QA.md`.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 07: Login, lockout, TOTP and session revoke`, push `dev`, archive this file.
