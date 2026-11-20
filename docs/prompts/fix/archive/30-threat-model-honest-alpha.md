# Fix 30. Security docs must say what is true (docs only)

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
The real boundary is the `opsbox` uid: a compromise of `serve` or `proxy` is root until fixes 26 and 27 land. `docs/THREAT_MODEL.md` (~15) does not say so.

## Steps
1. Update `docs/THREAT_MODEL.md` and add `SECURITY.md` (report via GitHub private advisory; supported: latest alpha) with a short 'Alpha status': treat panel compromise as root, use a VPN or IP allowlist, no Deployer for untrusted people before fix 19, `full` tokens are sensitive, restrict Safe Mode.
2. Align the security text in `README.md` and `site/index.html`. Never write 'least privilege'. Text only.

## Done when
Docs state the real boundary and mitigations, nothing the code does not back.

Commit `fix 30: Security docs must say what is true (docs only)`, push `dev`, archive this file.
