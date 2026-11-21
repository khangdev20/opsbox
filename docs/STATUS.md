# Status

Last prompt completed: **Fix 39 — Discord alert channel**.
Released: `v0.1.0-alpha.49` (fixes 33–34). Next release: `v0.1.0-alpha.50` after fix 38; fix 38 part 1 is done; parts 2–3 remain in `docs/QA.md`.

## Completed
- Fix 38 part 1: additive `ports_busy` proxy status with holder name/PID, stable fallback serving and injectable 30-second retries without catch-up bursts; unreadable helper port tables preserve cached holder details.
- Full-width pages (`main`) use `width: min(100%, 1680px)` with `clamp(16px, 2.5vw, 40px)` side padding instead of the 1180 px cap.
- Settings and other `.narrow` pages keep an 880 px left-aligned inner block; the Security checks column is capped at 880 px; dialogs keep their compact width.
- Cleanup no longer uses `main.narrow`: the page matches Overview's width and the "Weekly auto-clean" switch aligns with the list edge; 390/768/1024 are unchanged.
- New service uses the wide container for all five tabs; source forms sit in a two-column grid with readable 560 px fields; repository rows are a grid with a straight right-aligned action, 12 px padding, name ellipsis/title and "Choose build" when nothing is detected.
- Settings uses the wide container with two-column section cards above 1100 px, 560 px field caps inside cards, right-aligned header actions, and one column at or below 1100 px.
- Capacity responses now carry grouped disk `checks` (one per device), a memory check with a six-row `breakdown`, `actions` with stable ids, and the legacy `text` built from the grouped reasons so a shared disk appears once.
- The blocked capacity state is a card: plain need/have sentence, two check rows with bars and "short by", disk paths once, collapsed details table, and five effect-labelled fix actions (with legacy text fallback).
- Capacity reasons now match the failing check: memory failures say "You asked for X MB, only Y MB are unreserved" and name the biggest reserved services; disk failures show only the disk line; passing checks never appear. Overview puts "builds blocked: not enough disk" under the Disk card.
- Discord is a sealed alert channel: HTTPS `discord.com`/`discordapp.com` webhooks only, severity-coloured embeds with length limits, one `retry_after` retry, masked last-four display, Connect/Send test in the panel, and no reveal path.
- Extended `tools/check-responsive-ui.mjs` with 1440/1920/2560 width assertions, Cleanup geometry, all New service tabs, Settings columns/alignment and the existing no-scroll checks.

## Verification
- Fix 38: socket-free fake startup/retry test and 3 helper network tests pass; fmt/clippy pass and maps regenerated. Full proxy tests hit sandbox socket-bind denial (9 pass, 4 existing library tests fail); rerun outside the sandbox.
- `cargo test -p opsbox-api --lib capacity` and `--test capacity_http` pass: one disk check for four paths on one device, two for two devices, single disk sentence, grouped `checks`/`actions`/`breakdown`.
- `node tools/check-capacity-ui.mjs` and `node tools/check-responsive-ui.mjs` pass, including the blocked card rows, single disk-path line, collapsed details, five 44 px actions at 390, the legacy fallback and the Overview disk label.
- `cargo test -p opsbox-api --lib capacity` covers the contradictory case: allocatable 680, requested 768, disk free 3484; disk failure shows only the disk line and memory failure names the requested limit and top reservations.
- `cargo test -p opsbox-alerts`, `cargo test -p opsbox-api --test alerts_http` and `node tools/check-alerts-ui.mjs` pass, including Discord embed limits/colours, one 429 retry, URL refusal, sealing and last-four display; clippy is clean.
- `timeout 120 node tools/check-responsive-ui.mjs` passes the source and Chrome browser checks, including every New service tab, Settings two-column alignment and 1440/390 row geometry.
- `timeout 120 python3 scripts/gen-index.py --check` passes; no generated map is stale.
- CI runs the workspace tests, release build, musl targets, cargo-deny and installer checks on the pushed commit.
- Release workflow `v0.1.0-alpha.49` published both signed musl archives, `install.sh` and the SBOM.

## Limits and next prompt
- Prior fixes' Linux owner checks in `docs/QA.md` still need the `opsbox-vm`; none were run here.
- Fix prompts 01–37 and 39 are complete; 38 part 1 (proxy busy state) is done; parts 2–3 (`proxy.mode`/installer/status, panel switch) remain. Prompt stays unarchived; no commit or push requested.

## Found, not fixed
- The Security width rule uses `main:has(> .hdr a[href="#/security/access"])`, coupled to the header link; a header or page change can silently drop the 880 px cap.
- Existing recursive delete/Trash policy checks only selected roots, not protected descendants; deleting an ancestor such as `/etc/systemd/system` can bypass descendant protection. Do not try it on an installed host.
- This environment strips setuid bits from temporary files, preventing the existing mode-display test from passing; rerun the complete file tests on Linux.
- Prior Fix 19/20 helper lifecycle, Fix 21 Safe Mode filtering/TLS isolation, and Fix 22 Linux activation/credential-consumer checks remain owner follow-ups.
- Historical database-create failure still needs QA 03 VM request/journal/kernel evidence; no OOM, timeout or panic cause is claimed.
- Navigation maps/lookup remain lexical rather than Rust/JavaScript type resolution.

## Next
- Run the pending Linux owner checks in `docs/QA.md` before the public alpha post; no new numbered fix prompt is queued.
