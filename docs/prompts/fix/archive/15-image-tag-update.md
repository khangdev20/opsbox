# Fix 15. Cannot change an image service's tag

Follow `docs/prompts/fix/README.md` (time box 20 minutes, test first, smallest fix).

## Problem
QA 17: `PATCH /api/v1/services/{id}` with a new `image` is accepted but ignored (unknown fields are dropped), so a redeploy keeps the old tag.

## Steps
1. Test: create an image service, PATCH `image` to a new valid reference, fetch it: the stored `image_ref` is the new one and the next deploy pulls it; invalid references are refused with a field error.
2. Add `image` to the update body, validate it with the same parser the create path uses, and show it in the service settings with a Redeploy hint. Capacity middleware already inspects the image: keep it working.

## Done when
Test passes; rollback to the previous tag still works.

## Record
Mark the matching entry in `docs/QA.md` as `Fixed in fix 15`. Commit `fix 15: Cannot change an image service's tag`, push `dev`, archive this file.
