# QA 09. Every page: zero console errors, zero failed requests

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Open each route in a real browser (Playwright script, 5 minutes max) with the real API, and record console errors, page errors and failed or 4xx/5xx requests.
2. Routes: overview, apps, containers, databases, backups, alerts, files, terminal, cleanup, system (all tabs), access, security, settings, login, setup.
3. Check: none, except expected 401 on the login page. List `404` assets (fonts, icons, source maps).
4. Fix the tiny ones (wrong path, missing `await`, undefined variable).

## Pass when
Report is zero errors on all routes, or a short list under 'Found, not fixed'.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 09: Every page: zero console errors, zero failed requests`, push `dev`, archive this file.
