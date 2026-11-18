// Shared API boundary: real Phase 1 endpoints in HTTP mode, sample data for later
// phases and the standalone demo. REAL below is the capability switch; docs/API.md
// documents the server contract.

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lag = (ms = 120) => sleep(ms + Math.random() * 120);
const now = () => Date.now();
const clone = (o) => JSON.parse(JSON.stringify(o));
const hhmm = (t = now()) => new Date(t).toTimeString().slice(0, 5);
const hhmmss = (t = now()) => new Date(t).toTimeString().slice(0, 8);
const MIN = 60000, HOUR = 3600000, DAY = 86400000;

// How this module gets its data.
//   'mock'  in-memory demo data (default; works from any static file server).
//   'http'  the opsbox agent on the same origin; functions in REAL use the API.
// To try http mode without editing this file, open the UI with ?mode=http
// (and ?mode=mock to go back).
const DEFAULT_MODE = 'mock';
export const MODE = pickMode();

function pickMode() {
  try {
    const wanted = new URLSearchParams(location.search).get('mode');
    if (wanted === 'http' || wanted === 'mock') return wanted;
    if (location.protocol === 'https:' || location.port === '8443') return 'http';
  } catch { /* no location (tests, workers): use the default */ }
  return DEFAULT_MODE;
}

// ---------------------------------------------------------------- real vs mock
// The single place that decides which functions use the real API in MODE === 'http'.
// To wire a function: implement its `if (real('name')) return http(...)` branch at the
// top of the function, then add its name here. Pages that still call functions not in
// this set get a "sample data" badge (main.js); the badge disappears by itself.
const REAL = new Set([
  'getProxyNetwork', 'takeProxyPort', 'undoProxyTakeover', 'keepProxyPorts',
  'getHost',
  'getAlerts', 'setRule', 'testAlert', 'connectChannel', 'disconnectChannel', 'silenceAlert', 'setQuiet',
  'auth.session', 'auth.login', 'auth.logout', 'auth.claim', 'auth.setupAdmin', 'auth.setupConfirm',
  'auth.enroll', 'auth.enrollConfirm', 'signOutAll',
  'getMetrics', 'getAttention',
  'getUpdates', 'installUpdates', 'setAutoUpdates', 'reboot', 'cancelReboot', 'getUnits', 'unitAction', 'getJournal',
  'getJobs', 'runJob', 'addJob', 'removeJob', 'getFirewall', 'addRule', 'removeRule', 'confirmFirewall', 'openSsh',
  'listDir', 'readFile', 'writeFile', 'chmod', 'mkdir', 'removePaths', 'upload',
  'getTerminalUsers', 'openTerminal',
  'scanCleanup', 'runCleanup', 'setAutoCleanup', 'getCleanupVolumes',
  'getAccess', 'addKey', 'removeKey', 'lockUser',
  'getSecurity', 'toggleCheck', 'setAllowIps', 'inviteUser', 'removeUser', 'auth.inviteInfo', 'auth.claimInvite',
  'dockerStatus', 'installDocker', 'listContainers', 'listDocker', 'inspectContainer', 'containerAction', 'containerLogs',
  'getSettings', 'saveSettings', 'createToken', 'revokeToken', 'setSwap',
  'checkPanelDns', 'issuePanelCert', 'removePanelDomain', 'setPanelLocalOnly',
  'checkOpsboxUpdate', 'applyOpsboxUpdate', 'rollbackOpsboxUpdate', 'setOpsboxAutoUpdate',
  'getRecovery',
  'getGithubStatus', 'startGithubManifest', 'completeGithubManifest', 'syncGithubInstallation', 'syncGithubWebhook',
  'disconnectGithub', 'getRepos', 'inspectPublicRepo', 'listDeployKeys', 'createDeployKey', 'deleteDeployKey',
  'getProject', 'createService', 'updateService', 'redeploy', 'rollback', 'buildLog', 'addVar', 'removeVar', 'removeService',
  'connectDatabase', 'disconnectDatabase', 'rotateDatabase', 'setServiceExposure',
  'getDatabase', 'rotateAppLogin', 'removeAppLogin', 'setAppPooled', 'setExtension', 'setPooler', 'upgradeDatabase', 'revealConnectionString',
  'getDomains', 'addDomain', 'checkDns', 'issueDomainCert', 'updateDomain', 'removeDomain', 'makePrimary',
  'getTemplates', 'addTemplate', 'deployCompose',
  'getBackups', 'runBackup', 'createBackupJob', 'updateBackupJob', 'deleteBackupJob', 'pruneBackups', 'checkBackups',
  'getProviders', 'testDestination', 'saveDestination', 'testSavedDestination', 'deleteDestination', 'setDefaultDestination',
  'downloadRecoveryKey', 'restoreBackup', 'testBackupRestore',
]);
const real = (name) => MODE === 'http' && REAL.has(name);
export const isReal = (name) => real(name);

// Which functions did the page that is being drawn use? (Cleared on every navigation.)
const used = { sample: new Set(), real: new Set() };
let untracked = false;
// Called first thing in every mock function. A no-op outside http mode.
function sample(name) { if (MODE === 'http' && !untracked && !REAL.has(name)) used.sample.add(name); }
// What each page shows from this module (including data it only fetches on a click,
// and static sample data). Generated from the pages' `api.*` references; the "sample
// data" badge is the entries here that are not in REAL, plus anything used at run time.
export const PAGE_USES = {
  // (Keys opsbox uses to reach other servers arrive with the desktop app; the section is hidden until then.)
  access: ['addKey', 'getAccess', 'lockUser', 'openSsh', 'removeKey'],
  alerts: ['disconnectChannel', 'connectChannel', 'getAlerts', 'setQuiet', 'setRule', 'silenceAlert', 'testAlert'],
  databases: ['addTemplate', 'getDatabase', 'getProject', 'removeAppLogin', 'revealConnectionString', 'rotateAppLogin', 'setAppPooled', 'setExtension', 'setPooler', 'upgradeDatabase'],
  apps: ['addDomain', 'addVar', 'buildLog', 'checkDns', 'connectDatabase', 'disconnectDatabase', 'getDomains', 'getProject', 'issueDomainCert', 'makePrimary', 'redeploy', 'removeDomain', 'removeService', 'removeVar', 'rollback', 'rotateDatabase', 'setServiceExposure', 'updateDomain', 'updateService'],
  backupdest: ['getProviders', 'saveDestination', 'testDestination', 'downloadRecoveryKey'],
  backups: ['getBackups', 'runBackup', 'createBackupJob', 'updateBackupJob', 'deleteBackupJob', 'pruneBackups', 'checkBackups', 'restoreBackup', 'testSavedDestination', 'deleteDestination', 'setDefaultDestination', 'downloadRecoveryKey', 'testBackupRestore'],
  cleanup: ['runCleanup', 'scanCleanup', 'setAutoCleanup'],
  configure: ['createService'],
  containers: ['containerAction', 'containerLogs', 'dockerStatus', 'inspectContainer', 'installDocker', 'listContainers', 'listDocker'],
  files: ['chmod', 'listDir', 'mkdir', 'readFile', 'removePaths', 'upload', 'writeFile'],
  login: ['auth.login'],
  invite: ['auth.inviteInfo', 'auth.claimInvite'],
  enroll: ['auth.enroll', 'auth.enrollConfirm'],
  newservice: ['createDeployKey', 'deleteDeployKey', 'getGithubStatus', 'getRepos', 'inspectPublicRepo', 'listDeployKeys'],
  overview: ['getProxyNetwork', 'getAttention', 'getHost', 'getMetrics'],
  recovery: MODE === 'http' ? ['getRecovery'] : ['getRecovery', 'recoveryAction', 'simulateFailure'],
  security: ['getSecurity', 'inviteUser', 'openSsh', 'removeUser', 'setAllowIps', 'signOutAll', 'toggleCheck'],
  settings: ['getProxyNetwork', 'takeProxyPort', 'undoProxyTakeover', 'keepProxyPorts', 'checkPanelDns', 'completeGithubManifest', 'createToken', 'deleteDeployKey', 'disconnectGithub', 'exportConfig', 'getGithubStatus', 'getSettings', 'growFs', 'issuePanelCert', 'listDeployKeys', 'removePanelDomain', 'revokeToken', 'saveSettings', 'setOpsboxAutoUpdate', 'setPanelLocalOnly', 'setSwap', 'startGithubManifest', 'syncGithubInstallation', 'syncGithubWebhook'],
  setup: ['auth.claim', 'auth.setupAdmin', 'auth.setupConfirm'],
  system: ['addJob', 'cancelReboot', 'confirmFirewall', 'removeJob', 'addRule', 'getFirewall', 'getJobs', 'getJournal', 'getUnits', 'getUpdates', 'installUpdates', 'openSsh', 'reboot', 'removeRule', 'runJob', 'setAutoUpdates', 'unitAction'],
  templates: ['addTemplate', 'deployCompose', 'getProject', 'getTemplates'],
  // In http mode the page shows only this server (the demo hosts and shell are mock-only).
  terminal: ['getTerminalUsers', 'openTerminal'],
};

export function resetUsage() { used.sample.clear(); used.real.clear(); }
// Sample and real sources behind page `page`, for the badge.
export function usageFor(page) {
  const names = new Set([...(PAGE_USES[page] || []), ...used.sample, ...used.real]);
  const all = [...names].sort();
  return { sample: all.filter((n) => !REAL.has(n)), real: all.filter((n) => REAL.has(n)) };
}
// Run `fn` without attributing its API calls to the page (status bar, polling).
export function quiet(fn) { const was = untracked; untracked = true; try { return fn(); } finally { untracked = was; } }

function cookie(name) {
  try { return document.cookie.split('; ').find((c) => c.startsWith(name + '='))?.slice(name.length + 1) || ''; } catch { return ''; }
}

