# Status

Release target: `v0.1.0-alpha.43`, tagged from `dev` after CI passes.

## Completed
- Fix 13: coarse-pointer checkboxes/radios retain 22px artwork with 44px hit areas and labels; Files/Updates selection spans rows while file links remain tappable and desktop density is unchanged.
- Fix 12: empty folders, updates, services, jobs, unit-filtered journal results and snapshots use shared guidance with working actions; backup guidance respects roles.
- Fix 11: failed Apps domain requests show an inline error and Retry; failures persist per service until Retry or the next poll, without a repaint loop.
- Fix 10: shared HTTP requests have 30-second deadlines (120 seconds for known slow endpoints); awaited mounts and System pane errors show Retry instead of retaining the skeleton.
- Fix 09: native nft address/port element timeouts expire emergency SSH after one hour even while the watchdog is stopped or restarted; renewal reuses the rule.
- Fix 08: per-unit systemd restart deltas trip safe mode at five restarts in ten minutes even with healthy probes; the three-failed-probes rule remains.
- Fix 07: stdin DB restores verify a complete dump in a private temporary file before replay; PostgreSQL replay is transactional.
- Fix 06: import preview/apply validate the checked-in schema; per-secret task results fail on missing or unrestored secrets and skip credentialless destinations.
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
- Fix 13: test-first Chromium regression now passes at 390px with coarse-pointer emulation, real touch activation, disabled controls and scrolled-row selection; 1440px desktop screenshot comparison passes. Git metadata writes remain blocked; owner Git/VM completion is in QA 11.
- Fix 12: eight new tests failed before implementation; all 36 focused/related Node regressions now pass. Source checks pass; browser localhost binds/headless startup and Git metadata writes are blocked here. See QA 13 for visual/VM and commit/push/CI/release completion.
- Fix 11: test-first HTTP 500 request-count regression failed before the fix; 4 Node/DOM regressions now pass, covering three-second request bounds, per-service failures, Retry, polling recovery and in-flight deduplication. Git staging/pull are blocked by read-only metadata; owner checks are in QA 13.
- Fix 10: 26 Node/DOM tests and five existing Node checks pass; four browser checks hit sandbox localhost-bind restrictions. Live VM and Git completion remain owner checks in QA 09.
- Fix 09: test-first controlled-clock IPv4/IPv6 expiry/restart, renewal and failure tests pass; watchdog suite 20 pass/2 sandbox socket-bind failures; formatting/Clippy pass. Live nft and Git completion remain owner checks in QA 26.
- Fix 08: regression failed before implementation; 17 watchdog tests pass, with 2 existing sandbox socket-bind failures. Formatting/Clippy pass; live systemd and Git completion remain owner checks in QA 26.
- Fix 07: test-first late-dump regression plus success/empty staging checks pass (6 backup tests); requested helper suite: 215 pass/2 existing setuid/sysctl environment failures; Clippy/formatting pass. Git metadata is read-only; host commit/push, CI/release and live VM checks are in QA 18.
- Fix 06: schema, unavailable-helper and Compose HTTP regressions pass; full import suite has 3 pass/5 sandbox Unix-socket bind failures. Clippy/formatting pass. Git pull/staging are blocked by read-only metadata; owner CI/release and real-helper verification are recorded in QA 24.
- Fix 05: test-first plaintext leak regression and helper-free HTTP export/import round trip pass; Clippy/formatting pass. Requested crate tests hit sandbox TLS binds (API 71 pass/4 fail); helper tests hit existing setuid/sysctl limits (212 pass/2 fail). See QA 24.
- Fix 04: 21 focused firewall tests, crate Clippy, formatting and whitespace checks passed, including test-first save/restore/overlap regressions; live backend fault injection remains an owner check in QA 25.
- Fix 03: helper guard and 2 UI regressions passed; crate Clippy and formatting passed. Full Cargo run attempted: sandbox socket binds block API TLS/HTTP tests; helper suite has existing setuid/sysctl environment failures. See QA 25 for exact checks.
- Linux VM: all 208 opsbox-priv tests passed in 20 consecutive parallel runs after the confinement-race fix; Clippy passed. The memory probe measured 14.37 MiB idle RSS against a 40 MiB limit. Current cargo-deny audit passed.
- Passed frontend DOM checks: package selection/clear/mixed state, tab persistence, selected-only mock install, notification refresh, read/read-all, snooze, keyboard navigation, focus return, text escaping, and empty state.
- Passed JS/CSS syntax, API docs, network UI contracts, source-level live-list checks, update status text, mock API allowlist, Rust formatting, shellcheck, and installer channel checks.
- Browser visual review was not completed: browser automation was unavailable and native browser control conflicted with an active user session.
- Full Linux Clippy, workspace tests, dependency audit, release builds, version stamping, and signed release artifacts are checked by GitHub Actions.

## Next
- Run fix 04/05/06/07/08/09/10/11/12/13 Linux/VM checks and complete pending commit/push from a host terminal (Git metadata is read-only here), then CI/release; next fix is 14 (cleanup scan overlap).
- Verify the alpha release's x86_64/arm64 archives, checksums, signatures, SBOM, and installer on GitHub Releases.
- Smoke-test the table and notification views at desktop/mobile widths on an updated server.
