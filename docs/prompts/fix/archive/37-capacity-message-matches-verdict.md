# Fix 37. Capacity error contradicts its own numbers

Follow `docs/prompts/fix/README.md`. Rust first (`crates/opsbox-api/src/capacity.rs`; find with `rg -n 'allocatable|builds blocked' crates web/js`), then one panel label. Do after fix 35.

## Problem
An action was blocked with: "Needs 128 MB, 680 MB allocatable (956 RAM, 192 OS, 128 opsbox, 150 Docker, 704 workloads, 898 swap credit); minimum limit 128 MB". 680 is more than 128, so memory is not short by these numbers, yet the action fails and the advice is "lower the memory limit". The Overview Memory card also says "builds blocked", while the machine uses only 0.4 of 0.9 GB. Likely causes to confirm: the check compares the REQUESTED limit (not the minimum printed), or the failing check is disk (needs 7168 MB free including 2048 reserve, about 3.5 to 4 GB free) and the message shows the memory line anyway.

## Steps
1. Reproduce first with a test: create the same situation (allocatable 680, minimum 128, free disk 3484, requested limit default). Print which check fails and with which numbers.
2. Make the message show the number that actually failed: if memory, "You asked for X MB, only Y MB are unreserved" (X is the requested limit); if disk, the disk line only. A passing check must never appear as a reason.
3. Explain "workloads 704 MB" as reserved limits of existing services, not current use. List the biggest reservations by name (the message already names `greenglish-dev-worker-1`: show its reserved MB).
4. On Overview, the "builds blocked" link goes under the resource that blocks it (Disk card for disk) and states it: "builds blocked: not enough disk".

## Done when
No error lists a check that passed, the failing check shows the right numbers, and the Overview label names the real cause. Old text field stays for old clients.

Commit `fix 37: capacity message matches verdict`, push `dev`, archive this file.
