// Mock backend. Every function here maps 1:1 to a REST endpoint the Rust server
// will expose later (see docs/API.md). Swap this file for a fetch() client and the
// UI keeps working. All state lives in memory and resets on reload.

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lag = (ms = 120) => sleep(ms + Math.random() * 120);
const now = () => Date.now();
const clone = (o) => JSON.parse(JSON.stringify(o));
const hhmm = (t = now()) => new Date(t).toTimeString().slice(0, 5);
const hhmmss = (t = now()) => new Date(t).toTimeString().slice(0, 8);
const MIN = 60000, HOUR = 3600000, DAY = 86400000;

export const MODE = 'mock';

// ---------------------------------------------------------------- state
const S = {
  host: { name: 'srv-sgn-01', os: 'Ubuntu 24.04 LTS', kernel: '6.8.0-48', ip: '10.0.1.4', cores: 8, memTotal: 8, diskTotal: 120, upDays: 41, version: '0.1.0' },
  metrics: {
    cpu: Array.from({ length: 24 }, (_, i) => 22 + Math.round(8 * Math.sin(i / 2.2) + (i * 7919 % 11))),
    mem: Array.from({ length: 24 }, () => 39),
    disk: 98, netIn: 4.2, netOut: 0.8, conns: 142, load: [0.62, 0.48, 0.41], memUsed: 3.1, cache: 2.4,
  },
  procs: [
    { name: 'postgres', pid: 1204, cpu: 8.1, mem: 612 }, { name: 'node', pid: 2231, cpu: 6.4, mem: 388 },
    { name: 'dockerd', pid: 887, cpu: 3.0, mem: 142 }, { name: 'nginx', pid: 1102, cpu: 1.2, mem: 38 },
    { name: 'opsbox', pid: 412, cpu: 0.1, mem: 14, self: true },
  ],
  containers: [
    { id: 'c1', name: 'web-app', image: 'node:22-alpine', state: 'running', since: now() - 3 * DAY, cpu: 2.1, mem: 210, ports: '3000→3000' },
    { id: 'c2', name: 'postgres-15', image: 'postgres:15', state: 'running', since: now() - 3 * DAY, cpu: 8.1, mem: 612, ports: '5432 internal' },
    { id: 'c3', name: 'redis', image: 'redis:7', state: 'running', since: now() - 3 * DAY, cpu: 0.4, mem: 24, ports: '6379 internal' },
    { id: 'c4', name: 'nginx-proxy', image: 'nginx:1.27', state: 'running', since: now() - 2 * HOUR, cpu: 1.2, mem: 38, ports: '80, 443', restarts: 3 },
    { id: 'c5', name: 'worker-old', image: 'acme/worker:1.2', state: 'exited', since: now() - 6 * DAY, cpu: 0, mem: 0, ports: '–' },
  ],
  images: [
    { name: 'node', tag: '22-alpine', size: 0.18, used: true }, { name: 'postgres', tag: '15', size: 0.43, used: true },
    { name: 'redis', tag: '7', size: 0.13, used: true }, { name: 'nginx', tag: '1.27', size: 0.19, used: true },
    { name: 'acme/worker', tag: '1.2', size: 0.31, used: true }, { name: '<none>', tag: '<none>', size: 1.4, used: false },
  ],
  volumes: [{ name: 'pgdata', size: 4.2, used: true }, { name: 'redisdata', size: 0.02, used: true }, { name: 'old_mysql', size: 1.4, used: false }],
  networks: [{ name: 'bridge', driver: 'bridge' }, { name: 'shop-platform', driver: 'bridge' }, { name: 'host', driver: 'host' }],
  project: {
    name: 'shop-platform', env: 'production',
    services: [
      { id: 'web', kind: 'app', state: 'active', repo: 'acme/shop', branch: 'main', build: 'Dockerfile', domain: 'shop.example.com', port: 3000, x: 0, y: 98,
        vars: [['NODE_ENV', 'production'], ['DATABASE_URL', 'postgres://app:••••@postgres:5432/app'], ['REDIS_URL', 'redis://redis:6379']],
        deploys: [
          { id: 'a3f9c21', msg: 'Fix cart when out of stock', status: 'live', at: now() - 2 * MIN, secs: 48 },
          { id: '7be1d04', msg: 'Add checkout page', status: 'replaced', at: now() - DAY, secs: 51 },
          { id: '1c80ae7', msg: 'Update dependencies', status: 'failed', at: now() - 3 * DAY, secs: 33 },
        ] },
      { id: 'api', kind: 'app', state: 'building', repo: 'acme/shop-api', branch: 'main', build: 'Nixpacks', domain: 'api.shop.example.com', port: 4000, x: 370, y: 10, vars: [['PORT', '4000']], deploys: [{ id: '5d2e8b0', msg: 'Add orders endpoint', status: 'building', at: now() - 40000, secs: 0 }] },
      { id: 'worker', kind: 'app', state: 'active', repo: 'acme/shop-api', branch: 'main', build: 'npm run worker', domain: '', port: 0, x: 370, y: 186, vars: [], deploys: [{ id: '5c1a7f4', msg: 'Retry failed jobs', status: 'live', at: now() - 2 * DAY, secs: 40 }] },
      { id: 'postgres', kind: 'db', state: 'active', repo: 'PostgreSQL 16 · volume 4.2 GB', domain: '', x: 740, y: 10, note: 'backup daily 02:00', deploys: [], vars: [] },
      { id: 'redis', kind: 'db', state: 'active', repo: 'Redis 7 · 24 MB', domain: '', x: 740, y: 186, note: 'internal only', deploys: [], vars: [] },
    ],
  },
  repos: [
    { name: 'acme/shop', vis: 'private', lang: 'TypeScript', pushed: '2 min ago', build: 'Dockerfile' },
    { name: 'acme/shop-api', vis: 'private', lang: 'TypeScript', pushed: '2 min ago', build: 'Nixpacks · Node 22' },
    { name: 'acme/landing', vis: 'public', lang: 'Astro', pushed: '4 days ago', build: 'static site' },
    { name: 'acme/billing-worker', vis: 'private', lang: 'Go', pushed: 'last week', build: 'Nixpacks · Go 1.23' },
    { name: 'acme/infra-notes', vis: 'private', lang: 'Markdown', pushed: '2 weeks ago', build: 'nothing to build' },
  ],
  updates: [
    { pkg: 'openssl', cur: '3.0.13-0ubuntu3.4', next: '3.0.13-0ubuntu3.5', sec: true },
    { pkg: 'libc6', cur: '2.39-0ubuntu8.2', next: '2.39-0ubuntu8.3', sec: true },
    { pkg: 'linux-image-generic', cur: '6.8.0-48', next: '6.8.0-52', sec: true, kernel: true },
    { pkg: 'curl', cur: '8.5.0-2ubuntu10.4', next: '8.5.0-2ubuntu10.5', sec: false },
    { pkg: 'tzdata', cur: '2024a-3ubuntu1.1', next: '2024b-0ubuntu0.24.04', sec: false },
  ],
  extraUpdates: 9, rebootRequired: false, autoUpdates: true, sshOpenUntil: 0,
  units: [
    { name: 'opsbox.service', state: 'active', boot: 'enabled', mem: 14 }, { name: 'docker.service', state: 'active', boot: 'enabled', mem: 142 },
    { name: 'chrony.service', state: 'active', boot: 'enabled', mem: 3 }, { name: 'snapd.service', state: 'failed', boot: 'enabled', mem: 0 },
    { name: 'ssh.service', state: 'inactive', boot: 'disabled', mem: 0 },
  ],
  jobs: [
    { id: 'j1', cron: '0 2 * * *', name: 'Back up postgres to /var/backups', last: 'today 02:00', ok: true },
    { id: 'j2', cron: '0 3 * * 0', name: 'Safe cleanup (journal, apt, tmp)', last: 'Sun 03:00', ok: true, note: '2.1 GB' },
    { id: 'j3', cron: '17 */12 * * *', name: 'Renew TLS certificates', last: 'today 03:17', ok: true },
  ],
  firewall: [
    { id: 'f1', port: '443', proto: 'tcp', src: 'anywhere', note: 'HTTPS for apps and this panel' },
    { id: 'f2', port: '80', proto: 'tcp', src: 'anywhere', note: 'Redirect to HTTPS, certificate checks' },
    { id: 'f3', port: '8443', proto: 'tcp', src: '203.0.113.0/24', note: 'Admin port, office network only' },
  ],
  cleanup: [
    { id: 'docker', title: 'Docker: unused images and build cache', desc: '8 dangling images, 3 older than 30 days · running containers untouched', gb: 9.8, on: true, color: 'accent' },
    { id: 'journal', title: 'systemd journal older than 7 days', desc: '/var/log/journal · keeps the last 7 days', gb: 2.1, on: true, color: 'text' },
    { id: 'apt', title: 'apt cache and unused packages', desc: 'apt clean + autoremove · 4 orphaned packages', gb: 1.2, on: true, color: 'warn' },
    { id: 'tmp', title: '/tmp and rotated logs (.gz, .1)', desc: 'Files not accessed in 10+ days', gb: 0.7, on: true, color: 'muted' },
    { id: 'volumes', title: 'Docker volumes not attached to any container', desc: '▲ May contain data · unchecked by default · 2 volumes', gb: 1.4, on: false, warn: true, color: 'ls' },
  ],
  autoCleanup: true,
  users: [{ name: 'admin', role: 'Full access', tfa: true }, { name: 'minh.dev', role: 'Deploy and view logs, no terminal', tfa: false }],
  audit: [
    { t: now() - 15 * MIN, user: 'minh.dev', action: 'deploy web a3f9c21', ip: '203.0.113.21' },
    { t: now() - 30 * MIN, user: 'admin', action: 'restart web-app', ip: '203.0.113.5' },
    { t: now() - 50 * MIN, user: 'admin', action: 'open terminal srv-sgn-01', ip: '203.0.113.5' },
    { t: now() - 65 * MIN, user: 'unknown', action: 'failed sign-in ×5, IP locked 15 min', ip: '203.0.113.9', warn: true },
  ],
  checks: [
    { id: 'tls', t: "TLS (HTTPS) · Let's Encrypt auto-renew", ok: true },
    { id: '2fa', t: 'Two-factor authentication (TOTP) for admin', ok: true },
    { id: 'lock', t: 'Lock out after 5 failed sign-ins (15 min)', ok: true },
    { id: 'ip', t: 'Allow listed IPs only', ok: false, action: 'Enable' },
    { id: 'bind', t: 'Admin port bound to 127.0.0.1:8443 (reach via proxy or tunnel)', ok: true },
    { id: 'keys', t: 'Private SSH keys encrypted at rest', ok: true },
    { id: 'sshd', t: 'SSH daemon on this server (port 22 closed, managed through opsbox)', ok: true, label: 'off', action: 'Enable for 1 hour' },
    { id: 'wd', t: 'Watchdog restarts opsbox if it stops answering', ok: true },
    { id: 'rb', t: 'Automatic rollback if an opsbox update fails its health check', ok: true },
  ],
  allowIps: '203.0.113.0/24, 198.51.100.7',
  recovery: { failed: false },
};

