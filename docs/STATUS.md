# Status

Release target: `v0.1.0-alpha.43`, tagged from `dev` after CI passes.

## Completed
- Fix 05: whole Compose YAML is omitted or passphrase-sealed on export, restored exactly on import, and missing source skips stacks and members.
- Fix 04: firewall undo is persisted before risky mutation, verified before clearing, retained with capped retry backoff on restore failure, and protected from overlapping risky changes.
- Fix 03: panel/recovery firewall allowance removal now requires typed confirmation and schedules the existing 60-second undo; default/custom listener, CIDR/IPv6 and unknown-caller regressions added.
- Redesigned shared tables with consistent cell padding, borders, compact action columns, and selected-row states.
- Redesigned System → Updates with select-all/mixed states, selection counts, disabled empty selection, and selection preserved across tab changes.
- Redesigned Notifications with a quieter unread background, storage/security icons, unread count, clearer content/time/action hierarchy, and a detailed empty state.
- Fixed stray commas between notification rows during refresh by returning one safely rendered HTML fragment.
- Fixed the Docker HTTP test's assumption that a missing fixture socket means Docker binaries are absent on the CI host; production behavior is unchanged.
- Matched the isolated loopback memory probe's listener to its HTTP readiness request; production TLS defaults are unchanged.
- Updated CI's cargo-deny to 0.21.0 for CVSS 4.0 advisories; allowed versionless local paths in private workspace crates while keeping registry wildcards denied.
- Mapped Linux openat2 confinement races (EAGAIN) to Refused while continuing to deny the operation; reproduced the prior intermittent Internal error under parallel file tests.

## Verification
- Fix 05: test-first plaintext leak regression and helper-free HTTP export/import round trip pass; Clippy/formatting pass. Requested crate tests hit sandbox TLS binds (API 71 pass/4 fail); helper tests hit existing setuid/sysctl limits (212 pass/2 fail). See QA 24.
- Fix 04: 21 focused firewall tests, crate Clippy, formatting and whitespace checks passed, including test-first save/restore/overlap regressions; live backend fault injection remains an owner check in QA 25.
- Fix 03: helper guard and 2 UI regressions passed; crate Clippy and formatting passed. Full Cargo run attempted: sandbox socket binds block API TLS/HTTP tests; helper suite has existing setuid/sysctl environment failures. See QA 25 for exact checks.
- Linux VM: all 208 opsbox-priv tests passed in 20 consecutive parallel runs after the confinement-race fix; Clippy passed. The memory probe measured 14.37 MiB idle RSS against a 40 MiB limit. Current cargo-deny audit passed.
- Passed frontend DOM checks: package selection/clear/mixed state, tab persistence, selected-only mock install, notification refresh, read/read-all, snooze, keyboard navigation, focus return, text escaping, and empty state.
- Passed JS/CSS syntax, API docs, network UI contracts, source-level live-list checks, update status text, mock API allowlist, Rust formatting, shellcheck, and installer channel checks.
- Browser visual review was not completed: browser automation was unavailable and native browser control conflicted with an active user session.
- Full Linux Clippy, workspace tests, dependency audit, release builds, version stamping, and signed release artifacts are checked by GitHub Actions.

## Next
- Run fix 04/05 Linux/VM checks and complete fix 05 commit/push from a host terminal if Git remains read-only here, then CI/release; next fix is 06 (import schema and key errors).
- Verify the alpha release's x86_64/arm64 archives, checksums, signatures, SBOM, and installer on GitHub Releases.
- Smoke-test the table and notification views at desktop/mobile widths on an updated server.
