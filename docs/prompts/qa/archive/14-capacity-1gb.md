# QA 14. Capacity guard on the 1 GB VM

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. On `opsbox-vm` (about 956 MB RAM) open System > Capacity: numbers add up (RAM, OS, opsbox, Docker, workloads, free).
2. Add PostgreSQL: it is blocked with a readable reason; the override requires the checkbox and the exact sentence; override creates it; the memory map updates.
3. Add a 2 GB swap file with the helper: allocatable grows, but the database reason still says swap does not count for databases.
4. Start three small containers until a new app is refused; the refusal names what to free.

## Pass when
Numbers match `free -m`; override and swap behave as written in the capacity prompts.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 14: Capacity guard on the 1 GB VM`, push `dev`, archive this file.
