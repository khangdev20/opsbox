#!/usr/bin/env python3
"""Root-only Docker takeover/Undo check on an isolated installed test VM.

Requires the test container `web` already publishing 80/443. Uses typed priv frames;
HTTP confirmation, RBAC, CSRF and audit are covered by network_http.rs.
"""
import json
from pathlib import Path
import socket
import struct
import subprocess
import time


def call(kind, args=None):
    op = {'type': kind}
    if args is not None:
        op['args'] = args
    request = json.dumps({'id': '03c-vm-check', 'actor': '03c-vm-check', 'op': op}).encode()
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(90)
        connection.connect('/run/opsbox/priv.sock')
        connection.sendall(struct.pack('>I', len(request)) + request)

        def read(size):
            data = b''
            while len(data) < size:
                part = connection.recv(size - len(data))
                if not part:
                    raise RuntimeError('priv disconnected')
                data += part
            return data

        size, = struct.unpack('>I', read(4))
        assert size <= 1024 * 1024
        return json.loads(read(size))['result']


def network():
    result = call('proxy_network')
    assert result['status'] == 'ok', result
    return result['reply']['data']['network']


def inspect():
    return json.loads(subprocess.check_output(['docker', 'inspect', 'web']))[0]


def wait_for(conflict):
    deadline = time.monotonic() + 35
    while True:
        result = call('proxy_network')
        if result['status'] == 'ok':
            net = result['reply']['data']['network']
            if net['port_conflict'] == conflict:
                return net
        assert time.monotonic() < deadline, result
        time.sleep(1)


def check_keep_ports():
    config = Path('/etc/opsbox/config.toml')
    original = config.read_bytes()
    try:
        assert network()['port_conflict']
        result = call('proxy_keep_ports', {'confirm': True})
        assert result['status'] == 'ok', result
        net = network()
        assert not net['port_conflict']
        assert net['primary_http'] == net['listen_http']
        assert net['primary_https'] == net['listen_https']
        assert 'listen_http = "0.0.0.0:8080"' in config.read_text()
        assert 'listen_https = "0.0.0.0:8445"' in config.read_text()
        print('Keep ports saved 8080/8445 and ended primary retries live.')
    finally:
        config.write_bytes(original)
        subprocess.run(['systemctl', 'restart', 'opsbox-proxy'], check=True)
    assert wait_for(True)['listen_http'].endswith(':8080')
    subprocess.run(['systemctl', 'restart', 'opsbox-watchdog'], check=True)
    print('Test configuration restored; web and the fallback proxy remain running.')


def main():
    old = inspect()
    assert old['State']['Running']
    net = network()
    assert net['port_conflict'] and set(net['conflict_ports']) == {80, 443}, net
    assert any(h.get('container', {}).get('name') == 'web' for h in net['holders']), net
    print('Holder identified: web; proxy healthy on fallback ports.')
    pid = subprocess.check_output(['systemctl', 'show', '--property=MainPID', '--value', 'opsbox-proxy']).strip()
    request = {'port': 80, 'target': {'kind': 'container', 'id': old['Id']}, 'confirm': False}
    refused = call('proxy_takeover', request)
    assert refused['status'] == 'error' and refused['error']['code'] == 'confirm_required', refused
    assert inspect()['State']['Running']
    print('Unconfirmed takeover refused safely; container still running.')
    request['confirm'] = True
    taken = call('proxy_takeover', request)
    assert taken['status'] == 'ok', taken
    receipt = taken['reply']['data']
    try:
        current = inspect()
        assert not current['State']['Running'] and current['HostConfig']['RestartPolicy']['Name'] == 'no'
        net = wait_for(False)
        assert net['listen_http'].endswith(':80') and net['listen_https'].endswith(':443'), net
        assert subprocess.check_output(['systemctl', 'show', '--property=MainPID', '--value', 'opsbox-proxy']).strip() == pid
        print('Takeover promoted to 80/443 within one retry; proxy PID unchanged; restart=no.')
    finally:
        undone = call('proxy_undo', {'id': receipt['id'], 'confirm': True})
        assert undone['status'] == 'ok', undone
    current = inspect()
    assert current['State']['Running'] and current['HostConfig']['RestartPolicy'] == old['HostConfig']['RestartPolicy']
    assert wait_for(True)['listen_http'].endswith(':8080')
    health = json.loads(subprocess.check_output(['curl', '-skf', 'https://127.0.0.1:8444/healthz']))
    assert not health['failed'], health
    print('Undo restored running container and original restart policy; fallback resumed; no safe mode.')
    check_keep_ports()


if __name__ == '__main__':
    main()
