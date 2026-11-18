# 16k. Users: display names and real 2FA state in the account menu

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-store` (users table migration), `crates/opsbox-api/src/users.rs`, `session.rs` (session payload), `crates/opsbox-api/src/setup.rs` (first admin), `web/js/shell.js` (account menu), `web/js/pages/access.js` (users list, invite), `web/js/pages/setup.js`, `web/js/pages/invite.js`, `docs/API.md`.

Today: the account menu shows the sign-in name as the "name" and always says "2FA on" (16d note: no display names in the backend).

Do:
- Add `display_name` (optional, max 64 chars, trimmed, no control characters) to users. Setup and invite forms ask for it (optional); users can change their own in "Profile and security"; Owners can change anyone's.
- Session endpoint returns `{ user, displayName, role, totp: { enabled, method } }`. The menu shows display name (fallback: username), username, role, and the real 2FA state; initials come from the display name.
- Audit log shows `display name (username)` where the actor is shown.

Tests: migration on an existing database; validation; the session payload; a user without 2FA (if allowed by policy) shows "2FA off" with a link to enable it.

Verify on `opsbox-vm`: set a display name, the menu and audit log show it. Record in `docs/STATUS.md`.
