# 03. Self-update and rollback

Spec: 7.15. Crate: `opsbox-update`. Builds on prompt 02 (signed releases).

Goal: when a new version is published, the user updates from the UI with one click (or automatically), and a bad version rolls back by itself.

Do:
- Version slots `/var/lib/opsbox/bin/opsbox-<version>`, symlink `/usr/local/bin/opsbox`, keep current and previous. Migrate today's single-binary install to slots on first update.
- Check GitHub Releases daily (channel stable or beta), compare semver, download, verify SHA-256 and the minisign signature with the key compiled into the binary, run `opsbox selftest` on the new binary, switch the symlink, restart units in order (priv, then serve).
- Rollback without the watchdog (it comes in prompt 06): a transient systemd unit (`systemd-run`) started by priv before the switch waits 2 minutes, checks `/healthz` and `priv` ping, and switches back and restarts if either fails. Record the outcome in the audit log and show it in the UI.
- Database migrations: refuse to roll back across a migration that is not backward compatible; take a copy of the SQLite file before migrating.
- UI: Settings "opsbox updates" real (current version, channel, latest available with release notes link, Check now, Update now, auto-update toggle with a daily window). Overview attention item when an update is available.
- CLI: `opsbox update [--channel] [--version]`, `opsbox rollback`.

Verify: publish `v0.1.0-alpha.2`, update from the UI on the VM; then publish a deliberately broken `alpha.3` (e.g. serve exits at start) and confirm automatic rollback to `alpha.2` within about 2 minutes.
