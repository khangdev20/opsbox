# 03e. Self-update: say why nothing is offered

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-update/src/release.rs` (`check_latest`, `select_release`), `crates/opsbox-api/src/update.rs` (snapshot), `web/js/pages/settings.js` (opsbox updates section), `docs/API.md`. Depends on 03d (version stamping and channel).

Today the panel only says "up to date" whatever the reason, which hid the 03d bugs for weeks.

Do:
- `check` returns why nothing is offered: no releases on this channel, newest is older or equal, network error, or rate limited. The UI shows it ("No stable release yet; switch to beta to get alpha releases") instead of only "up to date".
- Show "last checked HH:MM" and the channel next to the status.
- GitHub API: send a User-Agent, handle 403 rate limits with a clear message and backoff, cache for 1 hour, and "Check now" bypasses the cache.


Tests: each reason (no release on channel, newest older or equal, network error, rate limited) is returned by `check` and rendered with its own text; the cache is used within an hour and bypassed by "Check now"; a 403 with rate-limit headers backs off until the reset time.

Verify on `opsbox-vm`: switch the channel to stable and see "No stable release yet"; switch back to beta and see the newest alpha or "up to date" with the last checked time; block outbound HTTPS briefly (firewall rule) and see the network error text. Record in `docs/STATUS.md`.
