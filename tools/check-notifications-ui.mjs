#!/usr/bin/env node
// Real headless-Chromium + API-contract verification for prompt 16f:
// - Notification bell API / CSRF contracts & zero mock leakage
// - Shared single /api/v1/attention poll between header bell and Overview
// - Bell badge, critical pulse, one-shot ring, polite critical announce, popover keyboard nav,
//   Mark read / Mark all read / Snooze (including critical-snooze-count invariant), mobile sheet
// - Overview critical-only thin banner + "and N more"
// (data-flip timing / reduced-motion / long-move skip live in tools/check-flip-ui.mjs)

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import http from 'node:http';
import { homedir, tmpdir } from 'node:os';
import { extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)));
const WEB_DIR = join(ROOT, 'web');

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.woff2': 'font/woff2',
  '.json': 'application/json',
};

function findChrome() {
  if (process.env.OPSBOX_CHROME && existsSync(process.env.OPSBOX_CHROME)) return process.env.OPSBOX_CHROME;
  const cacheDir = join(homedir(), 'Library/Caches/ms-playwright');
  if (existsSync(cacheDir)) {
    const dirs = readdirSync(cacheDir).filter((d) => d.startsWith('chromium-')).sort();
    for (const d of dirs.reverse()) {
      const macApp = join(cacheDir, d, 'chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing');
      if (existsSync(macApp)) return macApp;
      const macX64 = join(cacheDir, d, 'chrome-mac/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing');
      if (existsSync(macX64)) return macX64;
      const linux = join(cacheDir, d, 'chrome-linux/chrome');
      if (existsSync(linux)) return linux;
    }
  }
  const candidates = [
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/usr/bin/chromium',
    '/usr/bin/chromium-browser',
    '/usr/bin/google-chrome',
  ];
  for (const c of candidates) if (existsSync(c)) return c;
  return null;
}

// ---------- 1. API contract & CSRF check in Node ----------
async function checkApiContracts() {
  const prevLoc = globalThis.location;
  const prevDoc = globalThis.document;
  const prevFetch = globalThis.fetch;
  const calls = [];
  globalThis.location = { search: '?mode=http', protocol: 'https:', port: '8443' };
  globalThis.document = { cookie: 'opsbox_csrf=csrf-16f' };
  globalThis.fetch = async (url, options = {}) => {
    calls.push({ url, method: options.method || 'GET', headers: options.headers || {}, body: options.body ? JSON.parse(options.body) : undefined });
    return { ok: true, json: async () => ({ ok: true, items: [], unread: 0, critical: 0 }) };
  };
  try {
    const source = await readFile(join(WEB_DIR, 'js/api.js'), 'utf8');
    const api = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
    for (const fn of ['getAttention', 'markAttentionRead', 'markAllAttentionRead', 'snoozeAttention']) {
      assert.equal(api.isReal(fn), true, `${fn} should be in REAL`);
    }
    assert.deepEqual(api.usageFor('overview').sample, []);
    await api.getAttention();
    await api.markAttentionRead('disk:/');
    await api.snoozeAttention('backup:opsbox', 60);
    await api.markAllAttentionRead();
    assert.deepEqual(calls.map((c) => [c.method, c.url]), [
      ['GET', '/api/v1/attention'],
      ['POST', '/api/v1/attention/disk%3A%2F/read'],
      ['POST', '/api/v1/attention/backup%3Aopsbox/snooze'],
      ['POST', '/api/v1/attention/read-all'],
    ]);
    assert.deepEqual(calls[2].body, { minutes: 60 });
    for (const c of calls.slice(1)) assert.equal(c.headers['X-Opsbox-CSRF'], 'csrf-16f');
  } finally {
    globalThis.location = prevLoc;
    globalThis.document = prevDoc;
    globalThis.fetch = prevFetch;
  }
}

