# Fix 40. Add a server size guide (docs and site text only)

Follow `docs/prompts/fix/README.md`. Text only, no code. Do not publish numbers you did not check.

## Problem
Requirements currently say "1 GB RAM or more is comfortable", but a 0.9 GB server cannot build (builds use a 1536 MB BuildKit driver and need about 7 GB free disk including a 2 GB reserve). New users do not know what size of server to rent.

## Steps
1. Measure first, on the disposable VM: at 1 GB and at 2 GB RAM, record what fits with opsbox installed: idle memory used (OS, opsbox, Docker) from the Overview, and whether each case works: (a) 2 small prebuilt-image apps, (b) 2 to 3 apps plus 1 database with a build on the server, (c) the same with the build done in GitHub Actions. Put the measured numbers in `docs/QA.md` under "Owner checks" if you cannot run them, and mark the table below as "proposed" until they are filled in.
2. Add a "Server size guide" section to `docs/INSTALL.md`, one table, then two sentences. Proposed values (replace with the measured ones):

   | What you run | RAM | Disk |
   |---|---|---|
   | Try it: 1 to 2 small apps from prebuilt images | 1 GB | 20 GB |
   | 2 to 3 apps and 1 database, building on the server | 2 GB | 25 GB |
   | 4 to 8 containers, several databases, frequent builds | 4 GB | 50 GB |

   Sentences: a 1 GB server can run apps but not build them, so build in GitHub Actions and pull the image; these are guides, the panel's Capacity view gives the exact answer for your server.
3. Fix the Requirements line in `README.md`, `docs/INSTALL.md` and `site/index.html` (and `site/` Get started if it has its own): replace "1 GB RAM or more is comfortable" with "1 GB to run apps, 2 GB or more to build on the server" and link to the guide.
4. Use "disk" (not "ROM"), plain words, no promises such as "guaranteed".

## Done when
One table exists in `docs/INSTALL.md`, every Requirements mention matches it, and every number is either measured or marked "proposed".

Commit `fix 40: server size guide`, push `dev` (docs only, no tag), archive this file.
