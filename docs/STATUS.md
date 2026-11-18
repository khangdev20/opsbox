# Status

Last completed prompt: `docs/prompts/archive/16k-user-display-names.md` (display names + real 2FA in the account menu). Not committed, not tagged (per session request). Prior: `16j` (uninstall).

## What works (16k)
- **DB**: Migration `0013_user_display_name.sql` adds nullable `display_name` on `users` (app validates trim, ≤ 64 chars, no control characters). `User::actor_label()` formats audit actors as `Display (username)`.
- **API**: Optional `display_name` on setup admin, invite, and invite claim. `PATCH /api/v1/users/{name}` for self or owner. `GET /session` returns `{ user, displayName, role, totp: { enabled, method } }` (`method` is `"app"` when on).
- **UI**: Account menu shows display name (fallback username), username, role, real 2FA (link to enable when off); initials from display name. Setup, invite claim, Access profile, and invite forms accept display name. Owners can edit anyone’s name on Access.
- **Docs / tests**: `docs/API.md`; store migration + display_name unit tests; auth password validation; session + PATCH integration test in `security_http`.

## Partial / Found, not fixed
- **cargo test / clippy / generate-api.py not run in this session** (shell was unavailable). Please run: `PATH=/opt/homebrew/opt/rustup/bin:$PATH cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` and `python3 tools/generate-api.py`.
- **Archive:** content is in `docs/prompts/archive/16k-user-display-names.md`; remove the stub `docs/prompts/16k-user-display-names.md` with `git rm` (shell `git mv` was unavailable in this session).
- **Not verified on `opsbox-vm`**. Not checked at 390 px.
- Carried over: 16j uninstall VM check; 03f / 16e / 03e / 03d pending commit/release; import is 16q.

## Next
- Commit 16k (with the pending stack if intended), push `dev`, wait for CI, tag when appropriate.
- Then `16l-audit-ip-and-csv.md`.
- VM check for 16k: set a display name under Profile and security; menu and audit log show `Name (username)`; with 2FA reset (if policy allows) the menu shows “2FA off” with a link to enable. Must fail safely: a viewer `PATCH` of another user’s display name returns 403.
