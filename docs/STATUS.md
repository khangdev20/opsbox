# Status

Last prompt done: **18a — Project index: generator, crate map and symbol map**.
Scripts and documentation only; no release tag. Prior runtime release target remains `v0.1.0-alpha.43`, pending CI/release completion.

## Completed
- 18a: `scripts/gen-index.py` uses Python 3 standard library only, without Cargo, Git or network access; generates `docs/INDEX.md` and 18 crate symbol maps.
- The index covers layout, crate purposes, internal dependencies, non-test LOC, entry files, dependency graph and serve/priv/proxy/watchdog startup paths. The root helper's crate header now states its privilege role.
- Symbol maps cover public declarations/methods and lib re-exports; omit restricted/private items, test-only bodies, ignored/generated sources and excluded directories. Large maps prioritize crate-root re-exports and documented items within 400 lines.
- Generation preserves prose outside INDEX markers, removes obsolete crate maps and enforces size budgets before writing. `--check` prints unified diffs, exits 1 for stale maps and writes nothing.
- Marked 18a done and archived its prompt; no commit or push attempted, as requested.
- Previous fixes 03–09: firewall undo/confirmation and kernel SSH expiry; sealed Compose export/import and per-secret import results; transactional PostgreSQL restore; watchdog restart deltas.
- Previous fixes 10–14: request deadlines, domain retry, actionable empty states, coarse-pointer targets and shared/cancelled Cleanup scans.
- Previous fixes 15–17: image-source settings/redeploy, retryable service cleanup with preserved volumes, and `/health/ready` compatibility with canonical `/readyz`.
- Fix 18: audited the misleading helper error; retained existing 300-second startup/pull deadlines and OOM protection pending live evidence.
- Earlier table, update and notification redesigns, Docker fixture checks, memory-probe listener, cargo-deny update and openat2 race refusal remain implemented.

## Verification
- Run `python3 scripts/gen-index.py` to regenerate; `python3 scripts/gen-index.py --check` passes.
- `PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-gen-index.py`: all 9 tests pass, covering lexical scanning, test exclusion, doc association, dependency kinds/aliases, ignore rules, truncation, sensitive-prose redaction, freshness/idempotency, prose preservation, obsolete maps and size-limit refusal.
- Two actual repository generations changed neither bytes nor mtimes, taking 0.255 and 0.254 seconds. A temporary public symbol made `--check` exit 1 with unified diffs and zero writes; restored source, regenerated and rechecked successfully.
- `docs/INDEX.md`: 7,491 bytes / 135 lines. Combined INDEX and symbol maps: 166,430 bytes; each symbol file stays below 400 lines. No absolute home paths appear in generated output.
- Navigation review identifies backups as `opsbox-backup`, Docker client ownership as `opsbox-docker`, and root system operations as `opsbox-priv`.
- `git diff --check` passes. Cargo is absent from this environment, so workspace formatting, Clippy and Rust tests could not run; the sole Rust edit is a crate-doc one-liner.
- Git pull was attempted on `dev` but read-only metadata denied `.git/FETCH_HEAD`; no commit, push or release action was attempted.
- Prior focused fix regressions passed as recorded in their archived prompts. Sandbox socket binds and setuid/sysctl restrictions prevented some helper-backed HTTP, TLS and privileged tests; full Linux/CI coverage remains pending.
- Earlier Linux VM verification: 208 helper tests passed in 20 consecutive parallel runs, Clippy and cargo-deny passed, and idle memory measured 14.37 MiB against a 40 MiB limit.

## Found, not fixed
- Historical database-create failure still needs QA 03 VM request/journal/kernel evidence; no runtime OOM, timeout or panic cause is claimed.
- Live VM checks for fixes 04–18 and pending Git/CI/release work remain owner follow-ups; see QA 03/04/09/11/13/17/18/22/24/25/26. Fixes 15–18 excluded commit/push by request.
- Browser visual review and desktop/mobile smoke tests remain pending; prior browser automation was unavailable or conflicted with an active session.

## Next
- **18b**: routes, privileged operations, database and UI maps; 18c follows with agent instructions, lookup and CI freshness checking.
- On `opsbox-vm`, run the generator's `--check` and focused Python tests from `/home/ubuntu/opsbox`; the freshness regression must reject stale maps without writing them.
- Complete pending Linux workspace/VM checks and verify release archives, checksums, signatures, SBOM and installer from a host with Cargo and writable Git metadata.
