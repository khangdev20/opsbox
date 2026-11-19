# QA 04. serve and priv version match after update

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. After an update on the VM compare `opsbox status`, `/health/ready` (`curl -sk https://127.0.0.1:8443/health/ready`) and `systemctl show opsbox-priv -p ExecMainStartTimestamp -p ExecStart`.
2. Check: both processes restarted at the update time, `privileged_helper` is `ok`, both report the same version.
3. Replace the `priv` binary with the previous one in a throwaway slot (or fake the version in a test) and confirm `/health/ready` returns `version mismatch` with both versions and the panel banner shows.

## Pass when
Skew is detected and shown, never silent.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 04: serve and priv version match after update`, push `dev`, archive this file.