// ---------- 2. Minimal CDP client over Node built-in WebSocket ----------
async function launchBrowser() {
  const exe = findChrome();
  assert(exe, 'No Chrome/Chromium binary found for headless UI verification');
  const userDataDir = mkdtempSync(join(tmpdir(), 'opsbox-cdp-'));
  const proc = spawn(exe, [
    '--headless=new',
    '--disable-gpu',
    '--no-sandbox',
    '--no-first-run',
    '--no-default-browser-check',
    '--remote-debugging-port=0',
    `--user-data-dir=${userDataDir}`,
    'about:blank',
  ], { stdio: ['ignore', 'pipe', 'pipe'] });

  const wsUrl = await new Promise((resolveUrl, reject) => {
    let stderr = '';
    const timer = setTimeout(() => reject(new Error(`Timed out waiting for Chromium CDP: ${stderr}`)), 15000);
    const onChunk = (chunk) => {
      stderr += chunk.toString();
      const m = stderr.match(/DevTools listening on (ws:\/\/\S+)/);
      if (m) {
        clearTimeout(timer);
        resolveUrl(m[1]);
      }
    };
    proc.stderr.on('data', onChunk);
    proc.stdout.on('data', onChunk);
    proc.once('exit', (code) => {
      clearTimeout(timer);
      reject(new Error(`Chromium exited (${code}): ${stderr}`));
    });
  });

  const targetRes = await fetch(wsUrl.replace(/^ws:/, 'http:').replace(/\/devtools\/browser\/.*$/, '/json/list'));
  const targets = await targetRes.json();
  const pageTarget = targets.find((t) => t.type === 'page') || targets[0];
  assert(pageTarget?.webSocketDebuggerUrl, 'Missing page debugger URL');

  const ws = new WebSocket(pageTarget.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  let seq = 0;
  const pending = new Map();
  const events = [];
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id) {
      const p = pending.get(msg.id);
      if (p) {
        pending.delete(msg.id);
        if (msg.error) p.rej(new Error(msg.error.message || JSON.stringify(msg.error)));
        else p.res(msg.result);
      }
    } else {
      events.push(msg);
    }
  };
  const send = (method, params = {}) => new Promise((res, rej) => {
    const id = ++seq;
    pending.set(id, { res, rej });
    ws.send(JSON.stringify({ id, method, params }));
  });
  const evalJs = async (expression) => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text || 'JS error');
    return r.result?.value;
  };
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });

  return {
    send,
    evalJs,
    events,
    async close() {
      try { ws.close(); } catch {}
      if (proc.exitCode === null) {
        const exited = new Promise((res) => proc.once('exit', res));
        try { proc.kill('SIGKILL'); } catch {}
        await Promise.race([exited, new Promise((res) => setTimeout(res, 1500))]);
      }
      try { rmSync(userDataDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 }); } catch {}
    },
  };
}