// virtual file system
const F = {};
const addDir = (p, mtime = '03:21', mode = 'drwxr-xr-x') => (F[p] = { type: 'd', mode, mtime });
const addFile = (p, content, mode = '-rw-r--r--', mtime = '5 days ago', size) => (F[p] = { type: 'f', mode, mtime, content, size: size ?? content.length });
['/', '/var', '/var/www', '/var/www/app', '/var/www/app/public', '/var/www/app/logs', '/var/backups', '/etc', '/etc/opsbox', '/home', '/home/deploy', '/tmp'].forEach((d) => addDir(d));
F['/var/www/app/logs'].mode = 'drwxr-x---';
addFile('/var/www/app/.env', 'NODE_ENV=production\nPORT=3000\nDATABASE_URL=postgres://app:secret@postgres-15:5432/app\nREDIS_URL=redis://redis:6379\nSESSION_SECRET=change-me-please', '-rw-r--r--', '3 days ago');
addFile('/var/www/app/docker-compose.yml', 'services:\n  web:\n    build: .\n    ports: ["3000:3000"]\n    env_file: .env\n  redis:\n    image: redis:7\n', '-rw-r--r--', '5 days ago');
addFile('/var/www/app/package.json', '{\n  "name": "shop",\n  "version": "1.4.2",\n  "scripts": { "start": "node dist/server.js" }\n}\n', '-rw-r--r--', '5 days ago');
addFile('/var/www/app/public/index.html', '<!doctype html>\n<title>shop</title>\n', '-rw-r--r--', 'yesterday');
addFile('/var/www/app/logs/app.log', '03:21:07 INFO server listening :3000\n03:21:14 WARN slow query 412ms\n', '-rw-r-----', '03:21');
addFile('/etc/opsbox/config.toml', '[server]\nbind = "127.0.0.1:8443"\n\n[security]\nlockout_after = 5\n', '-rw-------', 'last week');
addFile('/home/deploy/backup-2025.tar.gz', '(binary)', '-rw-r--r--', '8 months ago', 2.3 * 1024 ** 3);
const norm = (p) => { const out = []; for (const s of p.split('/')) { if (!s || s === '.') continue; if (s === '..') out.pop(); else out.push(s); } return '/' + out.join('/'); };
const join = (a, b) => norm(b.startsWith('/') ? b : a + '/' + b);
const childrenOf = (p) => Object.keys(F).filter((k) => k !== p && norm(k.split('/').slice(0, -1).join('/') || '/') === norm(p));

