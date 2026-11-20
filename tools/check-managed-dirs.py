#!/usr/bin/env python3
"""Exercise the installer's actual managed-directory block without root or systemd."""
import os
from pathlib import Path
import shlex
import stat
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[1]
source = (REPO / "install.sh").read_text()
block = source.split("# BEGIN managed-state-dirs\n", 1)[1].split("# END managed-state-dirs", 1)[0]

with tempfile.TemporaryDirectory() as scratch:
    root = Path(scratch)
    state = root / "state"
    state.mkdir(mode=0o1770)
    log = root / "commands"
    # Retain actual shell control flow, mkdir and chmod. Record the root/group
    # arguments instead of chowning; Rust fixtures cover descriptor ownership.
    prelude = f'''
set -eu
STATE_DIR={shlex.quote(str(state))}
SVC_USER=opsbox
DRY_RUN=0
log={shlex.quote(str(log))}
die() {{ echo "$*" >&2; exit 1; }}
run() {{
  [ "$#" -eq 9 ] && [ "$1" = install ] && [ "$2" = -d ]
  [ "$3" = -m ] && [ "$5" = -o ] && [ "$6" = root ] && [ "$7" = -g ]
  printf '%s %s %s\\n' "$4" "$6:$8" "${{9##*/}}" >>"$log"
  mkdir -p "$9"
  chmod "$4" "$9"
}}
'''
    def run():
        return subprocess.run(["sh"], input=prelude + block, text=True, capture_output=True)

    expected = {"apps": 0o750, "xfer": 0o770, "trash": 0o700}
    for migration in (False, True):
        if migration:
            for name in expected:
                (state / name / "sentinel").write_text("keep")
                os.chmod(state / name, 0o777)
        result = run()
        assert result.returncode == 0, result.stderr
        for name, mode in expected.items():
            assert stat.S_IMODE((state / name).stat().st_mode) == mode
            if migration:
                assert (state / name / "sentinel").read_text() == "keep"
        assert stat.S_IMODE(state.stat().st_mode) == 0o1770
    assert log.read_text().splitlines() == [
        "0750 root:opsbox apps", "0770 root:opsbox xfer", "0700 root:root trash",
    ] * 2
    for name in expected:
        path = state / name
        saved = root / f"saved-{name}"
        path.rename(saved)
        path.symlink_to(saved, target_is_directory=True)
        before = saved.stat()
        commands = log.read_text()
        result = run()
        assert result.returncode != 0 and "symlink" in result.stderr
        assert saved.stat() == before
        assert (saved / "sentinel").read_text() == "keep"
        assert log.read_text() == commands, "must refuse before installing any directory"
        assert stat.S_IMODE(state.stat().st_mode) == 0o1770
        path.unlink()
        saved.rename(path)
print("Managed directories: fresh install, legacy modes and all three symlink refusals passed.")
