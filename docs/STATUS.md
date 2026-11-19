# Status

Release target: `v0.1.0-alpha.43`, tagged from `dev` after CI passes.

## Completed
- Redesigned shared tables with consistent cell padding, borders, compact action columns, and selected-row states.
- Redesigned System → Updates with select-all/mixed states, selection counts, disabled empty selection, and selection preserved across tab changes.
- Redesigned Notifications with a quieter unread background, storage/security icons, unread count, clearer content/time/action hierarchy, and a detailed empty state.
- Fixed stray commas between notification rows during refresh by returning one safely rendered HTML fragment.
- Fixed the Docker HTTP test's assumption that a missing fixture socket means Docker binaries are absent on the CI host; production behavior is unchanged.

## Verification
- Passed frontend DOM checks: package selection/clear/mixed state, tab persistence, selected-only mock install, notification refresh, read/read-all, snooze, keyboard navigation, focus return, text escaping, and empty state.
- Passed JS/CSS syntax, API docs, network UI contracts, source-level live-list checks, update status text, mock API allowlist, Rust formatting, shellcheck, and installer channel checks.
- Browser visual review was not completed: browser automation was unavailable and native browser control conflicted with an active user session.
- Full Linux Clippy, workspace tests, dependency audit, release builds, version stamping, and signed release artifacts are checked by GitHub Actions.

## Next
- Verify the alpha release's x86_64/arm64 archives, checksums, signatures, SBOM, and installer on GitHub Releases.
- Smoke-test the table and notification views at desktop/mobile widths on an updated server.
