# Status

Last prompt completed: **Fix 32 — Cleanup page width**.
Released: `v0.1.0-alpha.47` (fix 31). Next release: `v0.1.0-alpha.48` once CI passes.

## Completed
- Full-width pages (`main`) use `width: min(100%, 1680px)` with `clamp(16px, 2.5vw, 40px)` side padding instead of the 1180 px cap.
- Settings and other `.narrow` pages keep an 880 px left-aligned inner block; the Security checks column is capped at 880 px; dialogs keep their compact width.
- Cleanup no longer uses `main.narrow`: the page matches Overview's width and the "Weekly auto-clean" switch aligns with the list edge; 390/768/1024 are unchanged.
- Extended `tools/check-responsive-ui.mjs` with 1440/1920/2560 width assertions, Cleanup geometry, and the existing no-scroll checks.

## Verification
- `timeout 120 node tools/check-responsive-ui.mjs` passes the source and Chrome browser checks.
- `timeout 120 python3 scripts/gen-index.py --check` passes; no generated map is stale.
- CI runs the workspace tests, release build, musl targets, cargo-deny and installer checks on the pushed commit.

## Limits and next prompt
- Prior fixes' Linux owner checks in `docs/QA.md` still need the `opsbox-vm`; none were run here.
- Fix prompts 01–32 are complete in the working tree.

## Found, not fixed
- The Security width rule uses `main:has(> .hdr a[href="#/security/access"])`, coupled to the header link; a header or page change can silently drop the 880 px cap.
- Existing recursive delete/Trash policy checks only selected roots, not protected descendants; deleting an ancestor such as `/etc/systemd/system` can bypass descendant protection. Do not try it on an installed host.
- This environment strips setuid bits from temporary files, preventing the existing mode-display test from passing; rerun the complete file tests on Linux.
- Prior Fix 19/20 helper lifecycle, Fix 21 Safe Mode filtering/TLS isolation, and Fix 22 Linux activation/credential-consumer checks remain owner follow-ups.
- Historical database-create failure still needs QA 03 VM request/journal/kernel evidence; no OOM, timeout or panic cause is claimed.
- Navigation maps/lookup remain lexical rather than Rust/JavaScript type resolution.

## Next
- Run the pending Linux owner checks in `docs/QA.md` before the public alpha post; no new numbered fix prompt is queued.
