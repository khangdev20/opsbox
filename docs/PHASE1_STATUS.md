# Phase 1 handoff — 17 November 2026

## Where the previous run stopped

The repository was already on `dev`, in sync with `origin/dev`, at `8f0741b` (M7c). Commits covered M1–M6, then Cleanup (M7a), Access (M7b), Security/invitations/allow-list (M7c). Settings/API-token work was uncommitted and failed Clippy. M8 was unfinished; VERIFY.md stopped at M6 and API/README/ROADMAP still described the skeleton.

Unrelated edits in `design/pages4.py`, `design/project/Landing.dc.html` and the untracked `site/` directory were preserved and excluded from the continuation's commits.

## Continued implementation

| Milestone | State after review |
|---|---|
| M1 Honest UI | Real/mock capability switch remains central in api.js; mixed Settings sections now have individual sample labels and disabled sample actions. |
| M2 Auth/setup/audit | Existing password/TOTP/setup/session/role/lockout implementation retained and tested. Mutations now require a persisted acceptance audit before dispatch; token actors are distinguished. |
| M3 Metrics | Existing sampler/ring/rollups/Overview retained. Attention now reuses apt/dnf package discovery rather than requiring Ubuntu's optional apt-check. |
| M4 System | Inherited features retained; systemd now uses zbus D-Bus, selected-package install has typed validation and UI checkboxes, automatic-update setup/job runs use background tasks. Emergency SSH success is audited as success. |
| M5 Files | Existing canonicalization/jails/atomic writes/staged transfers/trash retained; real Monaco save checked in Chromium. |
| M6 Terminal | Existing PTY/xterm retained. Live revocation and stream limits added; shared lazy AMD loading fixes Files → Terminal navigation. Root-helper login shell confirmed in logind session scope on Ubuntu. |
| M7 Cleanup/Access/Security/Settings | Settings and scoped tokens finished in `ca99d34`. Scope checks stop deploy tokens inheriting deployer file access. Real UI/token lifecycle verified. |
| M8 Hardening | IP/auth/session token buckets, body/deadline/stream limits, no-store/security headers, bounded child output/task history, process-group timeouts, doctor, generated API inventory and CI memory budget implemented. |

## Evidence

- 391 workspace tests passed on Ubuntu 24.04 arm64; fmt, Clippy `-D warnings`, shellcheck and generated API checks passed. Release builds/reinstalls succeeded; status reported both processes healthy; audit chains verified.
- Chromium used an isolated database for real setup/TOTP, Settings, token creation/use/scope/revocation, D-Bus service listing, selected-update UI, Cleanup/Access/Security, Monaco save and a PTY shell, including Files → Terminal. No page errors.
- Disposable root-helper service enable/start/restart/stop/disable and systemd timer/job execution passed; fixtures removed. PTY cgroup: `/user.slice/user-1000.slice/session-c1.scope`.
- Idle installed serve: **6.91 MiB RSS**. Isolated serve after setup/login/UI activity: **14.03 MiB RSS**. Automated fresh-process probe: **6.82 MiB** after 15 seconds. Target: under 40 MiB. These measurements do not establish a seven-day resource budget.

## Decisions and remaining validation

See [DECISIONS.md](DECISIONS.md) for zbus, file/PTY staging/relay substitutions, capability scopes and API documentation generation. Token scopes in Phase 1 are read/deploy/full; service-specific deployments, token expiry and full OpenAPI schemas are deferred. Tasks use polling and bounded in-memory history; incremental task/event WebSockets remain deferred. The installer shell script still provisions the machine; `opsbox install` remains its pre-existing planning stub.

This review did **not** perform real package upgrades, reboot, firewall activation/lockout/revert on the active connection, the one-hour SSH expiry, edits to the current SSH account, a seven-day soak or multi-distribution testing. Those require the explicit VM/UI checks in [VERIFY.md](VERIFY.md). dnf/firewalld have parser/policy coverage but need Fedora/Amazon Linux validation. Extended file operations outside the current UI (resumable chunks, chown, rename) are not added here.

No Phase 2–6 implementation was started. Their UI remains labelled sample data. The installed VM's real account database was preserved; enrolment was tested separately, so first-run setup is still required on the installed instance and doctor may warn that its master key has not yet been generated.

## Reproduce

[VERIFY.md](VERIFY.md) has exact build, reinstall and UI steps for M1–M8. Start with:

```sh
multipass shell opsbox-vm
cd /home/ubuntu/opsbox
export PATH="$HOME/.cargo/bin:$PATH" CARGO_TARGET_DIR="$HOME/target"
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
python3 tools/check-idle-memory.py --binary ~/target/release/opsbox
sudo sh install.sh --dev --binary ~/target/release/opsbox
sudo opsbox status
sudo opsbox doctor
sudo opsbox audit verify
```

Use the existing SSH tunnel to open `http://127.0.0.1:8443/?mode=http`; `sudo opsbox setup-token` prints a fresh claim token if setup is still required.