function audit(action, user = 'admin', warn = false) { S.audit.unshift({ t: now(), user, action, ip: '203.0.113.5', warn }); }

// ---------------------------------------------------------------- auth
export const auth = {
  async login(user, pass, code) {
    await lag(300);
    if (!user || !pass) throw new Error('Enter a username and password.');
    if (!/^\d{6}$/.test(code || '')) throw new Error('Enter the 6-digit authentication code.');
    audit('sign-in', user); return { user };
  },
  async claim(token) {
    await lag(300);
    if (!/^[A-Z0-9]{4}(-[A-Z0-9]{4}){2}$/i.test((token || '').trim())) throw new Error('That token does not look right. Format: XXXX-XXXX-XXXX');
    return { ok: true };
  },
};

// ---------------------------------------------------------------- metrics
export function tick() {
  const m = S.metrics, walk = (v, s, lo, hi) => Math.min(hi, Math.max(lo, v + (Math.random() - 0.5) * s));
  const cpu = walk(m.cpu[m.cpu.length - 1], 14, 8, 62);
  m.cpu.push(Math.round(cpu)); m.cpu.shift();
  m.memUsed = walk(m.memUsed, 0.05, 2.8, 3.6);
  m.mem.push(Math.round(m.memUsed / S.host.memTotal * 100)); m.mem.shift();
  m.netIn = walk(m.netIn, 2, 0.4, 12); m.netOut = walk(m.netOut, 0.6, 0.1, 3); m.conns = Math.round(walk(m.conns, 8, 90, 220));
  m.load = [walk(m.load[0], .2, .2, 2.5), walk(m.load[1], .08, .2, 2), walk(m.load[2], .04, .2, 1.6)];
  S.procs.forEach((p) => { p.cpu = Math.max(0.1, walk(p.cpu, p.cpu * .4 + .2, 0.1, 30)); });
  S.containers.forEach((c) => { if (c.state === 'running') c.cpu = Math.max(0.1, walk(c.cpu, c.cpu * .3 + .2, 0.1, 40)); });
}
export async function getHost() { await lag(30); return clone(S.host); }
export async function getMetrics() {
  const m = S.metrics;
  return { cpu: [...m.cpu], mem: [...m.mem], cpuNow: m.cpu[m.cpu.length - 1], memUsed: m.memUsed, memTotal: S.host.memTotal, cache: m.cache,
    disk: m.disk, diskTotal: S.host.diskTotal, netIn: m.netIn, netOut: m.netOut, conns: m.conns, load: [...m.load], procs: clone(S.procs).sort((a, b) => b.cpu - a.cpu) };
}
export async function getAttention() {
  const sec = S.updates.filter((u) => u.sec).length;
  const out = [];
  if (sec) out.push({ kind: 'security', text: `${sec} security updates are waiting (${S.updates.filter((u) => u.sec).map((u) => u.pkg.replace('-generic', '')).join(', ')})`, link: ['Review and install →', '#/system'] });
  if (S.rebootRequired) out.push({ kind: 'reboot', text: 'A new kernel is installed. Reboot to use it.', link: ['Schedule reboot →', '#/system'] });
  const used = S.metrics.disk / S.host.diskTotal * 100;
  if (used > 75) out.push({ kind: 'disk', text: `Disk / is at ${Math.round(used)}%. ${S.cleanup.reduce((a, c) => a + c.gb, 0).toFixed(1)} GB can be reclaimed.`, link: ['Clean up →', '#/cleanup'] });
  return out;
}

// ---------------------------------------------------------------- containers
const LOGS = ['INFO  GET /api/health 200 3ms', 'INFO  GET /api/items 200 21ms', 'WARN  slow query 412ms items.list', 'INFO  POST /api/login 200 88ms', 'ERROR upstream timeout cache:6379', 'INFO  retry 1/3 ok'];
export async function listContainers() { await lag(); return clone(S.containers); }
export async function listDocker(kind) { await lag(); return clone(S[kind]); }
export async function containerAction(id, action) {
  await lag(350);
  const c = S.containers.find((x) => x.id === id); if (!c) throw new Error('No such container');
  if (action === 'restart') { c.state = 'running'; c.since = now(); }
  if (action === 'start') { c.state = 'running'; c.since = now(); c.mem = 30; c.cpu = 0.5; }
  if (action === 'stop') { c.state = 'exited'; c.cpu = 0; c.mem = 0; }
  if (action === 'delete') S.containers = S.containers.filter((x) => x.id !== id);
  audit(`${action} ${c.name}`); return { ok: true };
}
export async function containerLogs(id) {
  await lag(80);
  const c = S.containers.find((x) => x.id === id); if (!c) return [];
  const t = now(); return Array.from({ length: 9 }, (_, i) => `${hhmmss(t - (9 - i) * 7000)} ${LOGS[(i * 5 + id.length) % LOGS.length]}`);
}

// ---------------------------------------------------------------- apps
export async function getProject() { await lag(); return clone(S.project); }
export async function getRepos() { await lag(250); return clone(S.repos); }
export async function createService(cfg) {
  await lag(500);
  const name = (cfg.name || '').trim();
  if (!/^[a-z][a-z0-9-]{1,30}$/.test(name)) throw new Error('Service name: lowercase letters, numbers and dashes, starting with a letter.');
  if (S.project.services.some((s) => s.id === name)) throw new Error(`A service named "${name}" already exists.`);
  const id = Math.random().toString(16).slice(2, 9);
  const idx = S.project.services.length;
  S.project.services.push({ id: name, kind: 'app', state: 'building', repo: cfg.repo, branch: cfg.branch, build: cfg.build, domain: cfg.domain ? `${cfg.domain}.example.com` : '', port: +cfg.port || 3000,
    x: 370 * (idx % 3), y: 10 + 176 * Math.floor(idx / 3), vars: cfg.vars || [], deploys: [{ id, msg: 'Initial deploy', status: 'building', at: now(), secs: 0 }] });
  setTimeout(() => { const s = S.project.services.find((x) => x.id === name); if (s) { s.state = 'active'; s.deploys[0].status = 'live'; s.deploys[0].secs = 52; } }, 6000);
  audit(`deploy ${name} ${id}`); return { id };
}
export async function redeploy(svc) {
  await lag(300);
  const s = S.project.services.find((x) => x.id === svc); if (!s) throw new Error('No such service');
  const id = Math.random().toString(16).slice(2, 9);
  s.deploys.forEach((d) => { if (d.status === 'live') d.status = 'replaced'; });
  s.deploys.unshift({ id, msg: 'Manual redeploy', status: 'building', at: now(), secs: 0 }); s.state = 'building';
  setTimeout(() => { s.state = 'active'; s.deploys[0].status = 'live'; s.deploys[0].secs = 44; }, 4000);
  audit(`redeploy ${svc} ${id}`); return { id };
}
export async function rollback(svc, deployId) {
  await lag(400);
  const s = S.project.services.find((x) => x.id === svc); if (!s) throw new Error('No such service');
  s.deploys.forEach((d) => { if (d.status === 'live') d.status = 'replaced'; if (d.id === deployId) d.status = 'live'; });
  audit(`rollback ${svc} to ${deployId}`); return { ok: true };
}
export function buildLog(svc, deploy) {
  const t = deploy.at; const L = (s, m) => `${hhmmss(t + s * 1000)} ${m}`;
  const lines = [L(0, `clone    ${svc.repo}@${deploy.id}`), L(1, `detect   ${svc.build}`), L(1, 'build    FROM node:22-alpine'), L(19, 'build    npm ci · 412 packages'), L(42, 'build    npm run build · ok'),
    L(47, `push     ${svc.id}:${deploy.id} (118 MB)`), L(48, 'deploy   start new version, wait for /healthz'), L(51, 'health   200 OK after 3 s'), L(51, 'switch   traffic moved, zero downtime'), L(52, 'done     stopped previous version')];
  if (deploy.status === 'failed') return [...lines.slice(0, 4), L(30, 'ERROR    npm ci exited with code 1'), L(30, 'ERROR    peer dependency conflict: react@19 vs react-dom@18'), L(31, 'abort    build failed, previous version kept running')];
  if (deploy.status === 'building') return lines.slice(0, 4);
  return lines;
}

