#!/usr/bin/env python3
"""Linux smoke test: fill a disposable 64 MiB disk, never the VM's root disk.

Requires sudo for a temporary loop mount. Uses isolated serve/priv paths and removes
all processes, mounts and files on exit. Does not change the installed services.
"""
import argparse
import json
import os
from pathlib import Path
import socket
import ssl
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    binary = str(args.binary.resolve())
    with tempfile.TemporaryDirectory(prefix='ob-alert-', dir='/tmp') as tmp:
        root = Path(tmp)
        image, mount, db = root / 'disk.img', root / 'disk', root / 'opsbox.db'
        mount.mkdir()
        with image.open('wb') as f:
            f.truncate(64 * 1024 * 1024)
        subprocess.run(['mkfs.ext4', '-q', '-F', '-m', '0', str(image)], check=True)
        children = []
        mounted = False
        try:
            subprocess.run(['sudo', 'mount', '-o', 'loop', str(image), str(mount)], check=True)
            mounted = True
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
db_path = "{db}"
[files]
allow_external_xfer_dir = true
xfer_dir = "{root}/xfer"
trash_dir = "{root}/trash"
apps_dir = "{root}/apps"
''')
            with (root / 'process.log').open('w') as log:
                for command in ['priv', 'serve']:
                    children.append(subprocess.Popen([binary, '--config', str(config), command], stdin=subprocess.DEVNULL, stdout=log, stderr=log))
                def wait_state(expected, seconds=110):
                    deadline = time.monotonic() + seconds
                    while time.monotonic() < deadline:
                        if any(p.poll() is not None for p in children):
                            raise RuntimeError('Probe process exited: ' + (root / 'process.log').read_text())
                        try:
                            with sqlite3.connect(db) as conn:
                                row = conn.execute("SELECT value FROM settings WHERE key='alerts.engine.v1'").fetchone()
                            engine = json.loads(row[0]) if row else {}
                            incident = engine.get('incidents', {}).get(f'disk:{mount}')
                            if incident and incident['state'] == expected:
                                print(f'Disposable disk: {expected}; {incident["msg"]}', flush=True)
                                return
                        except (sqlite3.Error, FileNotFoundError):
                            pass
                        time.sleep(2)
                    raise RuntimeError(f'Disk alert never reached {expected}')
                # sudo writes only to the isolated loop filesystem, bounded to 50 MiB.
                subprocess.run(['sudo', 'dd', 'if=/dev/zero', f'of={mount}/fill', 'bs=1M', 'count=50', 'status=none'], check=True)
                wait_state('firing')
                subprocess.run(['sudo', 'rm', str(mount / 'fill')], check=True)
                wait_state('resolved', 35)
                try:
                    urllib.request.urlopen(f'https://127.0.0.1:{port}/api/v1/alerts', timeout=5, context=ssl.create_default_context(cadata=ssl.get_server_certificate(('127.0.0.1', port))))
                    raise RuntimeError('Anonymous alerts access was allowed')
                except urllib.error.HTTPError as error:
                    assert error.code in (401, 403), error.code
                    print(f'Anonymous alerts access refused: HTTP {error.code}', flush=True)
        finally:
            for process in reversed(children):
                process.terminate()
            for process in children:
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            if mounted:
                subprocess.run(['sudo', 'umount', str(mount)], check=True)


if __name__ == '__main__':
    main()