// ---------- 3. Local test HTTP server serving web/ + stateful /api/v1/* ----------
function startTestServer() {
  const nowSec = Math.floor(Date.now() / 1000);
  const state = {
    attentionGets: 0,
    readCalls: [],
    readAllCalls: 0,
    snoozeCalls: [],
    items: [
      { id: 'security:host', kind: 'security', severity: 'warning', since: nowSec - 300, text: '2 security updates are waiting', link: ['Review and install →', '#/system'], read: false, snoozed: false },
      { id: 'backup:opsbox', kind: 'backup', severity: 'warning', since: nowSec - 600, text: 'Scheduled backup missed for opsbox (daily 02:00)', link: ['View backups →', '#/backups'], read: false, snoozed: false },
    ],
  };

  function computeAttention() {
    const visible = state.items.filter((i) => !(i.snoozed && i.severity !== 'critical'));
    visible.sort((a, b) => ((a.severity !== 'critical') - (b.severity !== 'critical')) || (a.snoozed - b.snoozed) || (b.since - a.since));
    return {
      items: visible,
      unread: visible.filter((i) => !i.read).length,
      critical: visible.filter((i) => i.severity === 'critical' && !i.read).length,
    };
  }

  const server = http.createServer(async (req, res) => {
    const u = new URL(req.url, 'http://127.0.0.1');
    const json = (code, obj, extraHeaders = {}) => {
      res.writeHead(code, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...extraHeaders });
      res.end(JSON.stringify(obj));
    };

    if (u.pathname.startsWith('/api/v1/')) {
      let body = '';
      for await (const chunk of req) body += chunk;
      const parsed = body ? JSON.parse(body) : null;

      if (u.pathname === '/api/v1/session') {
        return json(200, { authenticated: true, user: 'admin', role: 'admin', enrollRequired: false, setupRequired: false }, { 'Set-Cookie': 'opsbox_csrf=test-csrf; Path=/' });
      }
      if (u.pathname === '/api/v1/host') {
        return json(200, { name: 'srv-test-01', os: 'Ubuntu 24.04 LTS', kernel: '6.8.0', cores: 4, memTotal: 8.0, diskTotal: 100.0, upDays: 5, version: '0.1.0' });
      }
      if (u.pathname === '/api/v1/metrics') {
        return json(200, {
          cpu: Array(24).fill(18), mem: Array(24).fill(42), cpuNow: 18.2, cores: 4,
          memUsed: 3.4, memTotal: 8.0, cache: 1.2, swapUsed: 0, swapTotal: 0,
          disk: 42.0, diskTotal: 100.0, netIn: 1.2, netOut: 0.4, conns: 18, load: [0.2, 0.3, 0.25],
          procs: [
            { name: 'postgres', pid: 101, cpu: 12.1, mem: 420, self: false },
            { name: 'nginx', pid: 102, cpu: 5.4, mem: 48, self: false },
            { name: 'node', pid: 103, cpu: 3.1, mem: 160, self: false },
            { name: 'redis', pid: 104, cpu: 1.2, mem: 32, self: false },
            { name: 'opsbox', pid: 105, cpu: 0.2, mem: 14, self: true },
          ],
          containers: [],
        });
      }
      if (u.pathname === '/api/v1/attention' && req.method === 'GET') {
        state.attentionGets += 1;
        return json(200, computeAttention());
      }
      if (u.pathname === '/api/v1/attention/read-all' && req.method === 'POST') {
        assert.equal(req.headers['x-opsbox-csrf'], 'test-csrf');
        state.readAllCalls += 1;
        for (const it of state.items) if (!(it.snoozed && it.severity !== 'critical')) it.read = true;
        return json(200, { ok: true, ...computeAttention() });
      }
      const readMatch = u.pathname.match(/^\/api\/v1\/attention\/([^/]+)\/read$/);
      if (readMatch && req.method === 'POST') {
        assert.equal(req.headers['x-opsbox-csrf'], 'test-csrf');
        const id = decodeURIComponent(readMatch[1]);
        state.readCalls.push(id);
        const it = state.items.find((x) => x.id === id);
        if (!it) return json(404, { error: { code: 'not_found', message: 'No such notification.' } });
        it.read = true;
        return json(200, { ok: true, ...computeAttention() });
      }
      const snoozeMatch = u.pathname.match(/^\/api\/v1\/attention\/([^/]+)\/snooze$/);
      if (snoozeMatch && req.method === 'POST') {
        assert.equal(req.headers['x-opsbox-csrf'], 'test-csrf');
        const id = decodeURIComponent(snoozeMatch[1]);
        state.snoozeCalls.push({ id, minutes: parsed?.minutes });
        const it = state.items.find((x) => x.id === id);
        if (!it) return json(404, { error: { code: 'not_found', message: 'No such notification.' } });
        it.snoozed = true;
        return json(200, { ok: true, ...computeAttention() });
      }
      if (u.pathname === '/api/v1/capacity') {
        return json(200, { profile: { memory_mb: 8192, swap_mb: 0, cpu_millis: 4000 }, allocatable: { os_mb: 256, opsbox_mb: 128, docker_mb: 150, swap_credit_mb: 0, allocatable_mb: 7000, database_allocatable_mb: 7000 }, reservations: [], features: [], pressure: false, swap_offer_mb: null });
      }
      if (u.pathname === '/api/v1/network/proxy') {
        return json(200, { network: { running: true, port_conflict: false, holders: [] }, undo: [] });
      }
      if (u.pathname === '/api/v1/docker/containers') {
        return json(200, [
          { id: 'c1', name: 'web', state: 'running', since: nowSec - 1000 },
          { id: 'c2', name: 'api', state: 'running', since: nowSec - 2000 },
          { id: 'c3', name: 'db', state: 'running', since: nowSec - 3000 },
          { id: 'c4', name: 'cache', state: 'running', since: nowSec - 4000 },
          { id: 'c5', name: 'worker', state: 'running', since: nowSec - 5000 },
        ]);
      }
      if (u.pathname === '/api/v1/cleanup/scan') {
        return json(200, { pending: false, items: [{ id: 'logs', bytes: 256 * 1024 * 1024 }] });
      }
      if (u.pathname === '/api/v1/security') {
        return json(200, { checks: [], users: [], audit: [] });
      }
      return json(404, { error: { code: 'not_found', message: 'not found' } });
    }

    const rel = u.pathname === '/' ? 'index.html' : u.pathname.replace(/^\/+/, '');
    const filePath = resolve(WEB_DIR, rel);
    if (!filePath.startsWith(WEB_DIR)) { res.writeHead(403); return res.end(); }
    try {
      const buf = await readFile(filePath);
      res.writeHead(200, { 'Content-Type': MIME[extname(filePath)] || 'application/octet-stream' });
      res.end(buf);
    } catch {
      res.writeHead(404);
      res.end('Not found');
    }
  });

  return new Promise((resolveSrv) => {
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      resolveSrv({ server, base: `http://127.0.0.1:${port}`, state, nowSec });
    });
  });
}