// ---------------------------------------------------------------- system
export async function getUpdates() {
  await lag();
  return { list: clone(S.updates), total: S.updates.length + S.extraUpdates, security: S.updates.filter((u) => u.sec).length, reboot: S.rebootRequired, auto: S.autoUpdates, running: S.host.kernel, checked: '03:30' };
}
export async function installUpdates(which) {
  await lag(1400);
  const before = S.updates.length;
  if (which === 'security') { S.updates.some((u) => u.sec && u.kernel) && (S.rebootRequired = true, S.host.kernelNew = '6.8.0-52'); S.updates = S.updates.filter((u) => !u.sec); }
  else { if (S.updates.some((u) => u.kernel)) S.rebootRequired = true; S.updates = []; S.extraUpdates = 0; }
  audit(`install ${which} updates (${before - S.updates.length})`); return { installed: before - S.updates.length };
}
export async function setAutoUpdates(on) { await lag(150); S.autoUpdates = on; audit(`auto security updates ${on ? 'on' : 'off'}`); }
export async function reboot() { await lag(500); S.rebootRequired = false; S.host.kernel = S.host.kernelNew || S.host.kernel; S.host.upDays = 0; audit('reboot instance'); return { ok: true }; }
export async function getUnits() { await lag(); return clone(S.units); }
export async function unitAction(name, action) {
  await lag(400);
  const u = S.units.find((x) => x.name === name); if (!u) throw new Error('No such unit');
  if (action === 'restart') u.state = 'active'; if (action === 'start') { u.state = 'active'; if (name === 'ssh.service') { S.sshOpenUntil = now() + HOUR; u.boot = 'disabled'; } }
  if (action === 'stop') u.state = 'inactive';
  audit(`${action} ${name}`); return { ok: true };
}
const JOURNAL = [
  ['opsbox[412]', 6, 'deploy web a3f9c21 queued by minh.dev'], ['dockerd[887]', 6, 'Container web-app health_status: healthy'],
  ['systemd[1]', 3, 'snapd.service: Main process exited, code=exited, status=1'], ['systemd[1]', 3, "snapd.service: Failed with result 'exit-code'."],
  ['kernel', 4, '[UFW BLOCK] IN=eth0 SRC=203.0.113.9 DST=10.0.1.4 DPT=22'], ['chrony[640]', 6, 'Selected source 169.254.169.123'],
  ['sshd[9121]', 4, 'Failed password for root from 203.0.113.9'], ['opsbox[412]', 6, 'admin restart web-app'],
];
export async function getJournal({ unit = '', prio = 6, q = '' } = {}) {
  await lag(100); const t = now();
  return JOURNAL.map((j, i) => ({ t: t - (JOURNAL.length - i) * 4 * MIN, proc: j[0], prio: j[1], msg: j[2] }))
    .filter((l) => (!unit || l.proc.startsWith(unit.replace('.service', ''))) && l.prio <= prio && (!q || (l.proc + l.msg).toLowerCase().includes(q.toLowerCase())));
}
export async function getJobs() { await lag(); return clone(S.jobs); }
export async function runJob(id) { await lag(700); const j = S.jobs.find((x) => x.id === id); if (!j) throw new Error('No such job'); j.last = `today ${hhmm()}`; audit(`run job: ${j.name}`); return { ok: true }; }
export async function addJob(job) {
  await lag(250);
  if (!/^(\S+\s+){4}\S+$/.test((job.cron || '').trim())) throw new Error('Schedule must have 5 fields, e.g. 0 2 * * *');
  if (!(job.name || '').trim()) throw new Error('Describe what the job does.');
  S.jobs.push({ id: 'j' + (S.jobs.length + 1) + Date.now() % 1000, cron: job.cron.trim(), name: job.name.trim(), last: 'never', ok: true }); audit(`add job: ${job.name}`);
}
export async function getFirewall() { await lag(); return { rules: clone(S.firewall), sshOpen: S.sshOpenUntil > now() }; }
export async function addRule(r) {
  await lag(250);
  const port = parseInt(r.port, 10);
  if (!(port >= 1 && port <= 65535)) throw new Error('Port must be between 1 and 65535.');
  if (!/^(anywhere|\d{1,3}(\.\d{1,3}){3}(\/\d{1,2})?)$/.test(r.src || 'anywhere')) throw new Error('Source must be "anywhere" or an IP / CIDR such as 203.0.113.0/24.');
  S.firewall.push({ id: 'f' + Date.now(), port: String(port), proto: r.proto || 'tcp', src: r.src || 'anywhere', note: r.note || '' }); audit(`firewall allow ${port}/${r.proto || 'tcp'} from ${r.src || 'anywhere'}`);
}
export async function removeRule(id) { await lag(250); const r = S.firewall.find((x) => x.id === id); S.firewall = S.firewall.filter((x) => x.id !== id); audit(`firewall remove ${r ? r.port : id}`); }
export async function openSsh() { await lag(400); S.sshOpenUntil = now() + HOUR; S.units.find((u) => u.name === 'ssh.service').state = 'active'; audit('emergency SSH enabled for 1 hour', 'admin', true); return { until: S.sshOpenUntil }; }

