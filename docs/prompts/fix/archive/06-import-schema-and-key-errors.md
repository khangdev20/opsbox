# Fix 06. Import accepts invalid documents and can hide key failures

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 24: import preview accepts a document that breaks the checked-in schema in `docs/schemas/` (`composeProjects` as a number, missing `host`); and when sealing keys fail (helper rejects `SecretSeal`, or a bad destination key), the task can finish as success with secrets missing.

## Steps
1. Tests: (a) the schema violations above are rejected with a readable error naming the field; (b) with a helper that rejects `SecretSeal`, import ends as failed or partial and lists which secrets were not restored.
2. Validate against the same schema the exporter uses (a minimal checker is enough, no new dependency), and make the task report per-item results; never mark success when any secret was skipped.

## Done when
Both tests pass and the panel shows 'Restored 3 of 4 secrets, 1 failed' style messages.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 06`. Commit `fix 06: Import accepts invalid documents and can hide key failures`, push `dev`, archive this file.
