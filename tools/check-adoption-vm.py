#!/usr/bin/env python3
"""Root-only adoption verification on the disposable opsbox-vm (installed services).

Uses the 03c `web` nginx fixture. Adds a temporary old.test vhost, tests the real
priv/proxy sockets, concurrent curls, app-route precedence, import, Undo and nginx
systemd adoption. Restores fixture files/routes/services even when an assertion fails.
HTTP RBAC/CSRF and task delivery are tested separately in network_http.rs.
"""
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import threading
import time


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def call(kind, args=None, proxy=False):
    op = {'type': kind}
    if args is not None:
        op['args'] = args
    request = json.dumps({'id': '16c-vm', **({} if proxy else {'actor': '16c-vm'}), 'op': op}).encode()
    with socket.socket(socket.AF_UNIX) as s:
        s.settimeout(600)
        s.connect('/run/opsbox-proxy/proxy.sock' if proxy else '/run/opsbox/priv.sock')
        s.sendall(struct.pack('>I', len(request)) + request)
        def read(n):
            b = b''
            while len(b) < n:
                part = s.recv(n - len(b))
                if not part:
                    raise RuntimeError('helper disconnected')
                b += part
            return b
        n, = struct.unpack('>I', read(4))
        assert n <= 1024 * 1024
        result = json.loads(read(n))['result']
        return result


def ok(kind, args=None, proxy=False):
    r = call(kind, args, proxy)
    assert r['status'] == 'ok', r
    return r['reply'].get('data')


def curl(domain='old.test', tls=True):
    port = 443 if tls else 80
    p = subprocess.run(['curl', '-ksS', '--noproxy', '*', '--max-time', '1',
                        '--resolve', f'{domain}:{port}:127.0.0.1',
                        f'{"https" if tls else "http"}://{domain}/'], capture_output=True, text=True)
    return p.returncode == 0 and ('old-site-16c' if domain == 'old.test' else 'new-app-16c') in p.stdout


def measure(target, confirmation):
    plan = ok('web_discover', {'target': target})
    assert plan['supported'], plan
    assert 'old.test' in [d['domain'] for d in plan['routes']['domains']], plan
    refused = call('web_move', {'target': target, 'domains': ['old.test'], 'everything_else': False, 'confirmation': 'wrong'})
    assert refused['status'] == 'error' and refused['error']['code'] == 'confirm_required', refused
    assert curl(), 'refused move changed the old site'
    stop = threading.Event()
    requests = []
    def loop():
        while not stop.is_set():
            requests.append(curl())
            time.sleep(.2)
    thread = threading.Thread(target=loop)
    thread.start()
    receipt = None
    try:
        receipt = ok('web_move', {'target': target, 'domains': ['old.test'], 'everything_else': False, 'confirmation': confirmation})
        assert curl() and curl(tls=False)
        assert not ok('proxy_network')['network']['port_conflict']
        print(json.dumps({'holder': plan['holder'], 'move': receipt['id'], 'steps': receipt['steps']}), flush=True)
        return receipt
    finally:
        stop.set()
        thread.join()
        print(f'Cutover {plan["holder"]}: {len(requests)} requests, {requests.count(False)} failures', flush=True)
        if requests.count(False) > 2:
            if receipt:
                undo(receipt)
            raise AssertionError(f'Too many cutover failures: {requests.count(False)}')


def undo(receipt):
    ok('web_undo', {'id': receipt['id'], 'confirm': True})
    assert curl() and curl(tls=False)
    assert ok('proxy_network')['network']['port_conflict']
    print('Undo verified original HTTP and HTTPS answers.', flush=True)