// One fetch helper for every real endpoint. Errors carry the server's own
// sentence (docs/BACKEND_SPEC.md 6.1), which the UI shows verbatim.
async function http(path, { method = 'GET', body } = {}) {
  let res;
  try {
    // Double-submit CSRF: the page echoes the (script-readable) cookie in a header.
    const csrf = method === 'GET' ? '' : cookie('opsbox_csrf');
    res = await fetch('/api/v1' + path, {
      method,
      credentials: 'same-origin',
      headers: {
        Accept: 'application/json',
        ...(body === undefined ? {} : { 'Content-Type': 'application/json' }),
        ...(csrf ? { 'X-Opsbox-CSRF': csrf } : {}),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new Error('Cannot reach the opsbox server. Check that it is running.');
  }
  let data = null;
  try { data = await res.json(); } catch { /* empty or non-JSON body */ }
  if (!res.ok) {
    const err = new Error(data?.error?.message || `The server answered with an error (HTTP ${res.status}).`);
    err.code = data?.error?.code; err.field = data?.error?.field; err.status = res.status;
    // The session ended (expired, signed out elsewhere): let the app send the person to sign in.
    if (res.status === 401 && !path.startsWith('/auth/login') && !path.startsWith('/session')) {
      try { dispatchEvent(new Event('opsbox:unauthorized')); } catch { /* no window */ }
    }
    throw err;
  }
  return data;
}


// ---- helpers for the System functions
// The page registers these so the API layer can ask a person things: confirm a risky firewall
// change, enter an address, keep a change that will otherwise be undone.
const hooks = { confirm: null, prompt: null, revert: null };
export function setUiHooks(h) { Object.assign(hooks, h); }

// A background task on the server (installs, swap): poll until it ends.
async function waitTask(id, { every = 1500, max = 45 * 60 * 1000 } = {}) {
  const t0 = Date.now();
  for (;;) {
    const t = await http('/tasks/' + id);
    if (t.status === 'ok') return t;
    if (t.status === 'failed') throw new Error(t.error || 'The operation failed.');
    if (Date.now() - t0 > max) throw new Error('This is taking very long. Check the System page again later.');
    await sleep(every);
  }
}

// "today 02:00", "Sun 03:00", "2026-11-01 02:00", "never"
function when(sec) {
  if (!sec) return 'never';
  const d = new Date(sec * 1000), t = new Date(), hm = hhmm(d.getTime());
  if (d.toDateString() === t.toDateString()) return `today ${hm}`;
  if (t - d < 6 * DAY) return `${d.toLocaleDateString('en', { weekday: 'short' })} ${hm}`;
  return `${d.toISOString().slice(0, 10)} ${hm}`;
}

// A firewall change the server thinks could lock you out comes back as "confirmation
// required": ask, then repeat it with confirm. If it is applied provisionally, the server
// undoes it after 60 s unless the person keeps it.
async function changeFirewall(path, opts, confirmed = false) {
  const withConfirm = !confirmed ? opts : opts.method === 'DELETE' ? opts : { ...opts, body: { ...opts.body, confirm: true } };
  try {
    const r = await http(confirmed && opts.method === 'DELETE' ? path + '?confirm=true' : path, withConfirm);
    if (r.revertAt && hooks.revert) hooks.revert(r.revertAt, () => confirmFirewall());
    return r;
  } catch (e) {
    if (e.code === 'confirmation_required' && !confirmed && hooks.confirm && await hooks.confirm(e.message)) return changeFirewall(path, opts, true);
    throw e;
  }
}

async function openSshReal(ip) {
  try {
    const r = await http('/ssh/emergency', { method: 'POST', body: ip ? { ip } : undefined });
    return { until: r.until };
  } catch (e) {
    // Through an SSH tunnel the server only sees its own address: ask for the real one.
    if (e.field === 'ip' && hooks.prompt) {
      const v = await hooks.prompt({ title: 'Which address should be allowed?', body: e.message, label: 'Your public IP address', placeholder: '203.0.113.5' });
      if (v) return openSshReal(v);
      throw new Error('Emergency SSH was not opened.');
    }
    throw e;
  }
}

// Live journal lines (http mode). `onEntries` gets arrays of {t, proc, prio, msg}; returns a stop function.
export function streamJournal({ unit = '', prio = 6, q = '' } = {}, onEntries) {
  if (MODE !== 'http' || !REAL.has('getJournal')) return () => {};
  const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/v1/stream/journal?${new URLSearchParams({ unit, prio, q })}`);
  ws.onmessage = (e) => { try { const m = JSON.parse(e.data); if (m.type === 'journal') onEntries(m.entries); } catch { /* ignore */ } };
  return () => { try { ws.close(); } catch { /* closed */ } };
}

export async function cancelReboot() { sample('cancelReboot'); if (real('cancelReboot')) return http('/system/reboot/cancel', { method: 'POST' }); await lag(150); return { ok: true }; }
export async function confirmFirewall() { sample('confirmFirewall'); if (real('confirmFirewall')) return http('/firewall/confirm', { method: 'POST' }); await lag(100); return { ok: true }; }
export async function removeJob(id) { sample('removeJob'); if (real('removeJob')) return http('/jobs/' + encodeURIComponent(id), { method: 'DELETE' }); await lag(200); const i = S.jobs.findIndex((x) => x.id === id); if (i >= 0) S.jobs.splice(i, 1); audit('remove job'); return { ok: true }; }


// ---- helpers for the Files functions
const enc = encodeURIComponent;
// "12:30" for today, "Mon 12" this week, else "2026-11-01"
function fileTime(sec) {
  if (!sec) return '';
  const d = new Date(sec * 1000), t = new Date();
  if (d.toDateString() === t.toDateString()) return hhmm(d.getTime());
  return d.toISOString().slice(0, 10);
}
// The editor does not pass the modification time it saw, so it is remembered here: saving
// sends it back, and the server refuses to overwrite a file someone else saved meanwhile.
const seenMtime = new Map();
// Where the browser downloads a file from (a normal link; the session cookie authorizes it).
export const downloadUrl = (path) => (MODE === 'http' && REAL.has('readFile') ? `/api/v1/files/download?path=${enc(path)}` : null);

async function uploadReal(dir, file, overwrite = false) {
  let res;
  try {
    res = await fetch(`/api/v1/files/upload?dir=${enc(dir)}&name=${enc(file.name)}${overwrite ? '&overwrite=true' : ''}`, {
      method: 'POST', credentials: 'same-origin',
      headers: { Accept: 'application/json', 'Content-Type': 'application/octet-stream', 'X-Opsbox-CSRF': cookie('opsbox_csrf') },
      body: file, // streamed from disk by the browser
    });
  } catch { throw new Error('The upload was interrupted. Check the connection and try again.'); }
  let data = null; try { data = await res.json(); } catch { /* none */ }
  if (res.ok) return data;
  const err = new Error(data?.error?.message || `The upload failed (HTTP ${res.status}).`);
  err.code = data?.error?.code;
  if (res.status === 409 && !overwrite && hooks.confirm && await hooks.confirm(`A file named ${file.name} already exists here. Replace it?`)) return uploadReal(dir, file, true);
  throw err;
}

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
    edges: [['web', 'postgres'], ['web', 'redis'], ['api', 'postgres'], ['worker', 'redis']],
    services: [
      { id: 'web', kind: 'app', state: 'active', repo: 'acme/shop', branch: 'main', build: 'Dockerfile', domain: 'shop.example.com', port: 3000, x: 0, y: 98,
        vars: [['NODE_ENV', 'production', false], ['DATABASE_URL', '${{ postgres.DATABASE_URL }}', true, true], ['REDIS_URL', '${{ redis.REDIS_URL }}', false, true]],
        deploys: [
          { id: 'a3f9c21', msg: 'Fix cart when out of stock', status: 'live', at: now() - 2 * MIN, secs: 48 },
          { id: '7be1d04', msg: 'Add checkout page', status: 'replaced', at: now() - DAY, secs: 51 },
          { id: '1c80ae7', msg: 'Update dependencies', status: 'failed', at: now() - 3 * DAY, secs: 33 },
        ] },
      { id: 'api', kind: 'app', state: 'building', repo: 'acme/shop-api', branch: 'main', build: 'Nixpacks', domain: 'api.shop.example.com', port: 4000, x: 370, y: 10, vars: [['PORT', '4000', false], ['DATABASE_URL', '${{ postgres.DATABASE_URL }}', true, true]], deploys: [{ id: '5d2e8b0', msg: 'Add orders endpoint', status: 'building', at: now() - 40000, secs: 0 }] },
      { id: 'worker', kind: 'app', state: 'active', repo: 'acme/shop-api', branch: 'main', build: 'npm run worker', domain: '', port: 0, x: 370, y: 186, vars: [['REDIS_URL', '${{ redis.REDIS_URL }}', false, true]], deploys: [{ id: '5c1a7f4', msg: 'Retry failed jobs', status: 'live', at: now() - 2 * DAY, secs: 40 }] },
      { id: 'postgres', kind: 'db', state: 'active', repo: 'PostgreSQL 16 · volume 4.2 GB', templateId: 'postgres', port: 5432, internalOnly: true, publicPort: false, networkMode: 'internal', domain: '', x: 740, y: 10, note: 'backup daily 02:00', connectedApps: [{ id: 'web', name: 'web', kind: 'app', state: 'active', vars: ['DATABASE_URL'], needsRedeploy: false }, { id: 'api', name: 'api', kind: 'app', state: 'building', vars: ['DATABASE_URL'], needsRedeploy: false }], deploys: [], vars: [['POSTGRES_USER', 'app', false], ['POSTGRES_PASSWORD', '••••••••', true], ['DATABASE_URL', '••••••••', true]] },
      { id: 'redis', kind: 'db', state: 'active', repo: 'Redis 7 · 24 MB', templateId: 'redis', port: 6379, internalOnly: true, publicPort: false, networkMode: 'internal', domain: '', x: 740, y: 186, note: 'internal only', connectedApps: [{ id: 'web', name: 'web', kind: 'app', state: 'active', vars: ['REDIS_URL'], needsRedeploy: false }, { id: 'worker', name: 'worker', kind: 'app', state: 'active', vars: ['REDIS_URL'], needsRedeploy: false }], deploys: [], vars: [['REDIS_URL', '••••••••', true]] },
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
  async setupStatus() {
    if (MODE === 'http') return http('/setup/status');
    return { setupRequired: false };
  },
  // GET /api/v1/session -> { authenticated, enrollRequired, setupRequired, user?, role? }
  async session() {
    if (real('auth.session')) return http('/session');
    sample('auth.session'); return { authenticated: !!sessionStorage.getItem('opsbox.user'), enrollRequired: false, setupRequired: false, user: sessionStorage.getItem('opsbox.user') };
  },
  // POST /api/v1/auth/login -> { user, role, enrollRequired }
  async login(user, pass, code) {
    if (real('auth.login')) return http('/auth/login', { method: 'POST', body: { username: user, password: pass, code: (code || '').trim() } });
    sample('auth.login'); await lag(300);
    if (!user || !pass) throw new Error('Enter a username and password.');
    if (!/^\d{6}$/.test(code || '')) throw new Error('Enter the 6-digit authentication code.');
    audit('sign-in', user); return { user, role: 'admin', enrollRequired: false };
  },
  // GET /api/v1/invites/{token} -> { name, role }
  async inviteInfo(token) {
    if (real('auth.inviteInfo')) return http('/invites/' + encodeURIComponent(token));
    sample('auth.inviteInfo'); await lag(150); return { name: 'dana', role: 'deployer' };
  },
  // POST /api/v1/invites/claim -> { user, role, enrollRequired: true } (a short enrolment session)
  async claimInvite(token, password) {
    if (real('auth.claimInvite')) return http('/invites/claim', { method: 'POST', body: { token, password } });
    sample('auth.claimInvite'); await lag(300);
    if ((password || '').length < 12) throw new Error('Use at least 12 characters for the password.');
    return { user: 'dana', role: 'deployer', enrollRequired: true };
  },
  // POST /api/v1/auth/logout
  async logout() {
    if (real('auth.logout')) return http('/auth/logout', { method: 'POST' });
    sample('auth.logout'); return { ok: true };
  },
  // POST /api/v1/setup/claim
  async claim(token) {
    if (real('auth.claim')) return http('/setup/claim', { method: 'POST', body: { token: (token || '').trim() } });
    sample('auth.claim'); await lag(300);
    if (!/^[A-Z0-9]{4}(-[A-Z0-9]{4}){2}$/i.test((token || '').trim())) throw new Error('That token does not look right. Format: XXXX-XXXX-XXXX');
    return { ok: true };
  },
  // POST /api/v1/setup/admin -> { secret, otpauthUri, account }
  async setupAdmin(username, password) {
    if (real('auth.setupAdmin')) return http('/setup/admin', { method: 'POST', body: { username, password } });
    sample('auth.setupAdmin'); await lag(300);
    return { secret: 'JBSWY3DPEHPK3PXP', otpauthUri: `otpauth://totp/opsbox:${username}?secret=JBSWY3DPEHPK3PXP`, account: username };
  },
  // POST /api/v1/setup/totp/confirm -> signed in
  async setupConfirm(code) {
    if (real('auth.setupConfirm')) return http('/setup/totp/confirm', { method: 'POST', body: { code: (code || '').trim() } });
    sample('auth.setupConfirm'); await lag(300);
    if (!/^\d{6}$/.test(code || '')) throw new Error('Enter the 6-digit code from your authenticator app.');
    return { user: 'admin', role: 'admin' };
  },
  // POST /api/v1/auth/totp/enroll -> { secret, otpauthUri, account }   (enrolment sessions only)
  async enroll() {
    if (real('auth.enroll')) return http('/auth/totp/enroll', { method: 'POST' });
    sample('auth.enroll'); await lag(200);
    return { secret: 'JBSWY3DPEHPK3PXP', otpauthUri: 'otpauth://totp/opsbox:admin?secret=JBSWY3DPEHPK3PXP', account: 'admin' };
  },
  // POST /api/v1/auth/totp/confirm -> full session
  async enrollConfirm(code) {
    if (real('auth.enrollConfirm')) return http('/auth/totp/confirm', { method: 'POST', body: { code: (code || '').trim() } });
    sample('auth.enrollConfirm'); await lag(300);
    if (!/^\d{6}$/.test(code || '')) throw new Error('Enter the 6-digit code from your authenticator app.');
    return { user: 'admin', role: 'admin' };
  },
};

// ---------------------------------------------------------------- metrics
// http mode: the server pushes one frame a second over a WebSocket and `onFrame` is called
// for each. Reconnects with a growing delay; if the reason is an ended session, tells the
// app to go to the sign-in page instead of retrying forever. Returns a function that stops it.
export function startMetricsStream(onFrame) {
  if (MODE !== 'http' || !REAL.has('getMetrics')) return () => {};
  let ws, stopped = false, delay = 1000, timer;
  const open = () => {
    ws = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/v1/stream/metrics`);
    ws.onmessage = (e) => {
      let m; try { m = JSON.parse(e.data); } catch { return; }
      if (m.type !== 'metrics') return;
      streamFrame = m.data; streamAt = Date.now(); delay = 1000; onFrame(m.data);
    };
    ws.onclose = async () => {
      if (stopped) return;
      streamFrame = null;
      try { const s = await fetch('/api/v1/session').then((r) => r.json()); if (s && s.authenticated === false) { dispatchEvent(new Event('opsbox:unauthorized')); return; } } catch { /* server down: keep trying */ }
      timer = setTimeout(open, delay); delay = Math.min(delay * 2, 15000);
    };
    ws.onerror = () => { try { ws.close(); } catch { /* already closed */ } };
  };
  open();
  return () => { stopped = true; clearTimeout(timer); streamFrame = null; try { ws.close(); } catch { /* not open */ } };
}

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
export async function getHost() {
  if (real('getHost')) { if (!untracked) used.real.add('getHost'); return http('/host'); } // GET /api/v1/host
  sample('getHost'); await lag(30); return clone(S.host);
}
// Latest frame pushed by the metrics stream (http mode); see startMetricsStream().
let streamFrame = null, streamAt = 0;
const FRESH_MS = 3000;

export async function getMetrics() {
  // GET /api/v1/metrics?window=48s, or the newest frame from WS /api/v1/stream/metrics
  if (real('getMetrics')) return streamFrame && Date.now() - streamAt < FRESH_MS ? streamFrame : http('/metrics?window=48s');
 sample('getMetrics');
  const m = S.metrics;
  return { cpu: [...m.cpu], mem: [...m.mem], cpuNow: m.cpu[m.cpu.length - 1], memUsed: m.memUsed, memTotal: S.host.memTotal, cache: m.cache,
    disk: m.disk, diskTotal: S.host.diskTotal, netIn: m.netIn, netOut: m.netOut, conns: m.conns, load: [...m.load], procs: clone(S.procs).sort((a, b) => b.cpu - a.cpu) };
}
export async function getAttention() {
  if (real('getAttention')) return http('/attention'); // GET /api/v1/attention
  sample('getAttention');
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
export async function dockerStatus() { sample('dockerStatus');
  if (real('dockerStatus')) return http('/docker/status');
  await lag(40);
  return { installed: true, running: true, version: '27.3.1', apiVersion: '1.47', canInstall: false, installMethod: null, error: null };
}
export async function installDocker() { sample('installDocker');
  if (real('installDocker')) {
    const { task } = await http('/docker/install', { method: 'POST' });
    return waitTask(task, { every: 2000, max: 25 * 60 * 1000 });
  }
  await lag(600); audit('install docker'); return { status: 'ok', result: { ok: true } };
}
export async function listContainers() { sample('listContainers');
  if (real('listContainers')) return http('/docker/containers');
  await lag(); return clone(S.containers);
}
export async function listDocker(kind) { sample('listDocker');
  if (real('listDocker')) return http(`/docker/${encodeURIComponent(kind)}`);
  await lag(); return clone(S[kind]);
}
export async function inspectContainer(id) { sample('inspectContainer');
  if (real('inspectContainer')) return http(`/docker/containers/${encodeURIComponent(id)}`);
  await lag();
  const c = S.containers.find((x) => x.id === id || x.name === id); if (!c) throw new Error('No such container');
  return { id: c.id, fullId: c.id, name: c.name, image: c.image, created: new Date(c.since).toISOString(), state: c.state, running: c.state === 'running', restarting: !!c.restarts, paused: false, oomKilled: false, exitCode: 0, startedAt: new Date(c.since).toISOString(), finishedAt: '', restartCount: c.restarts || 0, pid: c.state === 'running' ? 1024 : null, cmd: ['node', 'server.js'], entrypoint: [], workingDir: '/app', user: 'node', ports: [], mounts: [], networks: [], env: [{ key: 'NODE_ENV', value: 'production', masked: false }], labels: {}, managed: true, service: c.name, deploy: null };
}
export async function containerAction(id, action) { sample('containerAction');
  if (real('containerAction')) return http(`/docker/containers/${encodeURIComponent(id)}/${encodeURIComponent(action)}`, { method: 'POST' });
  await lag(350);
  const c = S.containers.find((x) => x.id === id); if (!c) throw new Error('No such container');
  if (action === 'restart') { c.state = 'running'; c.since = now(); }
  if (action === 'start') { c.state = 'running'; c.since = now(); c.mem = 30; c.cpu = 0.5; }
  if (action === 'stop') { c.state = 'exited'; c.cpu = 0; c.mem = 0; }
  if (action === 'delete') S.containers = S.containers.filter((x) => x.id !== id);
  audit(`${action} ${c.name}`); return { ok: true };
}
export async function containerLogs(id, tail = 200) { sample('containerLogs');
  if (real('containerLogs')) return http(`/docker/containers/${encodeURIComponent(id)}/logs?tail=${encodeURIComponent(tail)}`);
  await lag(80);
  const c = S.containers.find((x) => x.id === id); if (!c) return [];
  const t = now();
  return Array.from({ length: 9 }, (_, i) => ({ ts: new Date(t - (9 - i) * 7000).toISOString(), stream: i % 4 === 2 ? 'stderr' : 'stdout', text: LOGS[(i * 5 + id.length) % LOGS.length] }));
}
export function streamContainerLogs(id, { tail = 200 } = {}, onEntries, onEnd, onError) {
  if (MODE !== 'http' || !REAL.has('containerLogs')) return () => {};
  const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/v1/stream/containers/${encodeURIComponent(id)}/logs?${new URLSearchParams({ tail })}`);
  ws.onmessage = (e) => {
    try {
      const m = JSON.parse(e.data);
      if (m.type === 'logs') onEntries?.(m.entries || []);
      else if (m.type === 'end') onEnd?.();
      else if (m.type === 'error') onError?.(m.message || 'Log stream error');
    } catch { /* ignore */ }
  };
  ws.onerror = () => onError?.('Log stream connection failed.');
  return () => { try { ws.close(); } catch { /* closed */ } };
}

// ---------------------------------------------------------------- apps & github
export async function getProject(id = 'default') {
  sample('getProject');
  if (real('getProject')) return http(`/projects/${encodeURIComponent(id)}`);
  await lag();
  return clone(S.project);
}
export async function getGithubStatus(refresh = false) {
  sample('getGithubStatus');
  if (real('getGithubStatus')) return http('/github' + (refresh ? '?refresh=1' : ''));
  await lag(60);
  return clone(ST.github);
}
export async function startGithubManifest(org = '') {
  sample('startGithubManifest');
  if (real('startGithubManifest')) return http('/github/manifest', { method: 'POST', body: org ? { org } : {} });
  await lag(180);
  const owner = (org || '').trim() || 'acme-co';
  ST.github = { connected: true, installed: true, appSlug: 'opsbox-demo', appName: 'opsbox demo', owner, account: owner, manageUrl: 'https://github.com/settings/installations', installUrl: 'https://github.com/apps/opsbox-demo/installations/new', repositorySelection: 'selected', webhookUrl: 'https://panel.example.com/api/v1/github/webhook', webhookActive: true, webhookConfigured: true, panelDomain: ST.panel || null, panelDomainSet: !!ST.panel };
  audit(`connect GitHub (${owner})`);
  return { demoConnected: true };
}
export async function completeGithubManifest(code, state) {
  sample('completeGithubManifest');
  if (real('completeGithubManifest')) return http('/github/complete', { method: 'POST', body: { code, state } });
  await lag(180);
  return clone(ST.github);
}
export async function syncGithubInstallation(installationId = null) {
  sample('syncGithubInstallation');
  if (real('syncGithubInstallation')) return http('/github/installation', { method: 'POST', body: installationId ? { installationId: Number(installationId) } : {} });
  await lag(150);
  return clone(ST.github);
}
export async function syncGithubWebhook() {
  sample('syncGithubWebhook');
  if (real('syncGithubWebhook')) return http('/github/webhook/sync', { method: 'POST' });
  await lag(150);
  ST.github.webhookConfigured = true; ST.github.webhookActive = true;
  return clone(ST.github);
}
export async function getRepos(q = '', { refresh = false } = {}) {
  sample('getRepos');
  if (real('getRepos')) {
    const p = new URLSearchParams();
    if (q) p.set('q', q);
    if (refresh) p.set('refresh', '1');
    const qs = p.toString();
    return http('/github/repos' + (qs ? '?' + qs : ''));
  }
  await lag(250);
  const term = (q || '').trim().toLowerCase();
  const repos = clone(S.repos).filter((r) => !term || r.name.toLowerCase().includes(term) || (r.lang || '').toLowerCase().includes(term) || (r.build || '').toLowerCase().includes(term));
  return { connected: !!ST.github?.connected, installed: !!ST.github?.installed, account: ST.github?.account || null, installUrl: ST.github?.installUrl || null, manageUrl: ST.github?.manageUrl || null, repos };
}
export async function inspectPublicRepo(url) {
  sample('inspectPublicRepo');
  if (real('inspectPublicRepo')) return http('/github/public-repo', { method: 'POST', body: { url } });
  await lag(250);
  const m = /^https:\/\/github\.com\/([A-Za-z0-9_.-]+)\/([A-Za-z0-9_.-]+?)(?:\.git)?\/?$/.exec((url || '').trim());
  if (!m) throw new Error('Enter a public GitHub URL like https://github.com/owner/repo');
  return { repo: `${m[1]}/${m[2]}`, url: `https://github.com/${m[1]}/${m[2]}`, provider: 'github', vis: 'public', lang: 'TypeScript', defaultBranch: 'main', build: 'Dockerfile', detected: true };
}
export async function listDeployKeys() {
  sample('listDeployKeys');
  if (real('listDeployKeys')) return http('/github/deploy-keys');
  await lag(60);
  return clone(ST.deployKeys || []);
}
export async function createDeployKey(repo) {
  sample('createDeployKey');
  if (real('createDeployKey')) return http('/github/deploy-keys', { method: 'POST', body: { repo } });
  await lag(200);
  const clean = (repo || '').trim().replace(/^https:\/\/github\.com\//, '').replace(/^git@github\.com:/, '').replace(/\.git$/, '');
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(clean)) throw new Error('Enter a repository like owner/repo');
  const k = { id: 'dk' + Date.now(), repo: clean, sshUrl: `git@github.com:${clean}.git`, publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoDeployKeyPublicMaterial opsbox-deploy-demo', fingerprint: 'SHA256:demoKeyFingerprint12345', createdBy: 'admin', createdAt: Math.floor(Date.now() / 1000) };
  ST.deployKeys.unshift(k);
  audit(`create deploy key for ${clean}`);
  return clone(k);
}
export async function deleteDeployKey(id) {
  sample('deleteDeployKey');
  if (real('deleteDeployKey')) return http('/github/deploy-keys/' + encodeURIComponent(id), { method: 'DELETE' });
  await lag(150);
  ST.deployKeys = ST.deployKeys.filter((k) => k.id !== id);
  return { ok: true };
}
export async function createService(cfg) {
  sample('createService');
  if (real('createService')) return http('/services', { method: 'POST', body: cfg });
  await lag(500);
  const name = (cfg.name || '').trim();
  if (!/^[a-z][a-z0-9-]{1,30}$/.test(name)) throw new Error('Service name: lowercase letters, numbers and dashes, starting with a letter.');
  if (S.project.services.some((s) => s.id === name)) throw new Error(`A service named "${name}" already exists.`);
  const id = Math.random().toString(16).slice(2, 9);
  const idx = S.project.services.length;
  const dom = (cfg.domain || '').trim();
  const domain = dom ? (dom.includes('.') ? dom : `${dom}.example.com`) : '';
  S.project.services.push({ id: name, kind: 'app', state: 'building', sourceKind: cfg.sourceKind || 'github', repo: cfg.repo || cfg.image || '', branch: cfg.branch || 'main', build: cfg.build || 'Dockerfile', domain, port: +cfg.port || 0,
    x: 370 * (idx % 3), y: 10 + 176 * Math.floor(idx / 3), vars: cfg.vars || [], deploys: [{ id, msg: 'Initial deploy', status: 'building', at: now(), secs: 0 }] });
  setTimeout(() => { const s = S.project.services.find((x) => x.id === name); if (s) { s.state = 'active'; s.deploys[0].status = 'live'; s.deploys[0].secs = 52; } }, 6000);
  audit(`deploy ${name} ${id}`); return { id: name, deployId: id };
}
export async function updateService(svc, patch) {
  sample('updateService');
  if (real('updateService')) return http(`/services/${encodeURIComponent(svc)}`, { method: 'PATCH', body: patch });
  await lag(200);
  const s = S.project.services.find((x) => x.id === svc); if (!s) throw new Error('No such service');
  Object.assign(s, patch);
  return clone(s);
}
export async function redeploy(svc) {
  sample('redeploy');
  if (real('redeploy')) return http(`/services/${encodeURIComponent(svc)}/deploys`, { method: 'POST' });
  await lag(300);
  const s = S.project.services.find((x) => x.id === svc); if (!s) throw new Error('No such service');
  const id = Math.random().toString(16).slice(2, 9);
  s.deploys.forEach((d) => { if (d.status === 'live') d.status = 'replaced'; });
  s.deploys.unshift({ id, msg: 'Manual redeploy', status: 'building', at: now(), secs: 0 }); s.state = 'building';
  setTimeout(() => { s.state = 'active'; s.deploys[0].status = 'live'; s.deploys[0].secs = 44; }, 4000);
  audit(`redeploy ${svc} ${id}`); return { id };
}
export async function rollback(svc, deployId) {
  sample('rollback');
  if (real('rollback')) return http(`/services/${encodeURIComponent(svc)}/deploys/${encodeURIComponent(deployId)}/rollback`, { method: 'POST' });
  await lag(400);
  const s = S.project.services.find((x) => x.id === svc); if (!s) throw new Error('No such service');
  s.deploys.forEach((d) => { if (d.status === 'live') d.status = 'replaced'; if (d.id === deployId) d.status = 'live'; });
  audit(`rollback ${svc} to ${deployId}`); return { ok: true };
}
export function buildLog(svc, deploy) { sample('buildLog');
  const t = deploy.at; const L = (s, m) => `${hhmmss(t + s * 1000)} ${m}`;
  const lines = [L(0, `clone    ${svc.repo}@${deploy.id}`), L(1, `detect   ${svc.build}`), L(1, 'build    FROM node:22-alpine'), L(19, 'build    npm ci · 412 packages'), L(42, 'build    npm run build · ok'),
    L(47, `push     ${svc.id}:${deploy.id} (118 MB)`), L(48, 'deploy   start new version, wait for /healthz'), L(51, 'health   200 OK after 3 s'), L(51, 'switch   traffic moved, zero downtime'), L(52, 'done     stopped previous version')];
  if (deploy.status === 'failed') return [...lines.slice(0, 4), L(30, 'ERROR    npm ci exited with code 1'), L(30, 'ERROR    peer dependency conflict: react@19 vs react-dom@18'), L(31, 'abort    build failed, previous version kept running')];
  if (deploy.status === 'building') return lines.slice(0, 4);
  return lines;
}
export async function fetchBuildLog(svc, deploy) {
  sample('buildLog');
  if (real('buildLog')) return http(`/deploys/${encodeURIComponent(deploy.id)}/log`);
  return { id: deploy.id, status: deploy.status, error: deploy.error || null, lines: buildLog(svc, deploy).map((line) => ({ line })) };
}
export function streamDeployLog(deployId, onLine, onDone, onError) {
  if (MODE !== 'http' || !REAL.has('buildLog')) return () => {};
  const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/v1/stream/deploys/${encodeURIComponent(deployId)}`);
  ws.onmessage = (e) => {
    try {
      const m = JSON.parse(e.data);
      if (m.type === 'log') onLine?.(m);
      else if (m.type === 'done') onDone?.(m);
    } catch { /* ignore */ }
  };
  ws.onerror = () => onError?.('Build log stream connection failed.');
  return () => { try { ws.close(); } catch { /* closed */ } };
}

// ---------------------------------------------------------------- system
export async function getUpdates() { sample('getUpdates');
  if (real('getUpdates')) { const u = await http('/system/updates'); return { ...u, checked: u.checkedAt ? hhmm(u.checkedAt * 1000) : 'unknown' }; }
  await lag();
  return { list: clone(S.updates), total: S.updates.length + S.extraUpdates, security: S.updates.filter((u) => u.sec).length, reboot: S.rebootRequired, auto: S.autoUpdates, running: S.host.kernel, checked: '03:30' };
}
export async function installUpdates(which, packages = []) { sample('installUpdates');
  if (real('installUpdates')) { const { task } = await http('/system/updates/install', { method: 'POST', body: { which, packages } }); return { installed: (await waitTask(task)).result?.installed ?? 0 }; }
  await lag(1400);
  const before = S.updates.length;
  if (which === 'security') { S.updates.some((u) => u.sec && u.kernel) && (S.rebootRequired = true, S.host.kernelNew = '6.8.0-52'); S.updates = S.updates.filter((u) => !u.sec); }
  else if (which === 'selected') { S.updates = S.updates.filter((u) => !packages.includes(u.pkg)); }
  else { if (S.updates.some((u) => u.kernel)) S.rebootRequired = true; S.updates = []; S.extraUpdates = 0; }
  audit(`install ${which} updates (${before - S.updates.length})`); return { installed: before - S.updates.length };
}
export async function setAutoUpdates(on) { sample('setAutoUpdates');
  if (real('setAutoUpdates')) { const { task } = await http('/system/updates/auto', { method: 'PUT', body: { on } }); return waitTask(task); } await lag(150); S.autoUpdates = on; audit(`auto security updates ${on ? 'on' : 'off'}`); }
export async function reboot() { sample('reboot');
  if (real('reboot')) return http('/system/reboot', { method: 'POST' }); await lag(500); S.rebootRequired = false; S.host.kernel = S.host.kernelNew || S.host.kernel; S.host.upDays = 0; audit('reboot instance'); return { ok: true }; }
export async function getUnits() { sample('getUnits');
  if (real('getUnits')) return http('/system/units'); await lag(); return clone(S.units); }
export async function unitAction(name, action) { sample('unitAction');
  if (real('unitAction')) return http(`/system/units/${encodeURIComponent(name)}/${action}`, { method: 'POST' });
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
export async function getJournal({ unit = '', prio = 6, q = '' } = {}) { sample('getJournal');
  if (real('getJournal')) return http('/system/journal?' + new URLSearchParams({ unit, prio, q, limit: 300 }));
  await lag(100); const t = now();
  return JOURNAL.map((j, i) => ({ t: t - (JOURNAL.length - i) * 4 * MIN, proc: j[0], prio: j[1], msg: j[2] }))
    .filter((l) => (!unit || l.proc.startsWith(unit.replace('.service', ''))) && l.prio <= prio && (!q || (l.proc + l.msg).toLowerCase().includes(q.toLowerCase())));
}
export async function getJobs() { sample('getJobs');
  if (real('getJobs')) return (await http('/jobs')).map((j) => ({ ...j, last: when(j.lastRun) })); await lag(); return clone(S.jobs); }
export async function runJob(id) { sample('runJob');
  if (real('runJob')) { const { task } = await http(`/jobs/${encodeURIComponent(id)}/run`, { method: 'POST' }); return waitTask(task); } await lag(700); const j = S.jobs.find((x) => x.id === id); if (!j) throw new Error('No such job'); j.last = `today ${hhmm()}`; audit(`run job: ${j.name}`); return { ok: true }; }
export async function addJob(job) { sample('addJob');
  if (real('addJob')) return http('/jobs', { method: 'POST', body: { name: job.name, cron: job.cron, command: job.command } });
  await lag(250);
  if (!/^(\S+\s+){4}\S+$/.test((job.cron || '').trim())) throw new Error('Schedule must have 5 fields, e.g. 0 2 * * *');
  if (!(job.name || '').trim()) throw new Error('Describe what the job does.');
  S.jobs.push({ id: 'j' + (S.jobs.length + 1) + Date.now() % 1000, cron: job.cron.trim(), name: job.name.trim(), last: 'never', ok: true }); audit(`add job: ${job.name}`);
}
export async function getFirewall() { sample('getFirewall');
  if (real('getFirewall')) return http('/firewall'); await lag(); return { rules: clone(S.firewall), sshOpen: S.sshOpenUntil > now() }; }
export async function addRule(r) { sample('addRule');
  if (real('addRule')) return changeFirewall('/firewall/rules', { method: 'POST', body: { port: r.port, proto: r.proto || 'tcp', src: r.src || 'anywhere', note: r.note || '' } });
  await lag(250);
  const port = parseInt(r.port, 10);
  if (!(port >= 1 && port <= 65535)) throw new Error('Port must be between 1 and 65535.');
  if (!/^(anywhere|\d{1,3}(\.\d{1,3}){3}(\/\d{1,2})?)$/.test(r.src || 'anywhere')) throw new Error('Source must be "anywhere" or an IP / CIDR such as 203.0.113.0/24.');
  S.firewall.push({ id: 'f' + Date.now(), port: String(port), proto: r.proto || 'tcp', src: r.src || 'anywhere', note: r.note || '' }); audit(`firewall allow ${port}/${r.proto || 'tcp'} from ${r.src || 'anywhere'}`);
}
export async function removeRule(id) { sample('removeRule');
  if (real('removeRule')) return changeFirewall('/firewall/rules/' + encodeURIComponent(id), { method: 'DELETE' }); await lag(250); const r = S.firewall.find((x) => x.id === id); S.firewall = S.firewall.filter((x) => x.id !== id); audit(`firewall remove ${r ? r.port : id}`); }
export async function openSsh() { sample('openSsh');
  if (real('openSsh')) return openSshReal(); await lag(400); S.sshOpenUntil = now() + HOUR; S.units.find((u) => u.name === 'ssh.service').state = 'active'; audit('emergency SSH enabled for 1 hour', 'admin', true); return { until: S.sshOpenUntil }; }

// ---------------------------------------------------------------- cleanup
let cleanupCache = null;
export async function scanCleanup(fresh = false) { sample('scanCleanup');
  if (real('scanCleanup')) {
    if (!fresh && cleanupCache && !cleanupCache.pending && (Date.now() - cleanupCache._at) < 60000) {
      return clone(cleanupCache);
    }
    const r = await http(fresh ? '/cleanup/scan?fresh=1' : '/cleanup/scan');
    cleanupCache = { ...r, _at: Date.now() };
    return clone(r);
  }
  await lag(200);
  return { items: clone(S.cleanup), disk: S.metrics.disk, total: S.host.diskTotal, system: 41, auto: S.autoCleanup, pending: false, scannedAt: Math.floor(Date.now() / 1000), scanned: hhmm() };
}
export async function getCleanupVolumes() { sample('getCleanupVolumes');
  if (real('getCleanupVolumes')) return http('/cleanup/volumes');
  await lag(300); return { bytes: 300_000_000, gb: 0.3, count: 2 };
}
export async function runCleanup(ids) { sample('runCleanup');
  cleanupCache = null;
  if (real('runCleanup')) { const { task } = await http('/cleanup/run', { method: 'POST', body: { ids } }); const t = await waitTask(task, { every: 1000, max: 30 * 60 * 1000 }); return { freed: t.result?.freed ?? 0, bytes: t.result?.bytes ?? 0, items: t.result?.items ?? [] }; }
  await lag(1200);
  const hit = S.cleanup.filter((c) => ids.includes(c.id)); const freed = hit.reduce((a, c) => a + c.gb, 0);
  S.cleanup = S.cleanup.filter((c) => !ids.includes(c.id)); S.metrics.disk = Math.max(20, S.metrics.disk - freed);
  audit(`cleanup ${hit.map((c) => c.id).join(', ')} −${freed.toFixed(1)} GB`); return { freed };
}
export async function setAutoCleanup(on) { sample('setAutoCleanup'); if (real('setAutoCleanup')) return http('/cleanup/auto', { method: 'PUT', body: { on } }); await lag(150); S.autoCleanup = on; }

// ---------------------------------------------------------------- files
export async function listDir(path) { sample('listDir');
  if (real('listDir')) { const l = await http('/files?path=' + enc(path)); return { path: l.path, entries: l.entries.map((e) => ({ name: e.name, path: e.path, type: e.type, mode: e.mode, mtime: fileTime(e.mtime), size: e.size, warn: !!e.warn })) }; }
  await lag(80); const p = norm(path); if (!F[p] || F[p].type !== 'd') throw new Error(`Not a directory: ${p}`);
  const kids = childrenOf(p).map((k) => ({ name: k.split('/').pop(), path: k, ...F[k] }));
  kids.sort((a, b) => (a.type === b.type ? a.name.localeCompare(b.name) : a.type === 'd' ? -1 : 1));
  return { path: p, entries: kids.map((k) => ({ name: k.name, path: k.path, type: k.type, mode: k.mode, mtime: k.mtime, size: k.size ?? null, warn: k.name.startsWith('.env') && k.mode !== '-rw-------' })) };
}
export async function readFile(path) { sample('readFile');
  if (real('readFile')) { const f = await http('/files/content?path=' + enc(path)); seenMtime.set(f.path, f.mtime); return { path: f.path, content: f.content, mode: f.mode }; } await lag(60); const f = F[norm(path)]; if (!f || f.type !== 'f') throw new Error('Not a file'); return { path: norm(path), content: f.content, mode: f.mode }; }
export async function writeFile(path, content) { sample('writeFile');
  if (real('writeFile')) { const r = await http('/files/content', { method: 'PUT', body: { path, content, mtime: seenMtime.get(path) } }); seenMtime.set(path, r.mtime); return; } await lag(200); const f = F[norm(path)]; if (!f) throw new Error('No such file'); f.content = content; f.size = content.length; f.mtime = 'just now'; audit(`edit ${norm(path)}`); }
export async function chmod(path, mode) { sample('chmod');
  if (real('chmod')) return http('/files/chmod', { method: 'POST', body: { path, mode } });
  await lag(200); const f = F[norm(path)]; if (!f) throw new Error('No such file');
  if (!/^[0-7]{3}$/.test(mode)) throw new Error('Mode must be 3 octal digits, e.g. 600');
  const bits = ['---', '--x', '-w-', '-wx', 'r--', 'r-x', 'rw-', 'rwx']; f.mode = (f.type === 'd' ? 'd' : '-') + [...mode].map((d) => bits[+d]).join(''); audit(`chmod ${mode} ${norm(path)}`);
}
export async function mkdir(path) { sample('mkdir');
  if (real('mkdir')) return http('/files/mkdir', { method: 'POST', body: { path } }); await lag(150); const p = norm(path); if (F[p]) throw new Error('Already exists'); addDir(p, 'just now'); audit(`mkdir ${p}`); }
export async function removePaths(paths) { sample('removePaths');
  if (real('removePaths')) return http('/files/delete', { method: 'POST', body: { paths } }); await lag(250); paths.forEach((p) => { Object.keys(F).filter((k) => k === p || k.startsWith(p + '/')).forEach((k) => delete F[k]); }); audit(`delete ${paths.join(', ')}`); }
export async function upload(dir, file) { sample('upload');
  if (real('upload')) return uploadReal(dir, file); await lag(300); addFile(join(dir, file.name), '(uploaded)', '-rw-r--r--', 'just now', file.size); audit(`upload ${join(dir, file.name)}`); }

// ---------------------------------------------------------------- security
export async function getSecurity() { sample('getSecurity');
  if (real('getSecurity')) return http('/security');
  await lag(); const sshOpen = S.sshOpenUntil > now();
  const checks = clone(S.checks).map((c) => (c.id === 'sshd' ? { ...c, ok: !sshOpen, label: sshOpen ? 'on' : 'off', warn: sshOpen } : c));
  return { checks, users: clone(S.users), audit: clone(S.audit).slice(0, 12), allowIps: S.allowIps, passing: checks.filter((c) => c.ok).length, total: checks.length };
}
export async function toggleCheck(id) { sample('toggleCheck'); if (real('toggleCheck')) { if (id === 'autoupdates') return setAutoUpdates(true); return http(`/security/checks/${encodeURIComponent(id)}/fix`, { method: 'POST' }); } await lag(200); const c = S.checks.find((x) => x.id === id); if (c) { c.ok = true; delete c.action; } audit(`enable ${id}`); }
export async function setAllowIps(v, confirm = false) { sample('setAllowIps');
  if (real('setAllowIps')) {
    try { return await http('/security/allow-ips', { method: 'PUT', body: { value: v, confirm } }); }
    catch (e) { if (e.code === 'confirmation_required' && !confirm && hooks.confirm && await hooks.confirm(e.message)) return setAllowIps(v, true); throw e; }
  }
  await lag(200);
  const bad = v.split(',').map((s) => s.trim()).filter(Boolean).find((s) => !/^\d{1,3}(\.\d{1,3}){3}(\/\d{1,2})?$/.test(s));
  if (bad) throw new Error(`"${bad}" is not a valid IP or CIDR.`); S.allowIps = v; audit('update allowed IPs');
}
// An invitation: the new account's one-time link, shown once. `role` is viewer, deployer or admin.
export async function inviteUser(name, role = 'viewer') { sample('inviteUser');
  if (real('inviteUser')) { const r = await http('/users/invites', { method: 'POST', body: { name, role } }); return { ...r, link: `${location.origin}${location.pathname}#/invite?token=${r.token}` }; }
  await lag(250); if (!/^[a-z][a-z0-9._-]{2,}$/.test(name)) throw new Error('Username: at least 3 characters, lowercase.'); S.users.push({ name, role: 'Deploy and view logs, no terminal', tfa: false }); audit(`invite ${name}`); return { name, role, link: `${location.origin}${location.pathname}#/invite?token=demo-token` }; }
export async function removeUser(name) { sample('removeUser');
  if (real('removeUser')) return http('/users/' + encodeURIComponent(name), { method: 'DELETE' });
  await lag(200); S.users = S.users.filter((u) => u.name !== name); audit(`remove ${name}`); }
export async function signOutAll() {
  if (real('signOutAll')) return http('/auth/sessions/revoke-all', { method: 'POST' }); // POST /api/v1/auth/sessions/revoke-all
  sample('signOutAll'); await lag(300); audit('sign out all sessions');
}

// ---------------------------------------------------------------- recovery
export async function getRecovery() {
  if (real('getRecovery')) { if (!untracked) used.real.add('getRecovery'); return http('/recovery'); } // GET /api/v1/recovery
  sample('getRecovery');
  await lag(100); const failed = S.recovery.failed;
  return { failed, checks: failed
    ? [['opsbox.service', 'fail', 'failed', 'exit code 101 · restarted 3 times'], ['Health endpoint /healthz', 'fail', 'no answer', 'timeout after 5 s'], ['Disk /', 'fail', '100% full', '0 B free · likely cause: opsbox cannot write its database'], ['Docker', 'ok', 'running', '4 containers still serving traffic'], ['Network and DNS', 'ok', 'ok', '443 reachable from outside']]
    : [['opsbox.service', 'ok', 'active', 'running 3 days'], ['Health endpoint /healthz', 'ok', '200 OK', 'answered in 4 ms'], ['Disk /', 'warn', '82%', '22 GB free'], ['Docker', 'ok', 'running', '4 containers'], ['Network and DNS', 'ok', 'ok', '443 reachable from outside']],
    log: failed ? ['03:51:44 INFO  opsbox 0.1.4 starting', '03:51:46 ERROR sqlite: database or disk is full (code 13)', '03:51:47 FATAL cannot persist session store, exiting', '03:51:47 watchdog: exited with code 101, restart 1/3', '03:52:19 watchdog: restart 3/3 failed, entering safe mode'] : ['03:51:44 INFO  opsbox 0.1.0 starting', '03:51:45 INFO  listening on 127.0.0.1:8443'] };
}
export async function simulateFailure() { sample('simulateFailure'); S.recovery.failed = true; }
export async function recoveryAction(a) { sample('recoveryAction');
  await lag(900);
  if (a === 'free') { S.metrics.disk = 84; S.recovery.failed = false; return { msg: 'Freed 14.6 GB. opsbox restarted and is healthy.' }; }
  if (a === 'restart') { if (S.metrics.disk > 95) throw new Error('Restart failed again: disk is still full. Free disk space first.'); S.recovery.failed = false; return { msg: 'opsbox restarted and passed its health check.' }; }
  if (a === 'rollback') { S.recovery.failed = false; return { msg: 'Rolled back to 0.1.3. Settings and data kept.' }; }
  if (a === 'ssh') { S.sshOpenUntil = now() + HOUR; return { msg: 'Port 22 is open to your IP for one hour.' }; }
  if (a === 'reboot') { S.recovery.failed = false; return { msg: 'Rebooting… apps will return in about a minute.' }; }
  throw new Error('Unknown action');
}

// ---------------------------------------------------------------- terminal
// Linux accounts a terminal can be opened as: { default, users: [{ name, uid, home, shell, root }] }.
export async function getTerminalUsers() {
  if (real('getTerminalUsers')) { if (!untracked) used.real.add('getTerminalUsers'); return http('/terminal/users'); }
  sample('getTerminalUsers'); await lag(60);
  return { default: 'deploy', users: [{ name: 'deploy', uid: 1000, home: '/home/deploy', shell: '/bin/bash', root: false }, { name: 'root', uid: 0, home: '/root', shell: '/bin/bash', root: true }] };
}

// A real shell on this server over a WebSocket (http mode). Output arrives as bytes through
// `on.data`; `on.ready({user, shell})` fires once the shell runs, `on.exit(code)` when it
// ends, and `on.close({ ready, ended })` when the connection is gone. `confirmRoot` must be
// true to open a shell as root. Returns { send(text), sendBytes(bytes), resize(cols, rows), close() }.
export function openTerminal({ user = '', cols = 80, rows = 24, confirmRoot = false } = {}, on = {}) {
  if (!real('openTerminal')) throw new Error('The terminal needs a real server.');
  if (!untracked) used.real.add('openTerminal');
  const q = new URLSearchParams({ cols, rows }); if (user) q.set('user', user); if (confirmRoot) q.set('confirm', 'root');
  const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/v1/stream/terminal?${q}`);
  ws.binaryType = 'arraybuffer';
  const enc = new TextEncoder();
  let ready = false, ended = false, reason = '';
  ws.onmessage = (e) => {
    if (typeof e.data !== 'string') { on.data?.(new Uint8Array(e.data)); return; }
    let m; try { m = JSON.parse(e.data); } catch { return; }
    if (m.type === 'ready') { ready = true; on.ready?.(m); }
    else if (m.type === 'exit') { ended = true; on.exit?.(m.code); }
    else if (m.type === 'error') { reason = m.message || ''; on.error?.(reason); }
  };
  ws.onclose = async () => {
    // The server explains a refusal in an `error` message. Without one the upgrade itself was
    // refused (a browser cannot read why): the likely cause is the session or the role.
    let why = reason;
    if (!ready && !why) {
      try { await getTerminalUsers(); why = 'The server refused to open a terminal. It may be busy (too many are open) or its helper may be down.'; }
      catch (err) { why = err.message; }
    }
    on.close?.({ ready, ended, why });
  };
  const open = () => ws.readyState === 1;
  return {
    send: (text) => { if (open()) ws.send(enc.encode(text)); },
    sendBytes: (bytes) => { if (open()) ws.send(bytes); },
    resize: (c, r) => { if (open()) ws.send(JSON.stringify({ type: 'resize', cols: c, rows: r })); },
    close: () => { try { ws.close(); } catch { /* already closed */ } },
  };
}

export const HOSTS = [
  { id: 'local', name: 'srv-sgn-01', sub: 'local · root', user: 'root' },
  { id: 'db', name: 'db-primary', sub: 'deploy@10.0.1.12 · ed25519', user: 'deploy' },
  { id: 'edge', name: 'edge-hn-02', sub: 'ubuntu@203.0.113.40:2222', user: 'ubuntu' },
];
export async function termExec(host, cwd, line) { sample('termExec');
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
export async function getReclaimable() { sample('getReclaimable'); return { gb: S.cleanup.reduce((a, c) => a + c.gb, 0), parts: S.cleanup.map((c) => [c.id, c.gb, c.color]) }; }
export async function getAudit(n = 5) { sample('getAudit'); return clone(S.audit).slice(0, n); }
export async function addVar(svc, k, v, isSecret) {
  sample('addVar');
  if (real('addVar')) return http(`/services/${encodeURIComponent(svc)}/vars/${encodeURIComponent(k)}`, { method: 'PUT', body: { value: v, isSecret } });
  await lag(150); const s = S.project.services.find((x) => x.id === svc); if (!s) throw new Error('No such service');
  if (!/^[A-Z_][A-Z0-9_]*$/.test(k)) throw new Error('Variable names use UPPER_CASE letters, numbers and underscores.');
  s.vars = s.vars.filter((x) => x[0] !== k);
  s.vars.push([k, v, !!isSecret]); audit(`set variable ${k} on ${svc}`);
}
export async function removeVar(svc, k) {
  sample('removeVar');
  if (real('removeVar')) return http(`/services/${encodeURIComponent(svc)}/vars/${encodeURIComponent(k)}`, { method: 'DELETE' });
  await lag(150); const s = S.project.services.find((x) => x.id === svc); s.vars = s.vars.filter((x) => x[0] !== k); audit(`remove variable ${k} on ${svc}`);
}
export async function removeService(svc) {
  sample('removeService');
  if (real('removeService')) return http(`/services/${encodeURIComponent(svc)}`, { method: 'DELETE' });
  await lag(300); S.project.services = S.project.services.filter((x) => x.id !== svc); audit(`delete service ${svc}`);
}
export async function connectDatabase(svc, opts = {}) {
  sample('connectDatabase');
  if (real('connectDatabase')) {
    return http(`/services/${encodeURIComponent(svc)}/databases`, {
      method: 'POST',
      body: {
        database: opts.database,
        sourceKey: opts.sourceKey || undefined,
        asKey: opts.asKey || undefined,
        includeAll: !!opts.includeAll,
        perAppUser: opts.perAppUser,
        pooled: !!opts.pooled,
      },
    });
  }
  await lag(180);
  const s = S.project.services.find((x) => x.id === svc);
  const db = S.project.services.find((x) => x.id === opts.database && x.kind === 'db');
  if (!s || !db) throw new Error('Service or database not found.');
  const key = opts.asKey || opts.sourceKey || (db.templateId === 'redis' ? 'REDIS_URL' : db.templateId === 'mysql' || db.templateId === 'mariadb' ? 'MYSQL_URL' : db.templateId === 'mongo' ? 'MONGODB_URI' : 'DATABASE_URL');
  const srcKey = opts.sourceKey || key;
  const expr = `\${{ ${db.id}.${srcKey} }}`;
  s.vars = (s.vars || []).filter((x) => x[0] !== key);
  s.vars.push([key, expr, true, true]);
  S.project.edges = S.project.edges || [];
  if (!S.project.edges.some(([a, b]) => a === s.id && b === db.id)) S.project.edges.push([s.id, db.id]);
  db.connectedApps = db.connectedApps || [];
  const ex = db.connectedApps.find((c) => c.id === s.id);
  const need = s.state === 'active';
  if (ex) { if (!ex.vars.includes(key)) ex.vars.push(key); if (need) ex.needsRedeploy = true; }
  else db.connectedApps.push({ id: s.id, name: s.id, kind: s.kind, state: s.state, vars: [key], needsRedeploy: need });
  audit(`connect ${db.id} to ${s.id}`);
  return { ok: true, database: db.id, vars: [[key, expr, true, true]], needsRedeploy: need };
}
export async function disconnectDatabase(svc, dbId) {
  sample('disconnectDatabase');
  if (real('disconnectDatabase')) {
    return http(`/services/${encodeURIComponent(svc)}/databases/${encodeURIComponent(dbId)}`, { method: 'DELETE' });
  }
  await lag(150);
  const s = S.project.services.find((x) => x.id === svc);
  const db = S.project.services.find((x) => x.id === dbId);
  if (!s || !db) throw new Error('Not found');
  const pat = new RegExp(`^\\$\\{\\{\\s*${dbId}\\.`);
  const removedKeys = [];
  s.vars = (s.vars || []).filter((x) => {
    if (pat.test(String(x[1] || ''))) { removedKeys.push(x[0]); return false; }
    return true;
  });
  S.project.edges = (S.project.edges || []).filter(([a, b]) => !(a === s.id && b === dbId));
  db.connectedApps = (db.connectedApps || []).filter((c) => c.id !== s.id);
  audit(`disconnect ${dbId} from ${svc}`);
  return { ok: true, database: dbId, removedKeys };
}
export async function rotateDatabase(dbId) {
  sample('rotateDatabase');
  if (real('rotateDatabase')) return http(`/services/${encodeURIComponent(dbId)}/rotate`, { method: 'POST' });
  await lag(400);
  const db = S.project.services.find((x) => x.id === dbId && x.kind === 'db');
  if (!db) throw new Error('Database not found');
  const rotatedAt = Date.now();
  db.credentialsRotatedAt = rotatedAt;
  db.connectedApps = (db.connectedApps || []).map((c) => ({ ...c, needsRedeploy: true }));
  audit(`rotate credentials for ${dbId}`);
  return { ok: true, service: dbId, rotatedKeys: ['POSTGRES_PASSWORD', 'DATABASE_URL'], rotatedAt, affectedApps: clone(db.connectedApps) };
}
export async function setServiceExposure(svc, opts = {}) {
  sample('setServiceExposure');
  if (real('setServiceExposure')) {
    return http(`/services/${encodeURIComponent(svc)}/exposure`, {
      method: 'PUT',
      body: { mode: opts.mode, internalOnly: opts.internalOnly, publicPort: opts.publicPort, confirm: !!opts.confirm },
    });
  }
  await lag(200);
  const s = S.project.services.find((x) => x.id === svc);
  if (!s) throw new Error('Service not found');
  const mode = opts.mode || (opts.publicPort ? 'public' : opts.internalOnly ? 'internal' : 'loopback');
  if (mode === 'public' && s.kind === 'db' && !opts.confirm) {
    const err = new Error('Publishing database port publicly requires explicit confirmation.');
    err.code = 'confirmation_required';
    throw err;
  }
  s.networkMode = mode;
  s.internalOnly = mode === 'internal';
  s.publicPort = mode === 'public';
  audit(`set ${svc} network exposure to ${mode}`);
  return { ok: true, mode, redeployRequired: true, service: clone(s) };
}

// ---------------------------------------------------------------- databases: management (part B)
const DBM = {};
function dbSample(id, database) {
  const db = S.project.services.find((x) => x.id === id && x.kind === 'db');
  if (!db) throw new Error('Database not found.');
  const m = DBM[id] || (DBM[id] = {
    pooler: { enabled: false, mode: 'transaction' },
    apps: (db.connectedApps || []).map((c) => ({ app: c.id, database: `app_${c.id.replace(/-/g, '_')}`, user: `app_${c.id.replace(/-/g, '_')}`, pooled: false, connections: 2 })),
    ext: { vector: false, pg_stat_statements: true, pgcrypto: true, pg_trgm: false, postgis: false },
    version: (db.branch || '16.4'),
  });
  const cur = database || 'app';
  const flavorVector = db.templateId === 'postgres-pgvector', flavorGis = db.templateId === 'postgis';
  const available = (n) => !(n === 'vector' && !flavorVector) && !(n === 'postgis' && !flavorGis);
  const major = parseInt(String(m.version).replace(/^pg/, ''), 10);
  return {
    id, name: id, templateId: db.templateId || 'postgres', flavor: db.build || 'PostgreSQL', version: m.version,
    state: db.state, running: db.state === 'active',
    live: db.state !== 'active' ? null : {
      serverVersion: m.version, database: cur,
      databases: [{ name: 'app', owner: 'app', sizeBytes: 9_400_000 }, ...m.apps.map((a) => ({ name: a.database, owner: a.user, sizeBytes: 7_100_000 }))],
      users: [{ name: 'app', canLogin: true, superuser: true, connections: 1, databases: ['app', ...m.apps.map((a) => a.database)] }, ...m.apps.map((a) => ({ name: a.user, canLogin: true, superuser: false, connections: a.connections, databases: [a.database] }))],
      grants: m.apps.map((a) => ({ role: a.user, database: a.database, connect: true, create: true, temp: true })),
      extensions: Object.keys(m.ext).filter(available).map((n) => ({ name: n, installed: m.ext[n], installedVersion: m.ext[n] ? '1.0' : null, defaultVersion: '1.0', comment: '' })),
      tables: [{ schema: 'public', table: 'orders', totalBytes: 4_800_000, rowsEstimate: 18200 }, { schema: 'public', table: 'sessions', totalBytes: 1_300_000, rowsEstimate: 5400 }, { schema: 'public', table: 'users', totalBytes: 262_144, rowsEstimate: 810 }],
      connections: { current: 3 + m.apps.length * 2, max: 100 },
      statStatements: { loaded: true, enabled: m.ext.pg_stat_statements },
      slowQueries: m.ext.pg_stat_statements ? [{ query: 'SELECT * FROM orders WHERE customer_id = $1 ORDER BY created_at DESC', calls: 412, meanMs: 38.2, totalMs: 15738, rows: 4120 }, { query: 'UPDATE sessions SET seen_at = now() WHERE id = $1', calls: 9100, meanMs: 4.1, totalMs: 37310, rows: 9100 }] : [],
    },
    liveError: db.state === 'active' ? null : 'Start this database first. It is not running.',
    apps: clone(m.apps),
    pooler: { enabled: m.pooler.enabled, mode: m.pooler.mode, host: `${id}-pooler`, port: 6432 },
    upgrades: ['17.2'].filter(() => major < 17).map((v) => ({ version: v, major: 17 })),
    dataVolume: `opsbox-${id}-data`,
    credentialsRotatedAt: db.credentialsRotatedAt || null,
  };
}
export async function getDatabase(id, opts = {}) {
  sample('getDatabase');
  if (real('getDatabase')) return http(`/databases/${encodeURIComponent(id)}${opts.database ? `?database=${encodeURIComponent(opts.database)}` : ''}`);
  await lag(200);
  return dbSample(id, opts.database);
}
export async function rotateAppLogin(dbId, app) {
  sample('rotateAppLogin');
  if (real('rotateAppLogin')) return http(`/databases/${encodeURIComponent(dbId)}/apps/${encodeURIComponent(app)}/rotate`, { method: 'POST' });
  await lag(400); dbSample(dbId); audit(`rotate login of ${app} on ${dbId}`);
  return { ok: true, app, rotatedAt: now(), affectedApps: [{ id: app, name: app, needsRedeploy: true }] };
}
export async function removeAppLogin(dbId, app, opts = {}) {
  sample('removeAppLogin');
  if (real('removeAppLogin')) return http(`/databases/${encodeURIComponent(dbId)}/apps/${encodeURIComponent(app)}/remove`, { method: 'POST', body: { dropDatabase: !!opts.dropDatabase, confirm: opts.confirm } });
  await lag(300); dbSample(dbId); DBM[dbId].apps = DBM[dbId].apps.filter((a) => a.app !== app); audit(`remove login of ${app} on ${dbId}`);
  return { ok: true };
}
export async function setAppPooled(dbId, app, pooled) {
  sample('setAppPooled');
  if (real('setAppPooled')) return http(`/databases/${encodeURIComponent(dbId)}/apps/${encodeURIComponent(app)}/pooled`, { method: 'PUT', body: { pooled: !!pooled } });
  await lag(200); dbSample(dbId); const a = DBM[dbId].apps.find((x) => x.app === app); if (a) a.pooled = !!pooled;
  return { ok: true, pooled: !!pooled, needsRedeploy: true };
}
export async function setExtension(dbId, name, opts = {}) {
  sample('setExtension');
  if (real('setExtension')) return http(`/databases/${encodeURIComponent(dbId)}/extensions/${encodeURIComponent(name)}`, { method: 'PUT', body: { enabled: !!opts.enabled, database: opts.database || undefined } });
  await lag(250); dbSample(dbId); DBM[dbId].ext[name] = !!opts.enabled; audit(`${opts.enabled ? 'enable' : 'disable'} ${name} on ${dbId}`);
  return { ok: true };
}
export async function setPooler(dbId, opts = {}) {
  sample('setPooler');
  if (real('setPooler')) {
    const { task } = await http(`/databases/${encodeURIComponent(dbId)}/pooler`, { method: 'PUT', body: { enabled: !!opts.enabled, mode: opts.mode || undefined } });
    return waitTask(task, { every: 1000, max: 10 * 60 * 1000 });
  }
  await lag(900); dbSample(dbId); DBM[dbId].pooler = { enabled: !!opts.enabled, mode: opts.mode || 'transaction' }; audit(`${opts.enabled ? 'enable' : 'disable'} pooler on ${dbId}`);
  return { ok: true };
}
export async function upgradeDatabase(dbId, opts = {}) {
  sample('upgradeDatabase');
  if (real('upgradeDatabase')) {
    const { task } = await http(`/databases/${encodeURIComponent(dbId)}/upgrade`, { method: 'POST', body: { version: opts.version, confirm: !!opts.confirm } });
    return waitTask(task, { every: 2000, max: 4 * 60 * 60 * 1000 });
  }
  if (!opts.confirm) { const err = new Error('The database will be stopped for a few minutes. Confirm to continue.'); err.code = 'confirmation_required'; throw err; }
  await lag(1800); dbSample(dbId); DBM[dbId].version = opts.version; audit(`upgrade ${dbId} to ${opts.version}`);
  return { ok: true, result: { version: opts.version, oldVolume: `opsbox-${dbId}-data`, databases: 2 }, log: ['Safety backup saved.', 'Restored and checked 2 databases.'] };
}
export async function revealConnectionString(dbId, opts = {}) {
  sample('revealConnectionString');
  if (real('revealConnectionString')) return http(`/databases/${encodeURIComponent(dbId)}/connection-string`, { method: 'POST', body: { code: opts.code, app: opts.app || undefined, pooled: !!opts.pooled } });
  await lag(250);
  if (String(opts.code).trim() !== '123456') throw new Error('That code was not accepted. Codes work once, so wait for the next one and try again.');
  audit(`reveal connection string of ${dbId}`);
  const who = opts.app ? `app_${opts.app.replace(/-/g, '_')}` : 'app';
  return { connectionString: `postgresql://${who}:s4mpleP4ssw0rd@${dbId}:5432/${who}`, login: opts.app || 'administrator', note: 'This name only works from other containers in the same project. Your database is not reachable from the internet.' };
}

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
export async function getBackups() { sample('getBackups');
  if (real('getBackups')) {
    const res = await http('/backups');
    return {
      jobs: (res.jobs || []).map((j) => ({
        id: j.id,
        name: j.name,
        kind: j.kind,
        kindLabel: j.kindLabel || j.kind,
        what: j.name === j.what ? j.name : `${j.name} (${j.what})`,
        rawWhat: j.what,
        whatDetail: j.whatDetail || {},
        note: `${j.kindLabel || j.kind} · ${j.method} · ${j.what}`,
        method: j.method,
        cron: j.cron,
        cronHuman: j.cronHuman || j.cron,
        retainDaily: j.retainDaily,
        retainWeekly: j.retainWeekly,
        keep: j.retention || `${j.retainDaily} daily · ${j.retainWeekly} weekly`,
        dest: j.destinationId === 'local' ? 'local' : (j.destination || j.destinationId),
        destinationId: j.destinationId,
        enabled: j.enabled !== false,
        last: j.last || 'never',
        lastError: j.lastError || null,
        state: j.lastStatus === 'ok' ? 'ok' : j.lastStatus === 'failed' ? 'fail' : 'warn',
      })),
      items: (res.snapshots || []).map((s) => ({
        id: s.id,
        resticId: s.resticId,
        shortId: s.shortId,
        kind: s.kind,
        at: s.when || when(s.takenAt),
        what: s.what,
        size: s.sizeBytes,
        sizeBytes: s.sizeBytes,
        where: s.where || s.destinationId || 'local',
        check: s.checkLabel || s.check,
        checkState: s.check,
      })),
      dests: (res.destinations || []).map((d) => ({
        id: d.id,
        name: d.name,
        type: d.provider,
        loc: d.detail || '/var/backups/opsbox',
        ok: d.status === 'ok' && (d.id !== 'local' || !res.sameDisk),
        status: d.status,
        isDefault: !!d.isDefault,
        publicKey: d.publicKey || null,
        fingerprint: d.fingerprint || null,
        verifiedAt: d.verifiedAt || null,
      })),
      defaultDestinationId: res.defaultDestinationId || 'local',
      restoreTest: res.restoreTest || null,
      localDisk: res.localDisk || null,
      sameDisk: !!res.sameDisk,
      remoteDestinationsReady: !!res.remoteDestinationsReady,
      restoreReady: !!res.restoreReady,
    };
  }
  await lag();
  return {
    ...clone(BK),
    sameDisk: true,
    remoteDestinationsReady: true,
    restoreReady: true,
    defaultDestinationId: BK.dests[0]?.id || 'local',
    restoreTest: null,
    localDisk: {
      path: '/var/backups/opsbox',
      sameDisk: true,
      sameDiskPaths: ['/var/lib/opsbox/opsbox.db'],
      resticInstalled: true,
    },
  };
}
export async function runBackup(id) { sample('runBackup');
  if (real('runBackup')) {
    const path = id ? `/backups/jobs/${encodeURIComponent(id)}/run` : '/backups/run';
    const { task } = await http(path, { method: 'POST', body: {} });
    return waitTask(task, { every: 500 });
  }
  await lag(900); audit(`backup ${id || 'all jobs'}`); const j = BK.jobs.find((x) => x.id === id); if (j) j.last = 'ok · just now'; return { ok: true };
}
export async function createBackupJob(data) { sample('createBackupJob');
  if (real('createBackupJob')) return http('/backups/jobs', { method: 'POST', body: data });
  await lag(300);
  const id = 'j' + Date.now();
  BK.jobs.push({ id, what: data.name, note: (data.paths || data.volumes || []).join(', '), cron: data.cron, keep: `${data.retainDaily || 14} daily · ${data.retainWeekly || 8} weekly`, dest: 'Local disk', state: 'warn', last: 'never' });
  return { job: { id } };
}
export async function updateBackupJob(id, data) { sample('updateBackupJob');
  if (real('updateBackupJob')) return http(`/backups/jobs/${encodeURIComponent(id)}`, { method: 'PATCH', body: data });
  await lag(250);
  const j = BK.jobs.find((x) => x.id === id);
  if (j && data.cron) j.cron = data.cron;
  return { job: j };
}
export async function deleteBackupJob(id) { sample('deleteBackupJob');
  if (real('deleteBackupJob')) return http(`/backups/jobs/${encodeURIComponent(id)}`, { method: 'DELETE' });
  await lag(250);
  BK.jobs = BK.jobs.filter((x) => x.id !== id);
  return { ok: true };
}
export async function pruneBackups(opts = {}) { sample('pruneBackups');
  if (real('pruneBackups')) {
    const { task } = await http('/backups/prune', { method: 'POST', body: opts });
    return waitTask(task, { every: 500 });
  }
  await lag(600);
  return { ok: true, result: { removed: 0, kept: BK.items.length } };
}
export async function checkBackups(opts = {}) { sample('checkBackups');
  if (real('checkBackups')) {
    const { task } = await http('/backups/check', { method: 'POST', body: opts });
    return waitTask(task, { every: 500 });
  }
  await lag(800);
  return { ok: true, result: { ok: true, summary: 'Repository checksum verification passed.' } };
}
export async function restoreBackup(id, mode, opts = {}) { sample('restoreBackup');
  if (real('restoreBackup')) {
    const { task } = await http(`/backups/snapshots/${encodeURIComponent(id)}/restore`, {
      method: 'POST',
      body: { mode, confirm: opts.confirm, copyTarget: opts.copyTarget },
    });
    const done = await waitTask(task, { every: 500, max: 30 * 60 * 1000 });
    return { msg: (done.result && done.result.message) || 'Restore completed.', result: done.result };
  }
  await lag(1200); const k = BK.items.find((x) => x.id === id); if (!k) throw new Error('No such backup');
  if (mode !== 'copy' && mode !== 'replace') throw new Error('Choose how to restore.'); audit(`restore ${k.what} (${k.at}) as ${mode}`); return { msg: mode === 'copy' ? `Restored ${k.what} into ${opts.copyTarget || `${k.what}-restored`}. Check it, then swap it in.` : `${k.what} replaced from ${k.at}.` };
}
export async function testBackupRestore(id) { sample('testBackupRestore');
  if (real('testBackupRestore')) {
    const path = id ? `/backups/snapshots/${encodeURIComponent(id)}/test` : '/backups/restore-test';
    const { task } = await http(path, { method: 'POST', body: {} });
    return waitTask(task, { every: 500, max: 30 * 60 * 1000 });
  }
  await lag(900);
  const k = id ? BK.items.find((x) => x.id === id) : BK.items[0];
  if (k) k.check = 'restore-tested';
  return { ok: true, result: { ok: true, checkState: 'restore_tested' } };
}
export async function downloadRecoveryKey(password) { sample('downloadRecoveryKey');
  if (real('downloadRecoveryKey')) {
    return http('/backups/recovery-key', { method: 'POST', body: { password } });
  }
  await lag(250);
  if (!password) throw new Error('Enter your password.');
  return { password: 'demo-recovery-key-from-master-key', hostname: 'srv-sgn-01', instructions: 'Keep offline in a password manager.' };
}
export async function testSavedDestination(id) { sample('testSavedDestination');
  if (real('testSavedDestination')) {
    const r = await http(`/backups/destinations/${encodeURIComponent(id)}/test`, { method: 'POST', body: {} });
    return { ...r, steps: (r.steps || []).map((s) => (Array.isArray(s) ? s : [s.status, s.message])) };
  }
  await lag(600);
  const d = BK.dests.find((x) => x.id === id);
  if (d) d.ok = true;
  return { ok: true, verified: true, steps: [['ok', 'Verified repository read/write.']] };
}
export async function setDefaultDestination(id, opts = {}) { sample('setDefaultDestination');
  if (real('setDefaultDestination')) {
    return http(`/backups/destinations/${encodeURIComponent(id)}/default`, { method: 'POST', body: opts });
  }
  await lag(200);
  BK.dests.forEach((d) => { d.isDefault = d.id === id; });
  return { ok: true };
}
export async function deleteDestination(id) { sample('deleteDestination');
  if (real('deleteDestination')) {
    return http(`/backups/destinations/${encodeURIComponent(id)}`, { method: 'DELETE' });
  }
  await lag(250);
  BK.dests = BK.dests.filter((d) => d.id !== id);
  return { ok: true };
}
export async function addDestination(d) { sample('addDestination');
  await lag(500);
  if (!(d.name || '').trim()) throw new Error('Give the destination a name.');
  if (!/^s3:\/\/[a-z0-9.-]{3,}(\/.*)?$/.test(d.loc || '')) throw new Error('Location must look like s3://bucket/path');
  BK.dests.push({ id: 'd' + Date.now(), name: d.name.trim(), type: 'S3-compatible', loc: d.loc, ok: true }); audit(`add backup destination ${d.name}`);
}

// ---------------------------------------------------------------- alerts
const AL = {
  firing: true,
  incidents: [{ id: 'disk:/', name: 'Disk space', msg: 'Disk / is at 82%. Free space or expand the disk.', state: 'firing', since: Math.floor(now() / 1000) - 300, silenceUntil: 0, link: '#/cleanup' }],
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
export async function getAlerts() { if (real('getAlerts')) return http('/alerts'); sample('getAlerts'); await lag(); return clone(AL); }
export async function setRule(id, on) { if (real('setRule')) return http(`/alerts/rules/${encodeURIComponent(id)}`, { method: 'PUT', body: { on } }); sample('setRule'); await lag(150); const r = AL.rules.find((x) => x.id === id); if (!r) throw new Error('No such rule'); r.on = on; audit(`alert rule ${r.name} ${on ? 'on' : 'off'}`); }
export async function testAlert(channel) { if (real('testAlert')) return http('/alerts/test', { method: 'POST', body: { channel: channel || null } }); sample('testAlert'); await lag(600); const c = AL.channels.find((x) => x.id === channel); if (channel && (!c || !c.on)) throw new Error('Connect this channel first.'); audit(`test alert${c ? ' to ' + c.name : ''}`); return { ok: true }; }
export async function connectChannel(id, config) { if (real('connectChannel')) return http(`/alerts/channels/${encodeURIComponent(id)}`, { method: 'PUT', body: config }); const to = config.to || config.chat || config.url || 'Configured'; sample('connectChannel');
  await lag(400); const c = AL.channels.find((x) => x.id === id); if (!c) throw new Error('No such channel');
  if (id === 'c4' && !/^https:\/\//.test(to || '')) throw new Error('Webhook URL must start with https://');
  if (id === 'c3' && !/^-?\d{5,}$/.test(to || '')) throw new Error('Enter your Telegram chat id (digits only).');
  c.to = to; c.for = 'all alerts'; c.on = true; audit(`connect ${c.name}`);
}
export async function disconnectChannel(id) { if (real('disconnectChannel')) return http(`/alerts/channels/${encodeURIComponent(id)}`, { method: 'DELETE' }); sample('disconnectChannel'); const c = AL.channels.find((c) => c.id === id); if (c) c.on = false; return { ok: true }; }
export async function silenceAlert(id, seconds = 3600) { if (real('silenceAlert')) return http('/alerts/silence', { method: 'POST', body: { id, seconds } }); sample('silenceAlert'); await lag(150); AL.firing = false; const incident = AL.incidents.find((i) => i.id === id); if (incident) incident.silenceUntil = Math.floor(now() / 1000) + seconds; AL.history[0].state = 'silenced'; audit('silence alert 1 h'); }
export async function setQuiet(q) { if (real('setQuiet')) return http('/alerts/quiet', { method: 'PUT', body: q }); sample('setQuiet'); await lag(150); AL.quiet = { ...AL.quiet, ...q }; audit('update quiet hours'); }

// ---------------------------------------------------------------- templates
export const TEMPLATES = {
  databases: [
    { id: 'postgres', name: 'PostgreSQL', desc: 'Relational database · daily backup on by default', res: '≈ 100 MB RAM · 1 GB disk', versions: ['17.2', '16.4', '16.3', '15.8'], port: 5432, internalByDefault: true, backup: { enabledByDefault: true } },
    { id: 'postgres-pgvector', name: 'PostgreSQL + pgvector', desc: 'PostgreSQL with the pgvector extension for AI embeddings · daily backup on by default', res: '≈ 100 MB RAM · 1 GB disk', versions: ['pg17', 'pg16', 'pg15'], port: 5432, internalByDefault: true, backup: { enabledByDefault: true } },
    { id: 'postgis', name: 'PostgreSQL + PostGIS', desc: 'PostgreSQL with PostGIS for maps and locations · daily backup on by default', res: '≈ 200 MB RAM · 2 GB disk', versions: ['17-3.5', '16-3.4', '15-3.4'], port: 5432, internalByDefault: true, backup: { enabledByDefault: true } },
    { id: 'mysql', name: 'MySQL', desc: 'Relational database', res: '≈ 400 MB RAM · 1 GB disk', versions: ['8.4', '8.0'], port: 3306, internalByDefault: true, backup: { enabledByDefault: true } },
    { id: 'mariadb', name: 'MariaDB', desc: 'MySQL-compatible database', res: '≈ 150 MB RAM · 1 GB disk', versions: ['11.4'], port: 3306, internalByDefault: true, backup: { enabledByDefault: true } },
    { id: 'redis', name: 'Redis', desc: 'Cache and queues', res: '≈ 20 MB RAM', versions: ['7.4'], port: 6379, internalByDefault: true, backup: { enabledByDefault: true } },
    { id: 'mongo', name: 'MongoDB', desc: 'Document database', res: '≈ 300 MB RAM · 1 GB disk', versions: ['7.0'], port: 27017, internalByDefault: true, backup: { enabledByDefault: true } },
  ],
  tools: [
    { id: 'minio', name: 'MinIO', desc: 'S3-compatible file storage', res: '≈ 150 MB RAM', versions: ['latest'], port: 9000, internalByDefault: true, backup: { enabledByDefault: false } },
    { id: 'kuma', name: 'Uptime Kuma', desc: 'Watch your public URLs from outside', res: '≈ 100 MB RAM', versions: ['1', '2'], port: 3001, internalByDefault: true, backup: { enabledByDefault: false } },
    { id: 'umami', name: 'Umami', desc: 'Privacy-friendly analytics (needs PostgreSQL)', res: '≈ 150 MB RAM', versions: ['postgresql-latest', 'latest'], port: 3000, internalByDefault: true, requiresDatabase: true, backup: { enabledByDefault: false } },
    { id: 'n8n', name: 'n8n', desc: 'Workflow automation', res: '≈ 250 MB RAM', versions: ['latest'], port: 5678, internalByDefault: true, backup: { enabledByDefault: false } },
    { id: 'grafana', name: 'Grafana', desc: 'Dashboards for your own metrics', res: '≈ 200 MB RAM', versions: ['11', 'latest'], port: 3000, internalByDefault: true, backup: { enabledByDefault: false } },
    { id: 'adminer', name: 'Adminer', desc: 'Browse and edit your databases in the browser · opens only through opsbox sign-in', res: '≈ 30 MB RAM', versions: ['4.17.1', 'latest'], port: 8080, internalByDefault: false, backup: { enabledByDefault: false } },
    { id: 'pgweb', name: 'pgweb', desc: 'Lightweight PostgreSQL browser, already connected to one database · opens only through opsbox sign-in', res: '≈ 30 MB RAM', versions: ['0.16.2', 'latest'], port: 8081, internalByDefault: false, requiresDatabase: true, backup: { enabledByDefault: false } },
  ],
};
export async function getTemplates() {
  sample('getTemplates');
  if (real('getTemplates')) return http('/templates');
  await lag();
  return clone(TEMPLATES);
}
export async function addTemplate(id, cfg) {
  sample('addTemplate');
  if (real('addTemplate')) {
    const r = await http(`/templates/${encodeURIComponent(id)}/instances`, {
      method: 'POST',
      body: {
        name: cfg.name,
        version: cfg.version,
        disk: cfg.disk,
        backup: cfg.backup,
        internalOnly: cfg.internalOnly,
        publicPort: !!cfg.publicPort,
        confirm: !!cfg.confirm,
        databaseUrl: cfg.databaseUrl || undefined,
      },
    });
    if (r.task) await waitTask(r.task, { every: 500, max: 5 * 60 * 1000 });
    return r;
  }
  await lag(700);
  const t = [...TEMPLATES.databases, ...TEMPLATES.tools].find((x) => x.id === id); if (!t) throw new Error('Unknown template');
  const name = (cfg.name || '').trim();
  if (!/^[a-z][a-z0-9-]{1,30}$/.test(name)) throw new Error('Name: lowercase letters, numbers and dashes, starting with a letter.');
  if (S.project.services.some((x) => x.id === name)) throw new Error(`A service named "${name}" already exists.`);
  const idx = S.project.services.length;
  S.project.services.push({ id: name, kind: ['minio', 'kuma', 'umami', 'n8n', 'grafana', 'adminer', 'pgweb'].includes(t.id) ? 'tool' : 'db', state: 'active', repo: `${t.name} ${cfg.version || t.versions[0]} · volume ${cfg.disk || '1 GB'}`, templateId: t.id, port: t.port, internalOnly: cfg.internalOnly !== false, publicPort: !!cfg.publicPort, networkMode: cfg.publicPort ? 'public' : (cfg.internalOnly !== false ? 'internal' : 'loopback'), connectedApps: [], domain: '', x: 370 * (idx % 3), y: 10 + 176 * Math.floor(idx / 3), note: cfg.backup ? 'backup daily 02:00' : 'no backup', deploys: [], vars: [] });
  audit(`add ${t.name} as ${name}`);
  return { id: name, connectionVar: 'DATABASE_URL', connectionUrl: t.id.startsWith('postgres') || t.id === 'postgis' ? `postgresql://app:demo@${name}:5432/app` : undefined };
}
export async function deployCompose(yamlOrOpts, maybeOpts = {}, confirmed = false) {
  sample('deployCompose');
  const text = typeof yamlOrOpts === 'string' ? yamlOrOpts : yamlOrOpts?.yaml;
  const opts = typeof yamlOrOpts === 'string' ? maybeOpts : (yamlOrOpts || {});
  if (real('deployCompose')) {
    try {
      const r = await http('/compose', {
        method: 'POST',
        body: {
          stack: opts.stack || undefined,
          yaml: text,
          confirm: !!confirmed,
        },
      });
      if (r.task) await waitTask(r.task, { every: 500, max: 5 * 60 * 1000 });
      return r;
    } catch (e) {
      if (e.code === 'confirmation_required' && !confirmed && hooks.confirm && await hooks.confirm(e.message, 'Allow elevated Compose access?')) {
        return deployCompose(text, opts, true);
      }
      throw e;
    }
  }
  await lag(700);
  const names = [...(text || '').matchAll(/^ {2}([a-z][a-z0-9_-]*):\s*$/gm)].map((m) => m[1]);
  if (!/^services:/m.test(text || '') || !names.length) throw new Error('This does not look like a Compose file: it needs a "services:" section.');
  names.forEach((n) => { if (!S.project.services.some((x) => x.id === n)) S.project.services.push({ id: n, kind: 'compose', state: 'active', repo: 'compose', branch: n, build: 'Compose', domain: '', port: 0, x: 370 * ((S.project.services.length) % 3), y: 10 + 176 * Math.floor(S.project.services.length / 3), vars: [], deploys: [{ id: Math.random().toString(16).slice(2, 9), msg: 'Compose deploy', status: 'live', at: now(), secs: 20 }] }); });
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
export async function getAccess() { sample('getAccess'); if (real('getAccess')) return http('/access'); await lag(); return { ...clone(AC), sshOpen: S.sshOpenUntil > now() }; }
export async function addKey(user, pub) { sample('addKey');
  if (real('addKey')) { await http('/access/keys', { method: 'POST', body: { user, key: pub } }); return; }
  await lag(300);
  if (/PRIVATE KEY/.test(pub || '')) throw new Error('That is a private key. Never paste it here. Paste the .pub file instead.');
  const m = /^(ssh-ed25519|ssh-rsa|ecdsa-sha2-nistp\d+)\s+[A-Za-z0-9+/=]{20,}(\s+(.+))?$/.exec((pub || '').trim());
  if (!m) throw new Error('This is not a public key. It should start with ssh-ed25519 or ssh-rsa.');
  AC.keys.push({ id: 'a' + Date.now(), user, name: m[3] || 'unnamed key', type: m[1].replace('ssh-', ''), fp: 'SHA256:new…key', added: 'today', used: '' }); audit(`add ssh key for ${user}`);
}
export async function removeKey(id) { sample('removeKey'); if (real('removeKey')) { await http('/access/keys/remove', { method: 'POST', body: { id } }); return; } await lag(200); AC.keys = AC.keys.filter((k) => k.id !== id); audit('remove ssh key'); }
export async function lockUser(name, lock) { sample('lockUser');
  if (real('lockUser')) { await http(`/access/users/${encodeURIComponent(name)}/lock`, { method: 'POST', body: { lock } }); return; }
  await lag(250); const u = AC.users.find((x) => x.name === name); if (!u) throw new Error('No such user');
  if (lock && name === 'deploy') throw new Error('deploy runs your apps. Locking it would stop deployments.');
  u.locked = lock; audit(`${lock ? 'lock' : 'unlock'} user ${name}`);
}
export async function rotateOutKey(id) { sample('rotateOutKey'); await lag(500); audit('rotate outgoing key'); }

// ---------------------------------------------------------------- settings
const ST = {
  hostname: 'srv-sgn-01', tz: 'Asia/Ho_Chi_Minh', panel: 'panel.example.com', swap: '2 GB', autoUpdate: true, channel: 'stable', fsGrown: false,
  github: {
    connected: true, installed: true, appSlug: 'opsbox-srv-sgn-01', appName: 'opsbox srv-sgn-01',
    owner: 'acme-co', account: 'acme-co', manageUrl: 'https://github.com/settings/installations',
    installUrl: 'https://github.com/apps/opsbox-srv-sgn-01/installations/new',
    repositorySelection: 'selected', webhookUrl: 'https://panel.example.com/api/v1/github/webhook',
    webhookActive: true, webhookConfigured: true, panelDomain: 'panel.example.com', panelDomainSet: true,
  },
  deployKeys: [],
  panelInfo: {
    domain: 'panel.example.com', activeDomain: 'panel.example.com', certStatus: 'valid',
    certExpiresAt: Math.floor(Date.now() / 1000) + 74 * 86400, certDaysRemaining: 74, certIssuer: "Let's Encrypt",
    certStaging: false, certError: null, proxyReachable: true, rawPortOpen: true, listen: '0.0.0.0:8443',
  },
  tokens: [{ id: 't1', name: 'github-actions', can: 'deploy only, service web', last: '2 min ago' }, { id: 't2', name: 'uptime-check', can: 'read-only metrics', last: '1 min ago' }],
};
// Swap as the page's choices: None, 1 GB, 2 GB, 4 GB (the system reports a little under the size).
const swapLabel = (mb) => { if (!mb) return 'None'; const gb = Math.round(mb / 1024); return Math.abs(mb - gb * 1024) <= gb * 1024 * 0.02 ? `${gb} GB` : `${(mb / 1024).toFixed(1)} GB`; };
const normPanelDomain = (raw) => {
  const s = String(raw || '').trim().toLowerCase().replace(/^https:\/\//, '').replace(/\/$/, '');
  if (!DOMRE.test(s) || s.startsWith('*.')) throw new Error('Enter a domain name like panel.example.com (no wildcard, port, or path).');
  return s;
};
let realSettings = null;
export async function getSettings() { sample('getSettings');
  if (real('getSettings')) {
    // Host name, time zone, swap, tokens, opsbox updates, panel domain and GitHub are real;
    // disk growth and export/uninstall arrive with later phases.
    const r = await http('/settings'); realSettings = r;
    return {
      ...clone(ST),
      hostname: r.hostname,
      tz: r.tz,
      timezones: r.timezones,
      panel: r.panel?.domain || '',
      panelInfo: r.panel || null,
      swap: swapLabel(r.swapMb),
      tokens: r.tokens.map((t) => ({ id: t.id, name: t.name, can: t.can, last: t.last })),
      opsboxUpdate: r.opsboxUpdate,
      github: r.github || { connected: false, installed: false },
      deployKeys: r.deployKeys || [],
    };
  }
  await lag(); return clone(ST); }
export async function saveSettings(v) { sample('saveSettings');
  if (real('saveSettings')) {
    const cur = realSettings || await http('/settings');
    if (v.hostname !== cur.hostname) { await http('/system/hostname', { method: 'PUT', body: { name: v.hostname } }); cur.hostname = v.hostname; S.host.name = v.hostname; }
    if (v.tz !== cur.tz) { await http('/system/timezone', { method: 'PUT', body: { zone: v.tz } }); cur.tz = v.tz; }
    return;
  }
  await lag(300);
  if (!/^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/i.test(v.hostname || '')) throw new Error('Hostname: letters, numbers and dashes only.');
  Object.assign(ST, { hostname: v.hostname, tz: v.tz }); S.host.name = v.hostname; audit('update general settings');
}
export async function checkPanelDns(domain) { sample('checkPanelDns');
  if (real('checkPanelDns')) return http('/panel/dns-check', { method: 'POST', body: { domain } });
  await lag(300);
  const name = normPanelDomain(domain);
  ST.panel = name;
  const waiting = name.includes('wait');
  return {
    domain: name,
    status: waiting ? 'waiting' : 'points_here',
    pointsHere: !waiting,
    message: waiting ? 'DNS does not resolve to this server yet.' : 'DNS points to this server.',
    resolvedIps: waiting ? [] : ['203.0.113.10'],
    serverIps: ['203.0.113.10'],
    cname: null,
    records: [{ type: 'A', name, value: '203.0.113.10' }],
  };
}
export async function issuePanelCert(domain) { sample('issuePanelCert');
  if (real('issuePanelCert')) {
    const { task } = await http('/panel/certificate', { method: 'POST', body: { domain } });
    const t = await waitTask(task, { every: 1000, max: 3 * 60 * 1000 });
    return t.result;
  }
  await lag(700);
  const name = normPanelDomain(domain);
  const nowSec = Math.floor(Date.now() / 1000);
  Object.assign(ST, { panel: name });
  ST.panelInfo = { ...ST.panelInfo, domain: name, activeDomain: name, certStatus: 'valid', certExpiresAt: nowSec + 90 * 86400, certDaysRemaining: 90, certIssuer: "Let's Encrypt", certStaging: false, certError: null };
  audit(`issue certificate for ${name}`);
  return { domain: name, status: 'valid', notAfter: ST.panelInfo.certExpiresAt, daysRemaining: 90, issuer: "Let's Encrypt", staging: false, lastError: null };
}
export async function removePanelDomain(confirm = false) { sample('removePanelDomain');
  if (real('removePanelDomain')) return http('/panel/domain', { method: 'DELETE', body: { confirm } });
  await lag(250);
  ST.panel = '';
  ST.panelInfo = { ...ST.panelInfo, domain: null, activeDomain: null, certStatus: 'none', certExpiresAt: null, certDaysRemaining: null, certIssuer: null, certError: null };
  audit('remove panel domain');
  return { ok: true };
}
export async function setPanelLocalOnly(localOnly, confirm = false) { sample('setPanelLocalOnly');
  if (real('setPanelLocalOnly')) return http('/panel/local-only', { method: 'POST', body: { localOnly, confirm } });
  await lag(300);
  ST.panelInfo = { ...ST.panelInfo, rawPortOpen: !localOnly, listen: localOnly ? '127.0.0.1:8443' : '0.0.0.0:8443' };
  audit(localOnly ? 'close raw 8443 listener' : 're-open raw 8443 listener');
  return { ok: true, localOnly, restarting: true };
}
export async function createToken(name, can) { sample('createToken');
  if (real('createToken')) { const scope = { 'deploy only': 'deploy', 'read-only metrics': 'read', 'full access': 'full' }[can] || can; const r = await http('/tokens', { method: 'POST', body: { name, scope } }); return { secret: r.secret }; }
  await lag(300); if (!/^[a-z][a-z0-9-]{2,}$/.test(name || '')) throw new Error('Token name: at least 3 characters, lowercase, dashes allowed.');
  const secret = 'ops_' + Math.random().toString(36).slice(2) + Math.random().toString(36).slice(2);
  ST.tokens.push({ id: 't' + Date.now(), name, can, last: 'never' }); audit(`create api token ${name}`); return { secret };
}
export async function revokeToken(id) { sample('revokeToken'); if (real('revokeToken')) return http('/tokens/' + encodeURIComponent(id), { method: 'DELETE' }); await lag(200); ST.tokens = ST.tokens.filter((t) => t.id !== id); audit('revoke api token'); }
export async function setSwap(v) { sample('setSwap');
  if (real('setSwap')) { const mb = v === 'None' ? 0 : Math.round(parseFloat(v) * 1024); const { task } = await http('/system/swap', { method: 'PUT', body: { sizeMb: mb } }); await waitTask(task, { every: 1000 }); return; }
  await lag(400); ST.swap = v; audit(`swap ${v}`); }
export async function growFs() { sample('growFs'); await lag(900); ST.fsGrown = true; audit('grow filesystem'); }
export async function checkOpsboxUpdate() { sample('checkOpsboxUpdate');
  if (real('checkOpsboxUpdate')) return http('/updates/check', { method: 'POST' });
  await lag(250); return { current: '0.1.0', channel: ST.channel, auto: ST.autoUpdate, window: 3, updateAvailable: false, latest: null }; }
export async function applyOpsboxUpdate(version = null) { sample('applyOpsboxUpdate');
  if (real('applyOpsboxUpdate')) return http('/updates/apply', { method: 'POST', body: { version } });
  await lag(500); return { ok: true, version }; }
export async function rollbackOpsboxUpdate(version = null) { sample('rollbackOpsboxUpdate');
  if (real('rollbackOpsboxUpdate')) return http('/updates/rollback', { method: 'POST', body: { version } });
  await lag(500); return { ok: true }; }
export async function setOpsboxAutoUpdate(v) { sample('setOpsboxAutoUpdate');
  const body = typeof v === 'boolean' ? { auto: v } : v;
  if (real('setOpsboxAutoUpdate')) return http('/updates/auto', { method: 'PUT', body });
  await lag(150); if ('auto' in body) ST.autoUpdate = body.auto; if ('channel' in body) ST.channel = body.channel; }
export async function disconnectGithub() { sample('disconnectGithub');
  if (real('disconnectGithub')) return http('/github', { method: 'DELETE' });
  await lag(300); ST.github = { connected: false, installed: false }; audit('disconnect GitHub'); return { ok: true }; }
export async function exportConfig() { sample('exportConfig'); await lag(400); audit('export configuration'); return JSON.stringify({ opsbox: '0.1.0', host: S.host.name, services: S.project.services.map((s) => ({ id: s.id, kind: s.kind, repo: s.repo, branch: s.branch, build: s.build, domain: s.domain })), settings: { tz: ST.tz, panel: ST.panel } }, null, 2); }

// ---------------------------------------------------------------- domains
const DOM = {
  web: [
    { name: 'shop.example.com', primary: true, redirect: '', forceHttps: true, hsts: true, dns: 'ok', cert: 'valid', renews: 61 },
    { name: 'www.shop.example.com', primary: false, redirect: 'shop.example.com', forceHttps: true, hsts: true, dns: 'ok', cert: 'valid', renews: 61 },
    { name: 'shop.acme.co', primary: false, redirect: '', forceHttps: true, hsts: true, dns: 'pending', cert: 'none', custom: true },
  ],
};
const DOMRE = /^(?=.{1,253}$)(\*\.)?([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/;
export async function getDomains(svc) {
  sample('getDomains');
  if (real('getDomains')) return http(`/services/${encodeURIComponent(svc)}/domains`);
  await lag();
  const svcObj = S.project.services.find((s) => s.id === svc);
  const port = svcObj?.port ?? 3000;
  return {
    service: svc,
    project: 'default',
    network: 'opsbox-default',
    port,
    internalHost: svc,
    internalUrl: port > 0 ? `http://${svc}:${port}` : svc,
    proxyReachable: true,
    serverIps: ['203.0.113.10', '2001:db8::10'],
    domains: clone((DOM[svc] || []).map((d) => ({ forceHttps: true, hsts: true, ...d }))),
  };
}
export async function addDomain(svc, d) {
  sample('addDomain');
  if (real('addDomain')) {
    return http(`/services/${encodeURIComponent(svc)}/domains`, {
      method: 'POST',
      body: {
        domain: d.name,
        www: d.www || 'none',
        redirectTo: d.redirectTo || undefined,
        forceHttps: d.force !== false,
        hsts: d.hsts !== undefined ? !!d.hsts : d.force !== false,
      },
    });
  }
  await lag(400); const name = (d.name || '').trim().toLowerCase().replace(/^https?:\/\//, '').replace(/\/.*$/, '');
  if (!DOMRE.test(name)) throw new Error('Enter a domain like app.acme.co (no https://, no path).');
  if (name.startsWith('*.')) throw new Error('Wildcard domains require DNS-01 and are not supported yet.');
  const all = Object.values(DOM).flat(); if (all.some((x) => x.name === name)) throw new Error(`${name} is already used by a service.`);
  const forceHttps = d.force !== false;
  const hsts = d.hsts !== undefined ? !!d.hsts : forceHttps;
  (DOM[svc] = DOM[svc] || []).push({ name, primary: !(DOM[svc] || []).length, redirect: d.redirectTo || '', forceHttps, hsts, dns: 'pending', cert: 'none', custom: true, www: d.www });
  audit(`add domain ${name} to ${svc}`); return { name };
}
export async function checkDns(svc, name) {
  sample('checkDns');
  if (real('checkDns')) return http(`/domains/${encodeURIComponent(name)}/check`, { method: 'POST' });
  await lag(500); const x = (DOM[svc] || []).find((d) => d.name === name); if (!x) throw new Error('No such domain');
  x.checks = (x.checks || 0) + 1; if (x.checks >= 2) { x.dns = 'ok'; }
  return { ...clone(x), pointsHere: x.dns === 'ok', status: x.dns === 'ok' ? 'points_here' : 'waiting' };
}
export async function issueDomainCert(svc, name) {
  sample('issueDomainCert');
  if (real('issueDomainCert')) {
    const { task } = await http(`/domains/${encodeURIComponent(name)}/certificate`, { method: 'POST' });
    const t = await waitTask(task, { every: 1000, max: 3 * 60 * 1000 });
    return t.result;
  }
  await lag(700); const x = (DOM[svc] || []).find((d) => d.name === name); if (!x) throw new Error('No such domain');
  x.cert = 'valid'; x.renews = 90; x.certError = null; audit(`certificate issued for ${name}`);
  return clone(x);
}
export async function updateDomain(svc, name, patch) {
  sample('updateDomain');
  if (real('updateDomain')) return http(`/domains/${encodeURIComponent(name)}`, { method: 'PATCH', body: patch });
  await lag(200); const x = (DOM[svc] || []).find((d) => d.name === name); if (!x) throw new Error('No such domain');
  Object.assign(x, patch); audit(`update domain ${name}`);
  return clone(x);
}
export async function removeDomain(svc, name) {
  sample('removeDomain');
  if (real('removeDomain')) return http(`/domains/${encodeURIComponent(name)}`, { method: 'DELETE' });
  await lag(200); DOM[svc] = (DOM[svc] || []).filter((d) => d.name !== name); audit(`remove domain ${name}`);
}
export async function makePrimary(svc, name) {
  sample('makePrimary');
  if (real('makePrimary')) return http(`/domains/${encodeURIComponent(name)}/primary`, { method: 'POST' });
  await lag(200); (DOM[svc] || []).forEach((d) => { d.primary = d.name === name; d.redirect = d.primary ? '' : d.redirect; }); audit(`primary domain ${name}`);
}

// ---------------------------------------------------------------- backup providers
const MOCK_PROVIDERS = [
  { id: 'r2', name: 'Cloudflare R2', sub: 'Cloudflare', mono: 'R2', sftp: false, endpointTemplate: 'https://{account}.r2.cloudflarestorage.com', fields: [{ key: 'account', label: 'Account ID', hint: 'Cloudflare dashboard › R2 › Overview, right-hand side.' }, { key: 'bucket', label: 'Bucket' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'Access Key ID' }, { key: 'secret', label: 'Secret Access Key', secret: true }], steps: ['Open R2 Object Storage and create a bucket.', 'Manage R2 API Tokens › Create API token.', 'Choose Object Read & Write, limited to that bucket.', 'Copy the Access Key ID and Secret here. The secret is shown only once.'], tip: 'R2 charges nothing to download, so restoring costs only storage.' },
  { id: 's3', name: 'AWS S3', sub: 'Amazon', mono: 'S3', sftp: false, endpointTemplate: 'https://s3.{region}.amazonaws.com', fields: [{ key: 'region', label: 'Region', hint: 'For example ap-southeast-2.' }, { key: 'bucket', label: 'Bucket' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'Access Key ID' }, { key: 'secret', label: 'Secret Access Key', secret: true }], steps: ['Create a bucket in S3 (block public access stays on).', 'In IAM create a user with a policy limited to that bucket.', 'Create an access key for that user.', 'Copy the key and secret here.'] },
  { id: 'b2', name: 'Backblaze B2', sub: 'Backblaze', mono: 'B2', sftp: false, endpointTemplate: 'https://s3.{region}.backblazeb2.com', fields: [{ key: 'region', label: 'Region', hint: 'Shown on the bucket page, e.g. us-west-004.' }, { key: 'bucket', label: 'Bucket' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'Application Key ID' }, { key: 'secret', label: 'Application Key', secret: true }], steps: ['Create a private bucket.', 'App Keys › Add a New Application Key, limited to that bucket.', 'Copy the keyID and applicationKey here.'] },
  { id: 'wasabi', name: 'Wasabi', sub: 'Wasabi', mono: 'WA', sftp: false, endpointTemplate: 'https://s3.{region}.wasabisys.com', fields: [{ key: 'region', label: 'Region' }, { key: 'bucket', label: 'Bucket' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'Access Key' }, { key: 'secret', label: 'Secret Key', secret: true }], steps: ['Create a bucket.', 'Access Keys › Create new access key.', 'Copy both keys here.'] },
  { id: 'do', name: 'DigitalOcean Spaces', sub: 'DigitalOcean', mono: 'DO', sftp: false, endpointTemplate: 'https://{region}.digitaloceanspaces.com', fields: [{ key: 'region', label: 'Region', hint: 'e.g. sgp1' }, { key: 'bucket', label: 'Space name' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'Access Key' }, { key: 'secret', label: 'Secret', secret: true }], steps: ['Create a Space.', 'API › Spaces Keys › Generate New Key.', 'Copy both values here.'] },
  { id: 'hetzner', name: 'Hetzner Storage', sub: 'Hetzner', mono: 'HZ', sftp: false, endpointTemplate: 'https://{region}.your-objectstorage.com', fields: [{ key: 'region', label: 'Location', hint: 'fsn1, nbg1 or hel1' }, { key: 'bucket', label: 'Bucket' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'Access Key' }, { key: 'secret', label: 'Secret Key', secret: true }], steps: ['Create a bucket in Object Storage.', 'Security › S3 credentials › Generate.', 'Copy both values here.'] },
  { id: 'gcs', name: 'Google Cloud Storage', sub: 'Google Cloud', mono: 'GC', sftp: false, endpointTemplate: 'https://storage.googleapis.com', fields: [{ key: 'bucket', label: 'Bucket' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'HMAC Access ID' }, { key: 'secret', label: 'HMAC Secret', secret: true }], steps: ['Create a bucket.', 'Settings › Interoperability › Create a key for a service account.', 'Copy the access ID and secret here.'] },
  { id: 'minio', name: 'MinIO', sub: 'Self-hosted', mono: 'MI', sftp: false, endpointTemplate: '{endpoint}', fields: [{ key: 'endpoint', label: 'Endpoint URL', hint: 'https://minio.example.com:9000' }, { key: 'bucket', label: 'Bucket' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'Access Key' }, { key: 'secret', label: 'Secret Key', secret: true }], steps: ['Create a bucket and a user limited to it.', 'Create an access key for that user.'] },
  { id: 'sftp', name: 'SFTP server', sub: 'Any Linux box over SSH', mono: 'SF', sftp: true, endpointTemplate: 'sftp://{user}@{host}', fields: [{ key: 'host', label: 'Host', hint: 'backup.example.com or an IP' }, { key: 'port', label: 'SSH port', hint: '22', optional: true }, { key: 'user', label: 'User' }, { key: 'path', label: 'Folder on that server', hint: '/srv/backups/srv-sgn-01' }], steps: ['opsbox shows a public key after you save.', 'Add it to ~/.ssh/authorized_keys of that user on the backup server.', 'Run the test again.'] },
  { id: 'custom', name: 'Other S3-compatible', sub: 'Custom endpoint', mono: 'S3', sftp: false, endpointTemplate: '{endpoint}', fields: [{ key: 'endpoint', label: 'Endpoint URL' }, { key: 'region', label: 'Region', optional: true }, { key: 'bucket', label: 'Bucket' }, { key: 'prefix', label: 'Folder in bucket', optional: true }, { key: 'key', label: 'Access Key' }, { key: 'secret', label: 'Secret Key', secret: true }], steps: ['Use the S3 endpoint your provider documents.', 'Create a key limited to one bucket.'] },
];
export async function getProviders() { sample('getProviders');
  if (real('getProviders')) return http('/backups/providers');
  await lag(50);
  return { providers: clone(MOCK_PROVIDERS), hostname: 'srv-sgn-01' };
}
export async function testDestination(pid, v) { sample('testDestination');
  if (real('testDestination')) {
    const r = await http('/backups/destinations/test', { method: 'POST', body: { provider: pid, values: v } });
    return { ...r, steps: (r.steps || []).map((s) => (Array.isArray(s) ? s : [s.status, s.message])) };
  }
  await lag(1100); const p = MOCK_PROVIDERS.find((x) => x.id === pid); if (!p) throw new Error('Choose a provider');
  const miss = p.fields.filter((f) => !f.optional && !(v[f.key] || '').trim()).map((f) => f.label);
  if (miss.length) return { ok: false, steps: [['fail', `Missing: ${miss.join(', ')}`]] };
  if (v.bucket && !/^[a-z0-9][a-z0-9.-]{1,61}[a-z0-9]$/.test(v.bucket)) return { ok: false, steps: [['ok', 'Connected to endpoint'], ['fail', 'Bucket names use lowercase letters, numbers, dots and dashes.']] };
  if (p.sftp) return { ok: true, steps: [['ok', `Reached ${v.host}:${v.port || 22}`], ['warn', 'Save to view the public key, add it to authorized_keys, then test again.']] };
  return { ok: true, steps: [['ok', 'Connected to endpoint'], ['ok', `Bucket ${v.bucket} found`], ['ok', 'Wrote encrypted probe and read it back'], ['ok', 'Verified delete permission']] };
}
export async function saveDestination(pid, v, name) { sample('saveDestination');
  if (real('saveDestination')) {
    return http('/backups/destinations', { method: 'POST', body: { provider: pid, name: (name || '').trim() || undefined, values: v, verify: true } });
  }
  await lag(400); const p = MOCK_PROVIDERS.find((x) => x.id === pid);
  const loc = p.sftp ? `sftp://${v.user}@${v.host}:${v.path}` : `${(v.endpoint || p.endpointTemplate).replace('{account}', v.account || '').replace('{region}', v.region || '').replace(/^https?:\/\//, '')}/${v.bucket}${v.prefix ? '/' + v.prefix : ''}`;
  const rec = { id: 'd' + Date.now(), name: (name || '').trim() || `Offsite (${p.name})`, type: p.name, loc, ok: !p.sftp, isDefault: !p.sftp, ...(p.sftp ? { publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMockKey opsbox-backup' } : {}) };
  BK.dests.unshift(rec);
  BK.jobs.forEach((j) => { if (j.dest === 'Local disk') { j.dest = rec.name; j.state = 'ok'; j.last = 'next run 02:40'; } });
  audit(`add backup destination ${p.name}`);
  return { destination: rec, ...(p.sftp ? { publicKey: rec.publicKey } : {}) };
}

// ---------------------------------------------------------------- proxy network
export async function getProxyNetwork() {
  if (real('getProxyNetwork')) return http('/network/proxy'); // GET /api/v1/network/proxy
  sample('getProxyNetwork'); return { network: null, undo: [] };
}
export async function takeProxyPort(port, target, confirm = false) {
  if (real('takeProxyPort')) return http('/network/proxy/takeover', { method: 'POST', body: { port, target, confirm } }); // POST /api/v1/network/proxy/takeover
  sample('takeProxyPort'); throw new Error('Port changes require a connected server.');
}
export async function undoProxyTakeover(id, confirm = false) {
  if (real('undoProxyTakeover')) return http('/network/proxy/undo', { method: 'POST', body: { id, confirm } }); // POST /api/v1/network/proxy/undo
  sample('undoProxyTakeover'); throw new Error('Port changes require a connected server.');
}
export async function keepProxyPorts(confirm = false) {
  if (real('keepProxyPorts')) return http('/network/proxy/keep', { method: 'POST', body: { confirm } }); // POST /api/v1/network/proxy/keep
  sample('keepProxyPorts'); throw new Error('Port changes require a connected server.');
}