// ---------------------------------------------------------------- cleanup
export async function scanCleanup() {
  await lag(500);
  return { items: clone(S.cleanup), disk: S.metrics.disk, total: S.host.diskTotal, system: 41, auto: S.autoCleanup, scanned: hhmm() };
}
export async function runCleanup(ids) {
  await lag(1200);
  const hit = S.cleanup.filter((c) => ids.includes(c.id)); const freed = hit.reduce((a, c) => a + c.gb, 0);
  S.cleanup = S.cleanup.filter((c) => !ids.includes(c.id)); S.metrics.disk = Math.max(20, S.metrics.disk - freed);
  audit(`cleanup ${hit.map((c) => c.id).join(', ')} −${freed.toFixed(1)} GB`); return { freed };
}
export async function setAutoCleanup(on) { await lag(150); S.autoCleanup = on; }

// ---------------------------------------------------------------- files
export async function listDir(path) {
  await lag(80); const p = norm(path); if (!F[p] || F[p].type !== 'd') throw new Error(`Not a directory: ${p}`);
  const kids = childrenOf(p).map((k) => ({ name: k.split('/').pop(), path: k, ...F[k] }));
  kids.sort((a, b) => (a.type === b.type ? a.name.localeCompare(b.name) : a.type === 'd' ? -1 : 1));
  return { path: p, entries: kids.map((k) => ({ name: k.name, path: k.path, type: k.type, mode: k.mode, mtime: k.mtime, size: k.size ?? null, warn: k.name.startsWith('.env') && k.mode !== '-rw-------' })) };
}
export async function readFile(path) { await lag(60); const f = F[norm(path)]; if (!f || f.type !== 'f') throw new Error('Not a file'); return { path: norm(path), content: f.content, mode: f.mode }; }
export async function writeFile(path, content) { await lag(200); const f = F[norm(path)]; if (!f) throw new Error('No such file'); f.content = content; f.size = content.length; f.mtime = 'just now'; audit(`edit ${norm(path)}`); }
export async function chmod(path, mode) {
  await lag(200); const f = F[norm(path)]; if (!f) throw new Error('No such file');
  if (!/^[0-7]{3}$/.test(mode)) throw new Error('Mode must be 3 octal digits, e.g. 600');
  const bits = ['---', '--x', '-w-', '-wx', 'r--', 'r-x', 'rw-', 'rwx']; f.mode = (f.type === 'd' ? 'd' : '-') + [...mode].map((d) => bits[+d]).join(''); audit(`chmod ${mode} ${norm(path)}`);
}
export async function mkdir(path) { await lag(150); const p = norm(path); if (F[p]) throw new Error('Already exists'); addDir(p, 'just now'); audit(`mkdir ${p}`); }
export async function removePaths(paths) { await lag(250); paths.forEach((p) => { Object.keys(F).filter((k) => k === p || k.startsWith(p + '/')).forEach((k) => delete F[k]); }); audit(`delete ${paths.join(', ')}`); }
export async function upload(dir, file) { await lag(300); addFile(join(dir, file.name), '(uploaded)', '-rw-r--r--', 'just now', file.size); audit(`upload ${join(dir, file.name)}`); }

// ---------------------------------------------------------------- security
export async function getSecurity() {
  await lag(); const sshOpen = S.sshOpenUntil > now();
  const checks = clone(S.checks).map((c) => (c.id === 'sshd' ? { ...c, ok: !sshOpen, label: sshOpen ? 'on' : 'off', warn: sshOpen } : c));
  return { checks, users: clone(S.users), audit: clone(S.audit).slice(0, 12), allowIps: S.allowIps, passing: checks.filter((c) => c.ok).length, total: checks.length };
}
export async function toggleCheck(id) { await lag(200); const c = S.checks.find((x) => x.id === id); if (c) { c.ok = true; delete c.action; } audit(`enable ${id}`); }
export async function setAllowIps(v) {
  await lag(200);
  const bad = v.split(',').map((s) => s.trim()).filter(Boolean).find((s) => !/^\d{1,3}(\.\d{1,3}){3}(\/\d{1,2})?$/.test(s));
  if (bad) throw new Error(`"${bad}" is not a valid IP or CIDR.`); S.allowIps = v; audit('update allowed IPs');
}
export async function inviteUser(name) { await lag(250); if (!/^[a-z][a-z0-9._-]{2,}$/.test(name)) throw new Error('Username: at least 3 characters, lowercase.'); S.users.push({ name, role: 'Deploy and view logs, no terminal', tfa: false }); audit(`invite ${name}`); }
export async function signOutAll() { await lag(300); audit('sign out all sessions'); }

// ---------------------------------------------------------------- recovery
export async function getRecovery() {
  await lag(100); const failed = S.recovery.failed;
  return { failed, checks: failed
    ? [['opsbox.service', 'fail', 'failed', 'exit code 101 · restarted 3 times'], ['Health endpoint /healthz', 'fail', 'no answer', 'timeout after 5 s'], ['Disk /', 'fail', '100% full', '0 B free · likely cause: opsbox cannot write its database'], ['Docker', 'ok', 'running', '4 containers still serving traffic'], ['Network and DNS', 'ok', 'ok', '443 reachable from outside']]
    : [['opsbox.service', 'ok', 'active', 'running 3 days'], ['Health endpoint /healthz', 'ok', '200 OK', 'answered in 4 ms'], ['Disk /', 'warn', '82%', '22 GB free'], ['Docker', 'ok', 'running', '4 containers'], ['Network and DNS', 'ok', 'ok', '443 reachable from outside']],
    log: failed ? ['03:51:44 INFO  opsbox 0.1.4 starting', '03:51:46 ERROR sqlite: database or disk is full (code 13)', '03:51:47 FATAL cannot persist session store, exiting', '03:51:47 watchdog: exited with code 101, restart 1/3', '03:52:19 watchdog: restart 3/3 failed, entering safe mode'] : ['03:51:44 INFO  opsbox 0.1.0 starting', '03:51:45 INFO  listening on 127.0.0.1:8443'] };
}
export async function simulateFailure() { S.recovery.failed = true; }
export async function recoveryAction(a) {
  await lag(900);
  if (a === 'free') { S.metrics.disk = 84; S.recovery.failed = false; return { msg: 'Freed 14.6 GB. opsbox restarted and is healthy.' }; }
  if (a === 'restart') { if (S.metrics.disk > 95) throw new Error('Restart failed again: disk is still full. Free disk space first.'); S.recovery.failed = false; return { msg: 'opsbox restarted and passed its health check.' }; }
  if (a === 'rollback') { S.recovery.failed = false; return { msg: 'Rolled back to 0.1.3. Settings and data kept.' }; }
  if (a === 'ssh') { S.sshOpenUntil = now() + HOUR; return { msg: 'Port 22 is open to your IP for one hour.' }; }
  if (a === 'reboot') { S.recovery.failed = false; return { msg: 'Rebooting… apps will return in about a minute.' }; }
  throw new Error('Unknown action');
}

