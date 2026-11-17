#!/usr/bin/env python3
"""Measure a release serve process on Linux with isolated database/helper paths."""
import argparse
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import time
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--budget-mib', type=float, default=40)
    parser.add_argument('--settle-seconds', type=float, default=15)
    args = parser.parse_args()
    binary = str(args.binary.resolve())
    if not Path('/proc/self/status').exists():
        raise SystemExit('This RSS check requires Linux /proc.')
    with tempfile.TemporaryDirectory(prefix='ob-mem-', dir='/tmp') as tmp:
        root = Path(tmp)
        with socket.socket() as reserve:
            reserve.bind(('127.0.0.1', 0))
            port = reserve.getsockname()[1]
        config = root / 'config.toml'
        config.write_text(f'''[api]
listen = "127.0.0.1:{port}"
[priv]
socket = "{root}/priv.sock"
allowed_users = []
allowed_uids = [{os.getuid()}]
socket_group = ""
master_key = "{root}/master.key"
[store]
db_path = "{root}/opsbox.db"
[files]
allow_external_xfer_dir = true
xfer_dir = "{root}/xfer"
trash_dir = "{root}/trash"
apps_dir = "{root}/apps"
''')
        children = []
        try:
            with (root / 'process.log').open('w') as log:
                for command in ['priv', 'serve']:
                    children.append(subprocess.Popen([binary, '--config', str(config), command], stdin=subprocess.DEVNULL, stdout=log, stderr=log))
                deadline = time.monotonic() + 20
                while True:
                    if any(p.poll() is not None for p in children):
                        raise SystemExit('A probe process exited before becoming ready.')
                    try:
                        with urllib.request.urlopen(f'http://127.0.0.1:{port}/readyz', timeout=1) as response:
                            if response.status == 200:
                                break
                    except OSError:
                        pass
                    if time.monotonic() > deadline:
                        raise SystemExit('Probe did not become ready within 20 seconds.')
                    time.sleep(0.1)
                time.sleep(args.settle_seconds)
                status = Path(f'/proc/{children[1].pid}/status').read_text()
                rss_kib = int(next(line for line in status.splitlines() if line.startswith('VmRSS:')).split()[1])
                rss = rss_kib / 1024
                print(f'serve idle RSS: {rss:.2f} MiB; budget: {args.budget_mib:.2f} MiB; settle: {args.settle_seconds:g}s')
                if rss >= args.budget_mib:
                    raise SystemExit('Idle memory budget exceeded.')
        finally:
            for process in reversed(children):
                if process.poll() is None:
                    process.send_signal(signal.SIGTERM)
            for process in children:
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()


if __name__ == '__main__':
    main()
