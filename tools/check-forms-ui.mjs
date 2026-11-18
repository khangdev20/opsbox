#!/usr/bin/env node
// Headless-Chromium check for prompt 03f:
// - a failed submit keeps every typed value (name, override box, confirmation sentence) on the
//   template form, the New app form and the memory dialog, and moves focus to the error line;
// - the helper banner shows when /readyz says the helper is down or a different build, and
//   clears by itself when it is back.
// Needs Chrome/Chromium: set OPSBOX_CHROME, or have Google Chrome installed.

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import http from 'node:http';
import { tmpdir } from 'node:os';
import { extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const WEB_DIR = join(resolve(fileURLToPath(new URL('..', import.meta.url))), 'web');
const MIME = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2', '.json': 'application/json' };
const SENTENCE = 'I understand this server may run out of memory';
const DOWN_MESSAGE = "OpsBox's helper is not running. Restart it with `sudo systemctl restart opsbox-priv`.";

function findChrome() {
  const candidates = [process.env.OPSBOX_CHROME, '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome'];
  return candidates.find((c) => c && existsSync(c)) || null;
}

function startServer() {
  const state = { posts: { tpl: [], svc: [], patch: [] }, ready: { status: 'ready', checks: { database: 'ok', privileged_helper: 'ok' } } };
  const failFirst = (list, req) => list.length === 1; // the first call of each kind is refused
  const server = http.createServer(async (req, res) => {
    const u = new URL(req.url, 'http://127.0.0.1');
    const json = (code, obj, headers = {}) => { res.writeHead(code, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store', ...headers }); res.end(JSON.stringify(obj)); };
    if (u.pathname === '/readyz') return json(state.ready.status === 'ready' ? 200 : 503, state.ready);
    if (u.pathname.startsWith('/api/v1/')) {
      let body = '';
      for await (const chunk of req) body += chunk;
      const parsed = body ? JSON.parse(body) : null;
      const p = u.pathname.slice('/api/v1'.length);
      const refuse = () => json(503, { error: { code: 'unavailable', message: DOWN_MESSAGE } });
      if (p === '/session') return json(200, { authenticated: true, user: 'admin', role: 'admin', enrollRequired: false, setupRequired: false }, { 'Set-Cookie': 'opsbox_csrf=test-csrf; Path=/' });
      if (p === '/host') return json(200, { name: 'srv-test-01', os: 'Ubuntu', kernel: '6.8', cores: 4, memTotal: 8, diskTotal: 100, upDays: 5, version: '0.1.0' });
      if (p === '/metrics') return json(200, { cpu: [1], mem: [1], cpuNow: 1, cores: 4, memUsed: 1, memTotal: 8, cache: 0, swapUsed: 0, swapTotal: 0, disk: 10, diskTotal: 100, netIn: 0, netOut: 0, conns: 1, load: [0, 0, 0], procs: [], containers: [] });
      if (p === '/attention') return json(200, { items: [], unread: 0, critical: 0 });
      if (p === '/capacity/check') return json(200, { fit: { status: 'ok', reasons: [], suggestions: [] } });
      if (p === '/templates') return json(200, { databases: [{ id: 'postgres', name: 'PostgreSQL', description: 'Relational database', resources: '100 MB', versions: ['16.4'], port: 5432, category: 'databases', min_memory_mb: 256 }], tools: [] });
      if (p.startsWith('/projects/')) return json(200, { services: [] });
      if (p === '/templates/postgres/instances' && req.method === 'POST') {
        state.posts.tpl.push(parsed);
        return failFirst(state.posts.tpl) ? refuse() : json(200, { id: 'pg-main' });
      }
      if (p === '/services' && req.method === 'POST') {
        state.posts.svc.push(parsed);
        return failFirst(state.posts.svc) ? refuse() : json(200, { deployId: 'd1' });
      }
      if (p.startsWith('/services/') && req.method === 'PATCH') {
        state.posts.patch.push(parsed);
        return failFirst(state.posts.patch) ? refuse() : json(200, { ok: true });
      }
      return json(404, { error: { code: 'not_found', message: 'not found' } });
    }
    const filePath = resolve(WEB_DIR, u.pathname === '/' ? 'index.html' : u.pathname.replace(/^\/+/, ''));
    if (!filePath.startsWith(WEB_DIR)) { res.writeHead(403); return res.end(); }
    try { const buf = await readFile(filePath); res.writeHead(200, { 'Content-Type': MIME[extname(filePath)] || 'application/octet-stream' }); res.end(buf); } catch { res.writeHead(404); res.end('Not found'); }
  });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok({ server, state, base: `http://127.0.0.1:${server.address().port}` })));
}

async function launch() {
  const exe = findChrome();
  assert(exe, 'No Chrome/Chromium binary found (set OPSBOX_CHROME)');
  const dir = mkdtempSync(join(tmpdir(), 'opsbox-forms-'));
  const proc = spawn(exe, ['--headless=new', '--disable-gpu', '--no-sandbox', '--no-first-run', '--remote-debugging-port=0', `--user-data-dir=${dir}`, 'about:blank'], { stdio: ['ignore', 'pipe', 'pipe'] });
  const wsUrl = await new Promise((ok, bad) => {
    let log = '';
    const timer = setTimeout(() => bad(new Error(`Timed out waiting for Chromium: ${log}`)), 15000);
    const onChunk = (c) => { log += c; const m = log.match(/DevTools listening on (ws:\/\/\S+)/); if (m) { clearTimeout(timer); ok(m[1]); } };
    proc.stderr.on('data', onChunk); proc.stdout.on('data', onChunk);
  });
  const targets = await (await fetch(wsUrl.replace(/^ws:/, 'http:').replace(/\/devtools\/browser\/.*$/, '/json/list'))).json();
  const ws = new WebSocket((targets.find((t) => t.type === 'page') || targets[0]).webSocketDebuggerUrl);
  await new Promise((ok, bad) => { ws.onopen = ok; ws.onerror = bad; });
  let seq = 0;
  const pending = new Map();
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    const p = msg.id && pending.get(msg.id);
    if (p) { pending.delete(msg.id); if (msg.error) p.bad(new Error(msg.error.message)); else p.ok(msg.result); }
  };
  const send = (method, params = {}) => new Promise((ok, bad) => { const id = ++seq; pending.set(id, { ok, bad }); ws.send(JSON.stringify({ id, method, params })); });
  const evalJs = async (expression) => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description || r.exceptionDetails.text);
    return r.result?.value;
  };
  await send('Page.enable'); await send('Runtime.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
  return { send, evalJs, async close() { try { ws.close(); } catch { /* closed */ } try { proc.kill('SIGKILL'); } catch { /* gone */ } rmSync(dir, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 }); } };
}

