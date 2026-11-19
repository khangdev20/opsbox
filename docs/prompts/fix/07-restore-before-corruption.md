# Fix 07. Database restore can write before a late corruption is found

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 18: a database restore with 'replace' streams the dump into the live database; if the backup is corrupt later in the stream, the live data is already partly changed.

## Steps
1. Test with an injected dump that emits a valid first part and then fails: after the failed restore the live rows must equal their values before the request.
2. Fix, smallest first: restore into a temporary file or temporary database first (verify restic integrity and that the dump ends cleanly), and only then swap or replay into the live database inside one transaction where the engine allows it. If it needs more than the time box, implement the pre-check (full read and checksum of the snapshot before any write) and write the swap design under QA 18.

## Done when
The injected-failure test leaves live data untouched.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 07`. Commit `fix 07: Database restore can write before a late corruption is found`, push `dev`, archive this file.
