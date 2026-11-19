# QA 01. Fresh install on a clean VM

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. On a fresh `opsbox-vm` snapshot (or a new Ubuntu 24.04 VM), run the install command from `README.md` exactly as written.
2. Check: exit code 0, `systemctl is-active opsbox opsbox-priv` both `active`, `opsbox status` prints a version and a URL, port 80/443 or the documented fallback is listening (`ss -ltnp`).
3. Open the printed URL; the claim page shows, a wrong setup token is refused, the right one creates the admin.
4. Run the install command a second time: it must be safe (no data loss, no duplicate units).

## Pass when
Installer prints a clear error for each failure; no step needs a second attempt; nothing is left half-installed after a failure (cut the network once with `sudo ip link set <if> down` mid-download and check).

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 01: Fresh install on a clean VM`, push `dev`, archive this file.