// ---------------------------------------------------------------- terminal
export const HOSTS = [
  { id: 'local', name: 'srv-sgn-01', sub: 'local · root', user: 'root' },
  { id: 'db', name: 'db-primary', sub: 'deploy@10.0.1.12 · ed25519', user: 'deploy' },
  { id: 'edge', name: 'edge-hn-02', sub: 'ubuntu@203.0.113.40:2222', user: 'ubuntu' },
];
export async function termExec(host, cwd, line) {
  await sleep(40);
  const [cmd, ...args] = line.trim().split(/\s+/); const m = S.metrics;
  const dir = (a) => join(cwd, a || '~'.replace('~', '/home/deploy'));
  switch (cmd) {
    case '': return { out: '' };
    case 'help': return { out: 'Demo shell. Try: ls, cd, cat, pwd, whoami, hostname, uname -a, uptime, df -h, free -m, docker ps, ps, date, echo, clear' };
    case 'pwd': return { out: cwd };
    case 'whoami': return { out: HOSTS.find((h) => h.id === host).user };
    case 'hostname': return { out: HOSTS.find((h) => h.id === host).name };
    case 'echo': return { out: args.join(' ') };
    case 'date': return { out: new Date().toString() };
    case 'clear': return { clear: true };
    case 'uname': return { out: `Linux ${HOSTS.find((h) => h.id === host).name} ${S.host.kernel}-generic #48-Ubuntu SMP x86_64 GNU/Linux` };
    case 'uptime': return { out: ` ${hhmmss()} up ${S.host.upDays} days,  2 users,  load average: ${m.load.map((x) => x.toFixed(2)).join(', ')}` };
    case 'df': return { out: `Filesystem      Size  Used Avail Use% Mounted on\n/dev/vda1       ${S.host.diskTotal}G   ${Math.round(m.disk)}G   ${Math.round(S.host.diskTotal - m.disk)}G  ${Math.round(m.disk / S.host.diskTotal * 100)}% /` };
    case 'free': return { out: `               total        used        free      cache\nMem:        ${Math.round(S.host.memTotal * 1024)}        ${Math.round(m.memUsed * 1024)}        ${Math.round((S.host.memTotal - m.memUsed) * 1024)}      ${Math.round(m.cache * 1024)}` };
    case 'ps': return { out: 'PID   CPU%  MEM   COMMAND\n' + S.procs.map((p) => `${String(p.pid).padEnd(5)} ${p.cpu.toFixed(1).padStart(4)}  ${String(p.mem).padStart(4)}M ${p.name}`).join('\n') };
    case 'docker': if (args[0] === 'ps') return { out: 'NAMES         STATUS\n' + S.containers.filter((c) => c.state === 'running').map((c) => `${c.name.padEnd(13)} Up ${Math.max(1, Math.round((now() - c.since) / HOUR))} hours`).join('\n') }; return { out: 'docker: demo shell supports only "docker ps"' };
    case 'ls': { try { const r = await listDir(dir(args.filter((a) => !a.startsWith('-'))[0] || '.')); return { out: r.entries.map((e) => (e.type === 'd' ? e.name + '/' : e.name)).join('  ') || '(empty)' }; } catch (e) { return { err: `ls: ${e.message}` }; } }
    case 'cd': { const p = args[0] ? join(cwd, args[0]) : '/home/deploy'; if (!F[p] || F[p].type !== 'd') return { err: `cd: no such directory: ${args[0]}` }; return { cwd: p }; }
    case 'cat': { const f = F[join(cwd, args[0] || '')]; if (!f || f.type !== 'f') return { err: `cat: ${args[0] || ''}: No such file` }; return { out: f.content.replace(/(SESSION_SECRET|DATABASE_URL)=.*/g, '$1=••••••') }; }
    default: return { err: `${cmd}: command not found (demo shell, type "help")` };
  }
}

// ---------------------------------------------------------------- small extras
export async function getReclaimable() { return { gb: S.cleanup.reduce((a, c) => a + c.gb, 0), parts: S.cleanup.map((c) => [c.id, c.gb, c.color]) }; }
export async function getAudit(n = 5) { return clone(S.audit).slice(0, n); }
export async function addVar(svc, k, v) {
  await lag(150); const s = S.project.services.find((x) => x.id === svc); if (!s) throw new Error('No such service');
  if (!/^[A-Z_][A-Z0-9_]*$/.test(k)) throw new Error('Variable names use UPPER_CASE letters, numbers and underscores.');
  if (s.vars.some((x) => x[0] === k)) throw new Error(`${k} already exists.`); s.vars.push([k, v]); audit(`set variable ${k} on ${svc}`);
}
export async function removeVar(svc, k) { await lag(150); const s = S.project.services.find((x) => x.id === svc); s.vars = s.vars.filter((x) => x[0] !== k); audit(`remove variable ${k} on ${svc}`); }
export async function removeService(svc) { await lag(300); S.project.services = S.project.services.filter((x) => x.id !== svc); audit(`delete service ${svc}`); }

// ---------------------------------------------------------------- backups
const BK = {
  jobs: [
    { id: 'b1', what: 'postgres-15', note: 'database · pg_dump', cron: 'daily 02:00', keep: '14 daily, 8 weekly', dest: 'Offsite', last: 'ok · 1.2 GB', state: 'ok' },
    { id: 'b2', what: 'Volumes pgdata, redisdata', note: 'Docker volumes', cron: 'daily 02:20', keep: '7 daily', dest: 'Offsite', last: 'ok · 4.2 GB', state: 'ok' },
    { id: 'b3', what: '/var/www/app, /etc/opsbox', note: 'files and panel settings', cron: 'daily 02:40', keep: '7 daily', dest: 'Local disk', last: 'same disk', state: 'warn' },
  ],
  dests: [
    { id: 'd1', name: 'Offsite', type: 'S3-compatible', loc: 's3://acme-backups/srv-sgn-01', ok: true },
    { id: 'd2', name: 'Local disk', type: 'folder', loc: '/var/backups', ok: false },
  ],
  items: [
    { id: 'k1', at: 'today 02:00', what: 'postgres-15', size: 1.2, where: 'S3', check: 'restore-tested' },
    { id: 'k2', at: 'today 02:20', what: 'volumes', size: 4.2, where: 'S3', check: 'checksum ok' },
    { id: 'k3', at: 'yesterday 02:00', what: 'postgres-15', size: 1.2, where: 'S3', check: 'checksum ok' },
    { id: 'k4', at: 'Sun 02:00', what: 'postgres-15 (weekly)', size: 1.1, where: 'S3', check: 'checksum ok' },
  ],
};
export async function getBackups() { await lag(); return clone(BK); }
export async function runBackup(id) { await lag(900); audit(`backup ${id || 'all jobs'}`); const j = BK.jobs.find((x) => x.id === id); if (j) j.last = 'ok · just now'; return { ok: true }; }
export async function restoreBackup(id, mode) {
  await lag(1200); const k = BK.items.find((x) => x.id === id); if (!k) throw new Error('No such backup');
  if (mode !== 'copy' && mode !== 'replace') throw new Error('Choose how to restore.'); audit(`restore ${k.what} (${k.at}) as ${mode}`); return { msg: mode === 'copy' ? `Restored ${k.what} into ${k.what}-restored. Check it, then swap it in.` : `${k.what} replaced from ${k.at}.` };
}
export async function addDestination(d) {
  await lag(500);
  if (!(d.name || '').trim()) throw new Error('Give the destination a name.');
  if (!/^s3:\/\/[a-z0-9.-]{3,}(\/.*)?$/.test(d.loc || '')) throw new Error('Location must look like s3://bucket/path');
  BK.dests.push({ id: 'd' + Date.now(), name: d.name.trim(), type: 'S3-compatible', loc: d.loc, ok: true }); audit(`add backup destination ${d.name}`);
}

