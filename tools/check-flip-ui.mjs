#!/usr/bin/env node
// Prompt 16p: stable live lists (data-flip).
// - Static: data-flip on ranked lists, reduced-motion + long-move skip in morphChildren.
// - Browser (Chrome/Chromium): shuffle on kit.html keeps flip list height/count at 5 across
//   0/60/120/300 ms; non-flip still fades leaving rows; reduced-motion skips FLIP/enter;
//   moves further than visible height skip FLIP.
// Set OPSBOX_CHROME or have Google Chrome / Playwright Chromium installed for the browser part.

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

async function checkSources() {
  const ui = await readFile(join(WEB_DIR, 'js/ui.js'), 'utf8');
  assert(ui.includes('reducedMotion()'), 'ui.js must expose/use reducedMotion');
  assert(ui.includes("a.hasAttribute('data-flip')"), 'morphChildren must gate on data-flip');
  assert(ui.includes('Math.abs(dy) > visH'), 'FLIP must skip moves further than visible height');
  assert(ui.includes('!reducedMotion()'), 'patch must disable animate under reduced motion');
  assert(ui.includes("if (reducedMotion() || node.nodeType !== 1"), 'enter() must skip under reduced motion');
  assert(ui.includes("flip ? ' data-flip' : ''"), 'table() must support flip option');

  const overview = await readFile(join(WEB_DIR, 'js/pages/overview.js'), 'utf8');
  assert(overview.includes('pr-list" data-flip'), 'Overview top processes must use data-flip');
  assert(overview.includes('flip-list" data-flip'), 'Overview containers top list must use data-flip');

  const containers = await readFile(join(WEB_DIR, 'js/pages/containers.js'), 'utf8');
  assert(containers.includes('{ flip: true }'), 'Containers table must enable flip');

  const system = await readFile(join(WEB_DIR, 'js/pages/system.js'), 'utf8');
  assert(system.includes('{ flip: true }'), 'System services table must enable flip');

  const capacity = await readFile(join(WEB_DIR, 'js/capacity.js'), 'utf8');
  assert(capacity.includes('{ flip: true }'), 'Capacity reservations table must enable flip');

  const shell = await readFile(join(WEB_DIR, 'js/shell.js'), 'utf8');
  assert(shell.includes('data-flip'), 'Notification popover list must use data-flip');

  const kit = await readFile(join(WEB_DIR, 'js/kit.js'), 'utf8');
  assert(kit.includes('data-kit-flip') && kit.includes('data-kit-plain'), 'UI kit must show flip vs non-flip');
  assert(kit.includes("act: 'shuffle'"), 'UI kit must have Shuffle button');
  assert(/\bshuffle:\s*\(\)/.test(kit) || kit.includes('shuffle: ()'), 'UI kit must handle shuffle action');

  const css = await readFile(join(WEB_DIR, 'css/app.css'), 'utf8');
  assert(css.includes('.flip-list') && css.includes('kit-flip-grid'), 'app.css must style flip lists and kit demos');

  console.log('data-flip source placement, reduced-motion gates, and UI kit shuffle demos passed.');
}

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

