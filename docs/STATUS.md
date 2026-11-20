# Status

Last prompt done: **Fix 19 — Deployer must not reach root through Compose**.
No commit, push, tag or installation attempted, as requested; changes remain on `dev`.

## Completed
- Extended Compose risk detection to capabilities, devices, security options, user namespaces, sysctls, inherited volumes and host PID/IPC; external short/long bind mounts remain covered.
- Require Admin before capacity admission and deploy enqueue for risky Compose; YAML parse errors fail closed for Deployer.
- Ignore non-Admin confirmation and carry authenticated Admin authorization into background stack redeploy, including legacy/imported stacks. The helper retains raw/resolved checks.
- Imports already require Admin; fixed catalog template installs have no Compose input or arbitrary host-access options.
- Recorded the decision, QA evidence and owner checks; marked fix 19 done and archived its prompt.

## Verification
- Strict TDD: observed failing HTTP create, malformed YAML and legacy confirmed-stack redeploy regressions, plus missed cap_add in the detector.
- Three Compose role HTTP tests pass (every requested key, both confirmation values); five Compose detector tests pass.
- `cargo fmt --check`, targeted all-target Clippy with `-D warnings`, `git diff --check` and regenerated index freshness checks pass.
- Updated full lifecycle coverage for plain Deployer create/redeploy and confirmed elevated Admin deployment. Local TCP bind denied by sandbox before lifecycle assertions; run on the VM.

## Found, not fixed
- Live-helper lifecycle, VM installation and browser checks remain owner follow-ups (docs/QA.md, Fix 19).
- Historical database-create failure still requires QA 03 VM request/journal/kernel evidence; no OOM, timeout or panic cause is claimed.
- Prior Linux/VM and pending CI/release work remain owner follow-ups; prior runtime target remains `v0.1.0-alpha.43`.
- Navigation maps/lookup remain lexical rather than Rust/JavaScript type resolution.

## Next
- Fix 20 — Full API tokens reach root-equivalent endpoints.