const waitFor = async (evalJs, predExpr, msg, timeoutMs = 6000) => {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    if (await evalJs(`Boolean(${predExpr})`)) return;
    await new Promise((r) => setTimeout(r, 40));
  }
  throw new Error(`Timed out waiting for: ${msg}`);
};

async function runBrowserChecks() {
  const { server, base, state, nowSec } = await startTestServer();
  const browser = await launchBrowser();
  const errors = [];
  const origOnMsg = browser;
  try {
    // Capture runtime exceptions
    await browser.send('Page.addScriptToEvaluateOnNewDocument', {
      source: `window.__testErrors = []; window.addEventListener('error', (e) => window.__testErrors.push(e.message)); window.addEventListener('unhandledrejection', (e) => window.__testErrors.push(String(e.reason)));`,
    });

    // A. Overview + shared single poll + warnings-only state
    await browser.send('Page.navigate', { url: `${base}/?mode=http#/` });
    await waitFor(browser.evalJs, `document.getElementById('notif-btn') && document.getElementById('kpis')`, 'Overview and bell mount');
    await new Promise((r) => setTimeout(r, 200));

    assert.equal(state.attentionGets, 1, `Expected single shared GET /api/v1/attention poll, got ${state.attentionGets}`);
    const initCheck = await browser.evalJs(`({
      aria: document.getElementById('notif-btn').getAttribute('aria-label'),
      badgeText: document.getElementById('notif-badge').textContent,
      badgeHidden: document.getElementById('notif-badge').hidden,
      badgeCrit: document.getElementById('notif-badge').classList.contains('crit'),
      critBanner: document.getElementById('crit-banner').innerHTML.trim(),
      hasNeedsCard: document.body.textContent.includes('Needs attention'),
    })`);
    assert.equal(initCheck.aria, 'Notifications, 2 unread');
    assert.equal(initCheck.badgeText, '2');
    assert.equal(initCheck.badgeHidden, false);
    assert.equal(initCheck.badgeCrit, false);
    assert.equal(initCheck.critBanner, '', 'Warnings only must NOT render Overview critical banner');
    assert.equal(initCheck.hasNeedsCard, false, 'Old Needs attention card must be removed');

    // B. Critical items arrive -> one-shot ring + badge pop + danger pulse + announce + Overview banner
    state.items.unshift(
      { id: 'disk:/', kind: 'disk', severity: 'critical', since: nowSec - 30, text: 'Disk / is at 96%.', link: ['Clean up →', '#/cleanup'], read: false, snoozed: false },
      { id: 'cpu:host', kind: 'cpu', severity: 'critical', since: nowSec - 60, text: 'CPU is at 99%. Review busy processes.', link: ['View alert →', '#/alerts?id=cpu%3Ahost'], read: false, snoozed: false },
    );
    await browser.evalJs(`document.dispatchEvent(new Event('visibilitychange'))`);
    await waitFor(browser.evalJs, `document.getElementById('notif-badge').textContent === '4'`, 'Bell updated to 4 unread');
    await new Promise((r) => setTimeout(r, 100));

    const critCheck = await browser.evalJs(`({
      ring: document.getElementById('notif-btn').classList.contains('ring'),
      crit: document.getElementById('notif-badge').classList.contains('crit'),
      bannerText: document.querySelector('#crit-banner .crit-txt')?.textContent,
      bannerLink: document.querySelector('#crit-banner a')?.getAttribute('href'),
      bannerMore: document.querySelector('#crit-banner [data-act="open-bell"]')?.textContent,
      srLive: document.getElementById('sr-live')?.textContent,
    })`);
    assert.equal(critCheck.ring, true, 'Bell must wiggle once on new arrival');
    assert.equal(critCheck.crit, true, 'Badge must turn danger + pulse for unread critical');
    assert.equal(critCheck.bannerText, 'Disk / is at 96%.');
    assert.equal(critCheck.bannerLink, '#/cleanup');
    assert.equal(critCheck.bannerMore, 'and 1 more');
    assert(critCheck.srLive.includes('Disk / is at 96%'), `Expected polite announcement for critical item, got: ${critCheck.srLive}`);

    // Click "and 1 more" opens bell popover & moves focus inside
    await browser.evalJs(`document.querySelector('#crit-banner [data-act="open-bell"]').click()`);
    await waitFor(browser.evalJs, `!document.getElementById('notif-pop').hidden`, 'Popover opened from Overview banner');
    assert.equal(await browser.evalJs(`document.getElementById('notif-pop').contains(document.activeElement)`), true, 'Focus must move inside dialog');

    // Arrow key moves between items, Escape closes and returns focus to bell
    await browser.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'ArrowDown', code: 'ArrowDown', windowsVirtualKeyCode: 40 });
    assert.equal(await browser.evalJs(`document.activeElement?.dataset?.id`), 'cpu:host');
    await browser.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'ArrowUp', code: 'ArrowUp', windowsVirtualKeyCode: 38 });
    assert.equal(await browser.evalJs(`document.activeElement?.dataset?.id`), 'disk:/');
    await browser.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
    assert.equal(await browser.evalJs(`document.getElementById('notif-pop').hidden`), true);
    assert.equal(await browser.evalJs(`document.activeElement?.id`), 'notif-btn');

    // Re-open via bell click
    await browser.evalJs(`document.getElementById('notif-btn').click()`);
    await waitFor(browser.evalJs, `!document.getElementById('notif-pop').hidden`, 'Popover opened from bell');

    // Snooze warning backup:opsbox -> leaves list & unread drops from 4 to 3
    await browser.evalJs(`(() => {
      const row = [...document.querySelectorAll('.notif-item')].find((r) => r.dataset.id === 'backup:opsbox');
      row.querySelector('[data-nact="menu"]').click();
      row.querySelector('[data-nact="snooze"][data-min="60"]').click();
    })()`);
    await waitFor(browser.evalJs, `document.getElementById('notif-badge').textContent === '3'`, 'Snoozing warning reduced unread to 3');
    assert.equal(await browser.evalJs(`[...document.querySelectorAll('.notif-item')].some((r) => r.dataset.id === 'backup:opsbox')`), false);

    // Snooze critical cpu:host -> stays on top (dimmed .snoozed, below unsnoozed disk:/), unread stays 3, critical stays 2!
    await browser.evalJs(`(() => {
      const row = [...document.querySelectorAll('.notif-item')].find((r) => r.dataset.id === 'cpu:host');
      row.querySelector('[data-nact="menu"]').click();
      row.querySelector('[data-nact="snooze"][data-min="60"]').click();
    })()`);
    await new Promise((r) => setTimeout(r, 120));
    const afterCritSnooze = await browser.evalJs(`({
      unread: document.getElementById('notif-badge').textContent,
      ids: [...document.querySelectorAll('.notif-item')].map((r) => r.dataset.id),
      cpuDimmed: [...document.querySelectorAll('.notif-item')].find((r) => r.dataset.id === 'cpu:host')?.classList.contains('snoozed'),
    })`);
    assert.equal(afterCritSnooze.unread, '3');
    assert.deepEqual(afterCritSnooze.ids, ['disk:/', 'cpu:host', 'security:host']);
    assert.equal(afterCritSnooze.cpuDimmed, true);

    // Mark disk:/ read -> unread drops to 2, Overview banner switches to cpu:host without "and N more"
    await browser.evalJs(`(() => {
      const row = [...document.querySelectorAll('.notif-item')].find((r) => r.dataset.id === 'disk:/');
      row.querySelector('[data-nact="menu"]').click();
      row.querySelector('[data-nact="read"]').click();
    })()`);
    await waitFor(browser.evalJs, `document.getElementById('notif-badge').textContent === '2'`, 'Mark read reduced unread to 2');
    assert.equal(await browser.evalJs(`document.querySelector('#crit-banner .crit-txt')?.textContent`), 'CPU is at 99%. Review busy processes.');
    assert.equal(await browser.evalJs(`document.querySelector('#crit-banner [data-act="open-bell"]')`), null);

    // Mark all read -> badge hides, crit class removed, Overview banner clears
    await browser.evalJs(`document.getElementById('notif-read-all').click()`);
    await waitFor(browser.evalJs, `document.getElementById('notif-badge').hidden`, 'Badge hidden after Mark all read');
    assert.equal(await browser.evalJs(`document.getElementById('notif-badge').classList.contains('crit')`), false);
    assert.equal(await browser.evalJs(`document.getElementById('crit-banner').innerHTML.trim()`), '');

    // Outside click closes popover
    await browser.evalJs(`document.querySelector('main h1').click()`);
    assert.equal(await browser.evalJs(`document.getElementById('notif-pop').hidden`), true);

    // >9 badge displays "9+"
    state.items = Array.from({ length: 11 }, (_, i) => ({
      id: `disk:/m${i}`, kind: 'disk', severity: 'warning', since: nowSec - i * 10,
      text: `Disk /m${i} is at 80%.`, link: ['Clean up →', '#/cleanup'], read: false, snoozed: false,
    }));
    await browser.evalJs(`document.dispatchEvent(new Event('visibilitychange'))`);
    await waitFor(browser.evalJs, `document.getElementById('notif-badge').textContent === '9+'`, 'Badge caps at 9+');

    // Mobile 390px full-width top sheet
    await browser.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
    await browser.evalJs(`document.getElementById('notif-btn').click()`);
    await waitFor(browser.evalJs, `!document.getElementById('notif-pop').hidden`, 'Mobile popover opened');
    await new Promise((r) => setTimeout(r, 260));
    const sheetRect = await browser.evalJs(`(() => { const r = document.getElementById('notif-pop').getBoundingClientRect(); return { left: r.left, top: r.top, width: r.width, height: r.height }; })()`);
    assert.equal(sheetRect.left, 0);
    assert.equal(sheetRect.top, 0);
    assert(sheetRect.width >= 388, `Expected full-width mobile sheet, got ${sheetRect.width}`);
    assert(sheetRect.height > 150, `Expected visible height on mobile sheet, got ${sheetRect.height}`);
    assert.deepEqual(await browser.evalJs(`window.__testErrors`), []);
    await browser.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });

    assert.deepEqual(await browser.evalJs(`window.__testErrors`), [], `Unexpected page errors: ${errors.join(', ')}`);
  } finally {
    await browser.close();
    server.close();
  }
}

await checkApiContracts();
await runBrowserChecks();
console.log('Notification bell UI, shared poll, keyboard nav, and Overview banner checks passed.');