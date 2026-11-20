# Fix 18. Why did creating a database report that the helper is not running?

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
On a 1 GB server a PostgreSQL create with 'Override capacity' returned 'The privileged helper is not running' (older message). Messages and the form were fixed in 03f, but the root cause was never identified. This prompt finds it or proves it is gone.

## Steps
1. On `opsbox-vm` (1 GB, no swap, Docker installed) with the current alpha: create PostgreSQL at 256 MB with the override typed. Run `journalctl -u opsbox -u opsbox-priv --since '-10 min'` and `dmesg | grep -i -E 'oom|killed process'` before and after.
2. Classify: priv killed by the OOM killer, a timeout in a call that should use a longer `with_timeout`, a handshake or version mismatch, a panic. Record the finding in `docs/QA.md` with the log lines.
3. If it is a timeout or OOM, apply the smallest fix: the right timeout for that op, or `OOMScoreAdjust=-900` plus a reserve check; add a test for the timeout mapping. If it cannot be reproduced after two attempts, say so and stop.

## Done when
A written root cause with log evidence, or 'not reproducible on alpha.N' with the exact steps tried; plus the small fix when one is justified.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 18`. Commit `fix 18: Why did creating a database report that the helper is not running?`, push `dev`, archive this file.