def main():
    assert os.geteuid() == 0 and run('hostname') == 'opsbox-vm'
    work = Path('/var/tmp/opsbox-16c')
    work.mkdir(exist_ok=True)
    hosts = Path('/etc/hosts').read_bytes()
    certroot = Path('/etc/letsencrypt/live/old.test')
    assert not certroot.exists(), 'old.test certificate fixture already exists'
    certroot.mkdir(parents=True)
    run('openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '2',
        '-subj', '/CN=old.test', '-addext', 'subjectAltName=DNS:old.test,DNS:new.test',
        '-keyout', str(certroot / 'privkey.pem'), '-out', str(certroot / 'fullchain.pem'))
    os.chmod(certroot / 'privkey.pem', 0o600)
    cert = certroot / 'fullchain.pem'
    key = certroot / 'privkey.pem'
    vhost = f'''server {{
    listen 80;
    listen 443 ssl;
    server_name old.test;
    ssl_certificate {cert};
    ssl_certificate_key {key};
    location / {{ return 200 "old-site-16c\\n"; }}
}}
'''
    fixture = work / 'opsbox16c.conf'
    fixture.write_text(vhost)
    Path('/etc/hosts').write_bytes(hosts + b'\n127.0.0.1 old.test new.test\n')
    original = json.loads(run('docker', 'inspect', 'web'))[0]
    assert original['Config']['Image'].startswith('nginx')
    routes_path = Path('/var/lib/opsbox/proxy/routes.json')
    routes_before = json.loads(routes_path.read_text()) if routes_path.exists() else []
    receipt = None
    compose_dir = work / 'compose'
    compose_dir.mkdir(exist_ok=True)
    compose_file = compose_dir / 'compose.yml'
    compose_started = False
    nginx_fixture = Path('/etc/nginx/conf.d/opsbox16c.conf')
    nginx_was_installed = Path('/usr/sbin/nginx').exists()
    nginx_was_active = subprocess.run(['systemctl', 'is-active', '--quiet', 'nginx']).returncode == 0
    assert not nginx_was_active, 'unit fixture requires nginx initially inactive'
    try:
        # Copying into the existing fixture tests that recreation preserves its writable layer.
        run('docker', 'exec', 'web', 'mkdir', '-p', str(certroot))
        run('docker', 'cp', str(cert), f'web:{cert}')
        run('docker', 'cp', str(key), f'web:{key}')
        run('docker', 'cp', str(fixture), 'web:/etc/nginx/conf.d/opsbox16c.conf')
        run('docker', 'exec', 'web', 'nginx', '-s', 'reload')
        time.sleep(.3)
        assert curl() and curl(tls=False)
        receipt = measure({'kind': 'container', 'id': original['Id']}, 'recreate web')
        assert json.loads(run('docker', 'inspect', 'web-pre-opsbox'))[0]['State']['Running'] is False
        # New managed application route on the same :443 while old.test remains raw TLS.
        appconf = work / 'app.conf'
        appconf.write_text('server { listen 80; location / { return 200 "new-app-16c\\n"; } }\n')
        run('docker', 'run', '-d', '--name', 'opsbox16c-app', '--label', 'opsbox.managed=true',
            '-p', '127.0.0.1:19090:80', '-v', f'{appconf}:/etc/nginx/conf.d/default.conf:ro', original['Config']['Image'])
        sealed = ok('secret_seal', {'context': 'proxy:cert:new.test', 'plaintext': key.read_text()})['sealed']
        ok('import_cert', {'domain': 'new.test', 'cert_pem': cert.read_text(), 'key_sealed': sealed}, proxy=True)
        ok('set_routes', {'routes': routes_before + [{'host': 'new.test', 'upstream': '127.0.0.1:19090', 'upstream_tls': {'mode': 'plain'}, 'redirect_https': False, 'hsts': False}]}, proxy=True)
        adopted = ok('status', proxy=True)['adopted']
        adopted['domains'].append({**adopted['domains'][0], 'domain': 'new.test'})
        adopted['everything_else'] = True
        ok('set_adopted', {'routes': adopted}, proxy=True)
        assert 'old-site-16c' in run('curl', '-ksS', '--noproxy', '*', 'https://127.0.0.1/')
        assert 'old-site-16c' in run('curl', '-ksS', '--noproxy', '*', '--resolve', 'unknown.test:443:127.0.0.1', 'https://unknown.test/')
        assert curl('new.test') and curl()
        print('new.test app route and old.test passthrough share :443.', flush=True)
        ok('web_tls', {'domain': 'old.test'})
        assert curl()
        assert next(d for d in ok('status', proxy=True)['adopted']['domains'] if d['domain'] == 'old.test')['terminate_tls']
        print('Imported valid disk certificate before switching old.test to opsbox TLS.', flush=True)
        undo(receipt)
        receipt = None
        # A Compose override must replace public ports, preserve the source file,
        # and remove only the generated override on Undo.
        run('systemctl', 'stop', 'opsbox-proxy')
        run('docker', 'stop', 'web')
        compose_file.write_text(json.dumps({'services': {'site': {'image': original['Config']['Image'], 'ports': ['80:80', '443:443'], 'volumes': [f'{fixture}:/etc/nginx/conf.d/default.conf:ro', f'{certroot}:{certroot}:ro']}}}))
        compose_before = compose_file.read_bytes()
        compose = ['docker', 'compose', '-p', 'opsbox16c', '-f', str(compose_file)]
        run(*compose, 'up', '-d')
        compose_started = True
        run('systemctl', 'start', 'opsbox-proxy')
        time.sleep(.5)
        compose_id = run(*compose, 'ps', '-q', 'site')
        receipt = measure({'kind': 'container', 'id': compose_id}, 'move')
        assert compose_file.read_bytes() == compose_before
        assert (compose_dir / 'docker-compose.opsbox.override.yml').is_file()
        undo(receipt)
        receipt = None
        assert compose_file.read_bytes() == compose_before
        assert not (compose_dir / 'docker-compose.opsbox.override.yml').exists()
        print('Compose override, preserved original, cutover and Undo passed.', flush=True)
        run('systemctl', 'stop', 'opsbox-proxy')
        run(*compose, 'down')
        compose_started = False
        # Repeat through an actual nginx.service, including IPv6 wildcard listeners.
        if not nginx_was_installed:
            subprocess.run(['apt-get', 'update', '-qq'], check=True)
            subprocess.run(['apt-get', 'install', '-y', '-qq', 'nginx'], check=True)
        assert not nginx_fixture.exists()
        nginx_fixture.write_text(vhost.replace('    listen 80;', '    listen 80;\n    listen [::]:80;').replace('    listen 443 ssl;', '    listen 443 ssl;\n    listen [::]:443 ssl;'))
        run('nginx', '-t')
        # Package post-install restarts can revive an always-restart Docker fixture.
        run('docker', 'stop', 'web')
        run('systemctl', 'stop', 'opsbox-proxy')
        run('systemctl', 'restart', 'nginx')
        run('systemctl', 'start', 'opsbox-proxy')
        time.sleep(.5)
        receipt = measure({'kind': 'unit', 'id': 'nginx.service'}, 'move')
        assert curl()
        undo(receipt)
        receipt = None
        print('nginx.service cutover and Undo passed.', flush=True)
    finally:
        if receipt:
            undo(receipt)
        subprocess.run(['systemctl', 'stop', 'opsbox-proxy'])
        if compose_started:
            subprocess.run(['docker', 'compose', '-p', 'opsbox16c', '-f', str(compose_file), 'down'])
        subprocess.run(['systemctl', 'stop', 'nginx'])
        if not nginx_was_installed:
            subprocess.run(['systemctl', 'disable', 'nginx'])
        nginx_fixture.unlink(missing_ok=True)
        # Restore original nginx Listen files changed by unit adoption (Undo does this).
        subprocess.run(['docker', 'start', original['Id']])
        subprocess.run(['docker', 'exec', 'web', 'rm', '-f', '/etc/nginx/conf.d/opsbox16c.conf'])
        subprocess.run(['docker', 'exec', 'web', 'nginx', '-s', 'reload'])
        subprocess.run(['docker', 'rm', '-f', 'opsbox16c-app'])
        Path('/etc/hosts').write_bytes(hosts)
        subprocess.run(['systemctl', 'start', 'opsbox-proxy'])
        time.sleep(.5)
        ok('set_routes', {'routes': routes_before}, proxy=True)
        for domain in ['old.test', 'new.test']:
            ok('remove_cert', {'domain': domain}, proxy=True)
        cert.unlink(missing_ok=True)
        key.unlink(missing_ok=True)
        certroot.rmdir()
        print('Original web fixture, hosts and proxy routes restored.', flush=True)


if __name__ == '__main__':
    main()