// ---------------------------------------------------------------- alerts
const AL = {
  firing: true,
  channels: [
    { id: 'c1', name: 'Email', to: 'admin@acme.co', for: 'all alerts', on: true },
    { id: 'c2', name: 'Slack', to: '#ops-alerts', for: 'critical only', on: true },
    { id: 'c3', name: 'Telegram', to: '', for: '', on: false },
    { id: 'c4', name: 'Webhook', to: '', for: '', on: false },
  ],
  rules: [
    ['r1', 'Disk full', 'disk / above 85% for 5 min', 'critical', 'Slack + email', true], ['r2', 'Disk filling', 'disk / above 75% for 5 min', 'warning', 'email', true],
    ['r3', 'High CPU', 'above 90% for 10 min', 'warning', 'email', true], ['r4', 'High memory', 'above 90% for 5 min', 'warning', 'email', true],
    ['r5', 'Service or container down', 'any running one stops', 'critical', 'Slack + email', true], ['r6', 'Deploy failed', 'any service', 'warning', 'Slack', true],
    ['r7', 'Backup failed or missed', 'any job', 'critical', 'Slack + email', true], ['r8', 'TLS certificate expiring', 'under 14 days', 'warning', 'email', true],
    ['r9', 'Security updates waiting', 'for more than 3 days', 'warning', 'email', true], ['r10', 'Repeated failed sign-ins', '5 or more from one IP', 'warning', 'email', false],
  ].map(([id, name, cond, level, notify, on]) => ({ id, name, cond, level, notify, on })),
  history: [
    { t: '03:12 today', name: 'Disk filling', msg: 'disk / reached 82%', state: 'firing' },
    { t: 'yesterday 14:08', name: 'Deploy failed', msg: 'shop-api 1c80ae7 · npm ci exited with code 1', state: 'resolved' },
    { t: 'Mon 09:41', name: 'TLS certificate expiring', msg: 'shop.example.com renewed automatically', state: 'resolved' },
  ],
  quiet: { from: '23:00', to: '07:00', tz: 'Asia/Ho_Chi_Minh' },
};
export async function getAlerts() { await lag(); return clone(AL); }
export async function setRule(id, on) { await lag(150); const r = AL.rules.find((x) => x.id === id); if (!r) throw new Error('No such rule'); r.on = on; audit(`alert rule ${r.name} ${on ? 'on' : 'off'}`); }
export async function testAlert(channel) { await lag(600); const c = AL.channels.find((x) => x.id === channel); if (channel && (!c || !c.on)) throw new Error('Connect this channel first.'); audit(`test alert${c ? ' to ' + c.name : ''}`); return { ok: true }; }
export async function connectChannel(id, to) {
  await lag(400); const c = AL.channels.find((x) => x.id === id); if (!c) throw new Error('No such channel');
  if (id === 'c4' && !/^https:\/\//.test(to || '')) throw new Error('Webhook URL must start with https://');
  if (id === 'c3' && !/^-?\d{5,}$/.test(to || '')) throw new Error('Enter your Telegram chat id (digits only).');
  c.to = to; c.for = 'all alerts'; c.on = true; audit(`connect ${c.name}`);
}
export async function silenceAlert() { await lag(150); AL.firing = false; AL.history[0].state = 'silenced'; audit('silence alert 1 h'); }
export async function setQuiet(q) { await lag(150); AL.quiet = { ...AL.quiet, ...q }; audit('update quiet hours'); }

// ---------------------------------------------------------------- templates
export const TEMPLATES = {
  databases: [
    { id: 'postgres', name: 'PostgreSQL', desc: 'Relational database · daily backup on by default', res: '≈ 100 MB RAM · 1 GB disk', versions: ['16.4', '16.3', '15.8'], port: 5432 },
    { id: 'mysql', name: 'MySQL', desc: 'Relational database', res: '≈ 400 MB RAM · 1 GB disk', versions: ['8.4', '8.0'], port: 3306 },
    { id: 'mariadb', name: 'MariaDB', desc: 'MySQL-compatible database', res: '≈ 150 MB RAM · 1 GB disk', versions: ['11.4'], port: 3306 },
    { id: 'redis', name: 'Redis', desc: 'Cache and queues', res: '≈ 20 MB RAM', versions: ['7.4'], port: 6379 },
    { id: 'mongo', name: 'MongoDB', desc: 'Document database', res: '≈ 300 MB RAM · 1 GB disk', versions: ['7.0'], port: 27017 },
  ],
  tools: [
    { id: 'minio', name: 'MinIO', desc: 'S3-compatible file storage', res: '≈ 150 MB RAM', versions: ['latest'], port: 9000 },
    { id: 'kuma', name: 'Uptime Kuma', desc: 'Watch your public URLs from outside', res: '≈ 100 MB RAM', versions: ['1'], port: 3001 },
    { id: 'umami', name: 'Umami', desc: 'Privacy-friendly analytics (needs PostgreSQL)', res: '≈ 150 MB RAM', versions: ['latest'], port: 3000 },
    { id: 'n8n', name: 'n8n', desc: 'Workflow automation', res: '≈ 250 MB RAM', versions: ['latest'], port: 5678 },
    { id: 'grafana', name: 'Grafana', desc: 'Dashboards for your own metrics', res: '≈ 200 MB RAM', versions: ['11'], port: 3000 },
  ],
};
export async function addTemplate(id, cfg) {
  await lag(700);
  const t = [...TEMPLATES.databases, ...TEMPLATES.tools].find((x) => x.id === id); if (!t) throw new Error('Unknown template');
  const name = (cfg.name || '').trim();
  if (!/^[a-z][a-z0-9-]{1,30}$/.test(name)) throw new Error('Name: lowercase letters, numbers and dashes, starting with a letter.');
  if (S.project.services.some((x) => x.id === name)) throw new Error(`A service named "${name}" already exists.`);
  const idx = S.project.services.length;
  S.project.services.push({ id: name, kind: 'db', state: 'active', repo: `${t.name} ${cfg.version || t.versions[0]} · volume ${cfg.disk || '1 GB'}`, domain: '', x: 370 * (idx % 3), y: 10 + 176 * Math.floor(idx / 3), note: cfg.backup ? 'backup daily 02:00' : 'no backup', deploys: [], vars: [] });
  audit(`add ${t.name} as ${name}`); return { id: name };
}
export async function deployCompose(text) {
  await lag(700);
  const names = [...(text || '').matchAll(/^ {2}([a-z][a-z0-9_-]*):\s*$/gm)].map((m) => m[1]);
  if (!/^services:/m.test(text || '') || !names.length) throw new Error('This does not look like a Compose file: it needs a "services:" section.');
  names.forEach((n, i) => { if (!S.project.services.some((x) => x.id === n)) S.project.services.push({ id: n, kind: 'app', state: 'active', repo: 'compose', branch: '', build: 'docker compose', domain: '', port: 0, x: 370 * ((S.project.services.length) % 3), y: 10 + 176 * Math.floor(S.project.services.length / 3), vars: [], deploys: [{ id: Math.random().toString(16).slice(2, 9), msg: 'Compose deploy', status: 'live', at: now(), secs: 20 }] }); });
  audit(`deploy compose (${names.join(', ')})`); return { names };
}

// ---------------------------------------------------------------- access
const AC = {
  keys: [
    { id: 'a1', user: 'deploy', name: 'Khang · MacBook', type: 'ed25519', fp: 'SHA256:3Fq…n8Yk', added: 'Mar 02', used: 'today 03:40' },
    { id: 'a2', user: 'deploy', name: 'CI deploy key', type: 'ed25519', fp: 'SHA256:b71…Qd2w', added: 'May 19', used: '2 min ago', note: 'restricted to git pull' },
    { id: 'a3', user: 'ubuntu', name: 'Old laptop', type: 'rsa 2048', fp: 'SHA256:xK0…7mA1', added: '2023', used: '', stale: 'unused 8 months' },
  ],
  users: [
    { name: 'ubuntu', groups: 'sudo, docker', sudo: true, last: 'Mar 02', locked: false, note: 'default cloud user' },
    { name: 'deploy', groups: 'docker', sudo: false, last: 'today 03:40', locked: false, note: 'runs your apps' },
    { name: 'root', groups: '', sudo: false, last: 'never', locked: true, note: 'direct login' },
  ],
  out: [{ id: 'o1', name: 'db-primary', sub: 'deploy@10.0.1.12' }, { id: 'o2', name: 'edge-hn-02', sub: 'ubuntu@203.0.113.40:2222' }],
};
export async function getAccess() { await lag(); return { ...clone(AC), sshOpen: S.sshOpenUntil > now() }; }
export async function addKey(user, pub) {
  await lag(300);
  if (/PRIVATE KEY/.test(pub || '')) throw new Error('That is a private key. Never paste it here. Paste the .pub file instead.');
  const m = /^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp\d+)\s+[A-Za-z0-9+/=]{20,}(\s+(.+))?$/.exec((pub || '').trim());
  if (!m) throw new Error('This is not a public key. It should start with ssh-ed25519 or ssh-rsa.');
  AC.keys.push({ id: 'a' + Date.now(), user, name: m[3] || 'unnamed key', type: m[1].replace('ssh-', ''), fp: 'SHA256:new…key', added: 'today', used: '' }); audit(`add ssh key for ${user}`);
}
export async function removeKey(id) { await lag(200); AC.keys = AC.keys.filter((k) => k.id !== id); audit('remove ssh key'); }
export async function lockUser(name, lock) {
  await lag(250); const u = AC.users.find((x) => x.name === name); if (!u) throw new Error('No such user');
  if (lock && name === 'deploy') throw new Error('deploy runs your apps. Locking it would stop deployments.');
  u.locked = lock; audit(`${lock ? 'lock' : 'unlock'} user ${name}`);
}
export async function rotateOutKey(id) { await lag(500); audit('rotate outgoing key'); }

