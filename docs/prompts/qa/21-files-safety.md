# QA 21. Files: traversal, symlinks and big files

Follow `docs/prompts/qa/README.md` (time box 20 minutes, check first, fix only if tiny, otherwise write it up).

## Check
1. Try `../../etc/passwd`, an absolute path, URL-encoded dots, a symlink inside an app dir pointing to `/etc`, and a path with a null byte.
2. Upload a 200 MB file and a file with a weird name (spaces, unicode, leading dash); download and compare checksums.
3. Edit a file while another process changes it: no silent overwrite (race handling from prompt 01).
4. Each unsafe case is refused with a sentence and an audit entry.

## Pass when
Nothing outside the allowed roots is readable or writable.

## Record
One row in `docs/QA.md`; `Found, not fixed` and `Owner checks` sections for anything you could not do or fix in time. Commit `qa 21: Files: traversal, symlinks and big files`, push `dev`, archive this file.
