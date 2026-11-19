# QA 06. No secret in any API response

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Create a database, an app with env vars, a GitHub connection and a backup destination on the VM.
2. Fetch every list and detail endpoint of those with `curl` and `grep -i` for the known values (password, token, secret key, private key, connection URL).
3. Check: values are masked (`••••`) and only the explicit reveal endpoint returns them, with an audit entry and `no-store`.
4. Check logs (`journalctl -u opsbox*`) and the audit log for the same strings.

## Pass when
Zero plaintext hits outside the reveal endpoint.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 06: No secret in any API response`, push `dev`, archive this file.