// ---------------------------------------------------------------- settings
const ST = { hostname: 'srv-sgn-01', tz: 'Asia/Ho_Chi_Minh', panel: 'https://panel.example.com', swap: '2 GB', autoUpdate: true, channel: 'stable', github: true, fsGrown: false,
  tokens: [{ id: 't1', name: 'github-actions', can: 'deploy only, service web', last: '2 min ago' }, { id: 't2', name: 'uptime-check', can: 'read-only metrics', last: '1 min ago' }] };
export async function getSettings() { await lag(); return clone(ST); }
export async function saveSettings(v) {
  await lag(300);
  if (!/^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/i.test(v.hostname || '')) throw new Error('Hostname: letters, numbers and dashes only.');
  if (!/^https:\/\/[a-z0-9.-]+(:\d+)?$/i.test(v.panel || '')) throw new Error('Panel address must start with https:// and have no path.');
  Object.assign(ST, { hostname: v.hostname, tz: v.tz, panel: v.panel }); S.host.name = v.hostname; audit('update general settings');
}
export async function createToken(name, can) {
  await lag(300); if (!/^[a-z][a-z0-9-]{2,}$/.test(name || '')) throw new Error('Token name: at least 3 characters, lowercase, dashes allowed.');
  const secret = 'ops_' + Math.random().toString(36).slice(2) + Math.random().toString(36).slice(2);
  ST.tokens.push({ id: 't' + Date.now(), name, can, last: 'never' }); audit(`create api token ${name}`); return { secret };
}
export async function revokeToken(id) { await lag(200); ST.tokens = ST.tokens.filter((t) => t.id !== id); audit('revoke api token'); }
export async function setSwap(v) { await lag(400); ST.swap = v; audit(`swap ${v}`); }
export async function growFs() { await lag(900); ST.fsGrown = true; audit('grow filesystem'); }
export async function setOpsboxAutoUpdate(on) { await lag(150); ST.autoUpdate = on; }
export async function disconnectGithub() { await lag(300); ST.github = false; audit('disconnect GitHub'); }
export async function exportConfig() { await lag(400); audit('export configuration'); return JSON.stringify({ opsbox: '0.1.0', host: S.host.name, services: S.project.services.map((s) => ({ id: s.id, kind: s.kind, repo: s.repo, branch: s.branch, build: s.build, domain: s.domain })), settings: { tz: ST.tz, panel: ST.panel } }, null, 2); }