const waitFor = async (evalJs, expr, what, ms = 8000) => {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) { if (await evalJs(`Boolean(${expr})`)) return; await new Promise((r) => setTimeout(r, 40)); }
  throw new Error(`Timed out waiting for: ${what}`);
};

// Types into a control the way a person does (value + input event).
const TYPE = `window.__type = (sel, v) => { const el = document.querySelector(sel); el.value = v; el.dispatchEvent(new Event('input', { bubbles: true })); el.dispatchEvent(new Event('change', { bubbles: true })); };
window.__tick = (sel) => { const el = document.querySelector(sel); el.checked = true; el.dispatchEvent(new Event('input', { bubbles: true })); };`;

async function main() {
  const { server, state, base } = await startServer();
  const b = await launch();
  const { evalJs } = b;
  try {
    await b.send('Page.navigate', { url: `${base}/?mode=http#/apps/new/templates` });
    await waitFor(evalJs, `document.querySelector('#fm [data-act="add"]')`, 'template form');
    await evalJs(TYPE);

    // ---- template form: fill, tick override, type the sentence, submit, get refused
    await waitFor(evalJs, `document.querySelector('[data-capacity-result]')?.textContent === 'Fits'`, 'capacity result');
    await evalJs(`__type('#tn', 'my-pg'); __type('#td', '5 GB'); __type('#cap-mem', '512'); __tick('[data-capacity-override]'); __type('[data-capacity-confirmation]', ${JSON.stringify(SENTENCE)});
      document.querySelector('#tb').checked = false; document.querySelector('#tb').dispatchEvent(new Event('input', { bubbles: true }));`);
    await evalJs(`document.querySelector('#fm [data-act="add"]').click()`);
    await waitFor(evalJs, `document.querySelector('#err')?.textContent.includes('helper is not running')`, 'error line');
    const kept = await evalJs(`JSON.stringify({ name: document.querySelector('#tn').value, disk: document.querySelector('#td').value, mem: document.querySelector('#cap-mem').value,
      override: document.querySelector('[data-capacity-override]').checked, sentence: document.querySelector('[data-capacity-confirmation]').value,
      backup: document.querySelector('#tb').checked, focus: document.activeElement?.id, alert: document.querySelector('#err').getAttribute('role'),
      label: document.querySelector('#fm [data-act="add"]').textContent })`);
    assert.deepEqual(JSON.parse(kept), { name: 'my-pg', disk: '5 GB', mem: '512', override: true, sentence: SENTENCE, backup: false, focus: 'err', alert: 'alert', label: 'Add PostgreSQL' });
    // retrying works without retyping anything
    await waitFor(evalJs, `!document.querySelector('#fm [data-act="add"]').disabled`, 'button usable again');
    await evalJs(`document.querySelector('#fm [data-act="add"]').click()`);
    await waitFor(evalJs, `location.hash.startsWith('#/apps?s=pg-main')`, 'success navigation');
    assert.equal(state.posts.tpl.length, 2);
    assert.deepEqual(state.posts.tpl[1].name, 'my-pg');
    assert.equal(state.posts.tpl[1].backup, false);

    // ---- New app form
    await evalJs(`location.hash = '#/apps/new/configure?repo=acme/shop'`);
    await waitFor(evalJs, `document.querySelector('#f #name')`, 'configure form');
    await waitFor(evalJs, `document.querySelector('[data-capacity-result]')?.textContent === 'Fits'`, 'configure capacity');
    await evalJs(`__type('#name', 'shop-web'); __type('#port', '8080'); __type('#vars', 'A=1'); __type('#cap-mem', '384'); __tick('[data-capacity-override]'); __type('[data-capacity-confirmation]', ${JSON.stringify(SENTENCE)});
      document.querySelector('#f button[type=submit]').click()`);
    await waitFor(evalJs, `document.querySelector('#err')?.textContent.includes('helper is not running')`, 'configure error');
    const kept2 = await evalJs(`JSON.stringify({ name: document.querySelector('#name').value, port: document.querySelector('#port').value, vars: document.querySelector('#vars').value, mem: document.querySelector('#cap-mem').value,
      override: document.querySelector('[data-capacity-override]').checked, sentence: document.querySelector('[data-capacity-confirmation]').value, focus: document.activeElement?.id })`);
    assert.deepEqual(JSON.parse(kept2), { name: 'shop-web', port: '8080', vars: 'A=1', mem: '384', override: true, sentence: SENTENCE, focus: 'err' });

    // ---- memory dialog: stays open with its values after a refusal
    await evalJs(`import('/js/capacity.js').then((m) => { window.__memory = m.editMemory({ id: 'pg-main', kind: 'db', memoryLimitMb: 256 }); })`);
    await waitFor(evalJs, `document.querySelector('dialog [data-capacity-override]')`, 'memory dialog');
    await waitFor(evalJs, `document.querySelector('dialog [data-capacity-result]')?.textContent === 'Fits'`, 'dialog capacity');
    await evalJs(`__type('dialog [data-capacity-memory]', '512'); __tick('dialog [data-capacity-override]'); __type('dialog [data-capacity-confirmation]', ${JSON.stringify(SENTENCE)}); document.querySelector('dialog button[type=submit]').click()`);
    await waitFor(evalJs, `document.querySelector('dialog .form-err')?.textContent.includes('helper is not running')`, 'dialog error');
    const kept3 = await evalJs(`JSON.stringify({ open: !!document.querySelector('dialog[open]'), mem: document.querySelector('dialog [data-capacity-memory]').value,
      override: document.querySelector('dialog [data-capacity-override]').checked, sentence: document.querySelector('dialog [data-capacity-confirmation]').value, focusIsAlert: document.activeElement?.classList.contains('form-err') })`);
    assert.deepEqual(JSON.parse(kept3), { open: true, mem: '512', override: true, sentence: SENTENCE, focusIsAlert: true });
    await evalJs(`document.querySelector('dialog [data-no]').click()`);

    // ---- helper banner: down, then mismatch, then back without a reload
    await evalJs(`window.__shell = import('/js/shell.js')`);
    const poll = () => evalJs(`window.__shell.then((m) => m.refreshHelperBanner())`);
    await poll();
    assert.equal(await evalJs(`document.querySelector('#helper-banner').textContent.trim()`), '');
    state.ready = { status: 'not_ready', checks: { database: 'ok', privileged_helper: 'failing' } };
    await poll();
    const text = await evalJs(`document.querySelector('#helper-banner [role=alert]').textContent`);
    assert(text.includes("OpsBox's helper is not running. Actions that change the server are paused.") && text.includes('sudo systemctl restart opsbox-priv'), text);
    assert.equal(await evalJs(`document.querySelector('#helper-banner [data-act="copy-helper-cmd"]').textContent`), 'Copy');
    state.ready = { status: 'not_ready', checks: { database: 'ok', privileged_helper: 'version mismatch', versions: { serve: '0.1.0-alpha.21', helper: '0.1.0-alpha.20' } } };
    await poll();
    const text2 = await evalJs(`document.querySelector('#helper-banner').textContent`);
    assert(text2.includes('0.1.0-alpha.20') && text2.includes('0.1.0-alpha.21'), text2);
    state.ready = { status: 'ready', checks: { database: 'ok', privileged_helper: 'ok' } };
    await poll();
    assert.equal(await evalJs(`document.querySelector('#helper-banner').textContent.trim()`), '');
    console.log('Forms keep typed values after a refused submit (template, New app, memory dialog); helper banner shows down/mismatch and clears.');
  } finally {
    await b.close();
    server.close();
  }
}

main().then(() => process.exit(0), (e) => { console.error(e); process.exit(1); });