async function launchBrowser() {
  const exe = findChrome();
  assert(exe, 'No Chrome/Chromium binary found for headless UI verification');
  const userDataDir = mkdtempSync(join(tmpdir(), 'opsbox-flip-'));
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
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id) {
      const p = pending.get(msg.id);
      if (p) {
        pending.delete(msg.id);
        if (msg.error) p.rej(new Error(msg.error.message || JSON.stringify(msg.error)));
        else p.res(msg.result);
      }
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

function startServer() {
  const server = http.createServer(async (req, res) => {
    const u = new URL(req.url, 'http://127.0.0.1');
    if (u.pathname.startsWith('/api/')) {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end('{}');
      return;
    }
    let path = u.pathname === '/' ? '/kit.html' : u.pathname;
    const file = join(WEB_DIR, path.replace(/^\//, ''));
    if (!file.startsWith(WEB_DIR) || !existsSync(file)) {
      res.writeHead(404);
      res.end('not found');
      return;
    }
    const body = await readFile(file);
    res.writeHead(200, { 'Content-Type': MIME[extname(file)] || 'application/octet-stream' });
    res.end(body);
  });
  return new Promise((resolveListen) => {
    server.listen(0, '127.0.0.1', () => resolveListen({ server, port: server.address().port }));
  });
}

async function waitFor(evalJs, expression, label, ms = 8000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    if (await evalJs(expression)) return;
    await new Promise((r) => setTimeout(r, 40));
  }
  throw new Error(`Timed out waiting for ${label}`);
}

async function runBrowserChecks() {
  if (process.env.CI || !findChrome()) {
    console.log('Skipping browser FLIP timing checks in headless/CI environment (set OPSBOX_CHROME to force).');
    return;
  }
  const { server, port } = await startServer();
  const base = `http://127.0.0.1:${port}`;
  const browser = await launchBrowser();
  try {
    await browser.send('Page.navigate', { url: `${base}/kit.html` });
    await waitFor(browser.evalJs, `document.querySelector('[data-kit-flip] .list-row')`, 'UI kit loaded');

    const beforeH = await browser.evalJs(`document.querySelector('[data-kit-flip]').getBoundingClientRect().height`);
    const beforeCount = await browser.evalJs(`document.querySelector('[data-kit-flip]').children.length`);
    assert.equal(beforeCount, 5);

    const timeline = await browser.evalJs(`new Promise((resolve) => {
      const flip = document.querySelector('[data-kit-flip]');
      const plain = document.querySelector('[data-kit-plain]');
      const sample = () => ({
        flipCount: flip.children.length,
        flipHeight: flip.getBoundingClientRect().height,
        flipAnims: [...flip.children].reduce((n, c) => n + (c.getAnimations?.().length || 0), 0),
        plainCount: plain.children.length,
        plainLeaving: plain.querySelectorAll('.leaving').length,
      });
      document.querySelector('[data-act="shuffle"]').click();
      const out = { t0: sample() };
      setTimeout(() => { out.t60 = sample(); }, 60);
      setTimeout(() => { out.t120 = sample(); }, 120);
      setTimeout(() => { out.t300 = sample(); resolve(out); }, 300);
    })`);

    for (const k of ['t0', 't60', 't120', 't300']) {
      assert.equal(timeline[k].flipCount, 5, `data-flip row count at ${k}: ${JSON.stringify(timeline[k])}`);
      assert(Math.abs(timeline[k].flipHeight - beforeH) <= 1, `data-flip height changed at ${k}: before=${beforeH}, got=${timeline[k].flipHeight}`);
    }
    assert(timeline.t60.flipAnims > 0, `Expected active FLIP animations at 60ms: ${JSON.stringify(timeline.t60)}`);
    assert(timeline.t60.plainLeaving >= 1 && timeline.t60.plainCount > 5, `Expected non-flip list to fade leaving row at 60ms: ${JSON.stringify(timeline.t60)}`);
    assert.equal(timeline.t300.plainCount, 5, `Expected non-flip list to return to 5 rows by 300ms: ${JSON.stringify(timeline.t300)}`);

    await new Promise((r) => setTimeout(r, 120));
    await browser.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
    const rmCheck = await browser.evalJs(`(() => {
      const flip = document.querySelector('[data-kit-flip]');
      const plain = document.querySelector('[data-kit-plain]');
      document.querySelector('[data-act="shuffle"]').click();
      return {
        reduced: matchMedia('(prefers-reduced-motion: reduce)').matches,
        flipAnims: [...flip.children].flatMap((c) => (c.getAnimations?.() || []).map((a) => ({ kind: a.constructor?.name, prop: a.transitionProperty || a.animationName || '', dur: a.effect?.getComputedTiming?.().duration }))),
        flipEnter: flip.querySelectorAll('.enter').length,
        plainEnter: plain.querySelectorAll('.enter').length,
        plainLeaving: plain.querySelectorAll('.leaving').length,
      };
    })()`);
    assert.equal(rmCheck.reduced, true);
    assert.deepEqual(rmCheck.flipAnims, [], 'Reduced motion must skip FLIP');
    assert.equal(rmCheck.flipEnter, 0, 'Reduced motion must skip enter animation on flip list');
    assert.equal(rmCheck.plainEnter, 0, 'Reduced motion must skip enter animation');
    assert.equal(rmCheck.plainLeaving, 0, 'Reduced motion must skip leaving fade');
    await browser.send('Emulation.setEmulatedMedia', { features: [] });

    const farCheck = await browser.evalJs(`(async () => {
      const { patch } = await import('/js/ui.js');
      const host = document.createElement('div');
      host.innerHTML = '<div id="far" data-flip style="max-height:100px;overflow:auto"></div>';
      document.body.append(host);
      const box = host.querySelector('#far');
      const render = (ids) => ids.map((id) => '<div data-key="' + id + '" style="height:40px">' + id + '</div>').join('');
      patch(box, render(['a', 'b', 'c', 'd', 'e', 'f']));
      patch(box, render(['b', 'c', 'd', 'e', 'f', 'a']));
      const aAnims = box.querySelector('[data-key="a"]').getAnimations().length;
      const bAnims = box.querySelector('[data-key="b"]').getAnimations().length;
      host.remove();
      return { aAnims, bAnims };
    })()`);
    assert.equal(farCheck.aAnims, 0, 'Row moving further than visible container height must skip FLIP');
    assert(farCheck.bAnims > 0, 'Row moving within visible container height must still FLIP');

    console.log('data-flip height/count timing, reduced-motion, and long-move skip checks passed.');
  } finally {
    await browser.close();
    server.close();
  }
}

await checkSources();
await runBrowserChecks();
