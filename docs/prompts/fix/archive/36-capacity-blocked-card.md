# Fix 36. Redesign the "Blocked" capacity message

Follow `docs/prompts/fix/README.md`. Panel only. Needs fix 35 (`checks`, `actions`, `breakdown`). Find where the text is shown: `rg -n 'allocatable|capacity' web/js`.

## Problem
The blocked state is one grey paragraph of numbers separated by dots. People cannot tell what is wrong or what to do.

## Design (replace the paragraph with a card)
1. Header: "Can't build here" with a Blocked badge, then one plain sentence: "This build needs 1.5 GB of memory and 7 GB of free disk. This server has 0.7 GB memory available and 3.4 GB free disk."
2. Two rows, one per check: label, a bar of have versus need, and "need X, have Y". A failing row also shows a small "short by Z" in the warning colour plus an icon (not colour alone). Disk shows its paths once, under the row, in muted text.
3. "Details" collapsed by default: the memory breakdown as a small table (RAM, OS, opsbox, Docker, workloads, swap credit).
4. "How to fix", most helpful first, each a button with its effect: Lower the memory limit to [suggested] MB (sets the field), Build in GitHub Actions and pull the image, Stop [service] (frees 256 MB; asks to confirm), Clean up disk (opens Cleanup), Upgrade the instance (opens docs link).
5. Plain words: no "allocatable", "credit" or "reserve" in the main text; use "available" and "kept free for safety" with a tooltip.

## Steps
1. Test first (Playwright, 1440 and 390): with a blocked fixture the card shows two rows, the disk paths appear once, the details are collapsed, each action is a button of at least 44 px height at 390, and nothing overflows.
2. Build the card from `checks`, `actions` and `breakdown`. Fall back to the old text if the fields are missing (old server).
3. Reuse existing bar, badge and button styles; no new colours.

## Done when
The blocked state reads in under five seconds, the repeated disk lines are gone, every action is clickable, and 390 px is clean.

Commit `fix 36: capacity blocked card`, push `dev`, archive this file.
