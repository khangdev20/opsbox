# 03f. Forms keep what the user typed after an error; helper errors say what really happened

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `web/js/pages/templates.js` (`paint`, `form`, the `add` action), `web/js/capacity.js` (`capacityFields`, `bindCapacity`), `web/js/pages/configure.js`, `web/js/ui.js` (`patch`, `formDialog`), `crates/opsbox-api/src/error.rs` (`From<ClientError>`), `crates/opsbox-api/src/health.rs`, `crates/opsbox-priv/src/client.rs`. Spec: sections 2 and 7.2 for the priv split.

Problem seen on a real 1 GB server (`v0.1.0-alpha.20`): adding PostgreSQL with "Override capacity" ticked and the confirmation typed shows "The privileged helper is not running. Start opsbox-priv and try again.", and afterwards the override checkbox and the typed sentence are empty again, so it looks as if nothing was ticked.

## 1. Do not wipe the form (UI)
Cause: in `templates.js` the `add` action calls `paint()` when it starts (`busy = 'tpl'`) and again on error, and `paint()` rewrites `#fm` with `innerHTML = form().toString()`. That destroys the checkbox, the typed confirmation, the name, version, disk, backup and network choices.
- Stop replacing the whole form. Update only what changes while busy: the submit button label and disabled state, the error line, and the capacity result text. Use the keyed `patch()` from `ui.js` (the same morphing the Overview uses) or targeted DOM updates. Form controls keep their live values.
- Re-render the form from scratch only when the user picks a different template (`pick`), and then say nothing about "reset"; that is expected.
- Do the same check for every place that uses `capacityFields`: New app (`configure.js`), memory dialog (`editMemory`), templates and the Compose deploy button. After a failed submit the user must find every field exactly as typed, the override checkbox and the confirmation sentence included, with the error shown in `role="alert"` and focus moved to it.
- `bindCapacity` already guards against stale results with `generation`; keep that, and make sure `refresh()` after an error does not clear the override fields.

## 2. Say what really happened when `priv` fails
Today `ClientError::Connect` and `ClientError::Timeout` both become "The privileged helper is not running", which is wrong for a slow call (pulling an image, starting a container) and gives no next step.
- Split the messages in `From<ClientError> for HttpError`:
  - connect refused or socket missing: "OpsBox's helper is not running. Restart it with `sudo systemctl restart opsbox-priv`."
  - timeout: "The helper did not answer in time. The operation may still be running; check Containers or System > Jobs before trying again." (Do not tell people to start it.)
  - forbidden: keep, but add which service user was refused in the log, not in the message.
- Log the real error with the request id, the op name and the elapsed time (`privileged helper call failed op=... elapsed_ms=... error=...`).
- Operations that legitimately take long (template start, compose deploy, image pull, database create) already use longer timeouts through `with_timeout`; list in `docs/STATUS.md` any priv call reachable from a POST in the capacity middleware (`crates/opsbox-api/src/capacity.rs`) that still uses the 10 s default, and give those a fitting timeout. Do not raise the default.
- Version skew: `priv` and `serve` must be the same build. Include the helper's version in the `Hello`/handshake if it is not there, compare it in `/health/ready` and in `opsbox status`, and when they differ return `privileged_helper: "version mismatch"` with the two versions. After `opsbox update` and `opsbox rollback`, the updater must restart `opsbox-priv` together with `serve` (or fail the health check so rollback happens).
- The panel shows a persistent banner (not only a toast) when `/health/ready` says the helper is down or mismatched: "OpsBox's helper is not running. Actions that change the server are paused." with the restart command and a Copy button. Read-only pages keep working.

## 3. Crash and memory
- If `opsbox-priv` was killed, the cause is often out-of-memory on a small server. In the `opsbox-priv` systemd unit set `Restart=always`, `RestartSec=2`, and `OOMScoreAdjust=-900` so the kernel prefers other processes. Verify the unit in `install.sh` and in the update path.
- Add one capacity reserve check in `opsbox-capacity`: opsbox's own reservation (128 MB today) must cover both `serve` and `priv`; record the measured RSS of each in `docs/DECISIONS.md` if the number needs changing.

## Tests and checks
- Browser test (mock API that rejects the first submit): fill the template form, tick override, type the sentence, submit; after the error every field has the same value and the error line has focus. Same test for New app and the memory dialog.
- Rust tests: `ClientError::Connect`, `Timeout` and `Remote(Forbidden)` each map to their own message; a version mismatch makes `/health/ready` return 503 with both versions.
- `cargo fmt --check`, clippy and tests pass.

## Verify on `opsbox-vm` (1 GB)
1. Add PostgreSQL with Override ticked; whatever the result, the form keeps its values. Success shows the connection dialog.
2. `sudo systemctl stop opsbox-priv`, press Add: the message says the helper is not running and gives the restart command; the banner appears; Overview and Logs still load. Start it again and the banner clears without a reload.
3. Must fail safely: with the helper stopped, "Add" never creates a half-made database or container; after the restart the list shows no leftover entry.
