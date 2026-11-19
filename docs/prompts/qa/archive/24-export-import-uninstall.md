# QA 24. Export, uninstall, reinstall, import

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Export configuration from Settings; check the file has no plaintext secrets (or is encrypted) and a version header.
2. Run `opsbox uninstall` choosing to keep apps and data; confirm containers keep running and files remain, and opsbox units and the binary are gone.
3. Reinstall and import the export: settings return, apps are recognised, the admin can sign in with the old credentials or the documented new claim flow.

## Pass when
No data loss in either direction; secrets handled as documented.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 24: Export, uninstall, reinstall, import`, push `dev`, archive this file.
