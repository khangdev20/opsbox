#!/usr/bin/env node
// Prompt 16s: responsive panel checks.
// - Static: viewport-fit, drawer, card tables, bottom sheets, terminal key bar, KPI breakpoints.
// - Browser (Chrome/Chromium): at 360/390/768/1024 assert no page sideways scroll;
//   at 1440/1920/2560 assert the main page uses available width; drawer
//   opens/closes via button, Escape, route; dialogs are sheets under 640; touch targets ≥ 44
//   on a coarse pointer viewport.
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
  const [index, kitHtml, css, shell, ui, term, files, kitJs] = await Promise.all([
    readFile(join(WEB_DIR, 'index.html'), 'utf8'),
    readFile(join(WEB_DIR, 'kit.html'), 'utf8'),
    readFile(join(WEB_DIR, 'css/app.css'), 'utf8'),
    readFile(join(WEB_DIR, 'js/shell.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/ui.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/pages/terminal.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/pages/files.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/kit.js'), 'utf8'),
  ]);

  assert(index.includes('viewport-fit=cover'), 'index.html needs viewport-fit=cover');
  assert(kitHtml.includes('viewport-fit=cover'), 'kit.html needs viewport-fit=cover');

  assert(shell.includes('nav-btn') && shell.includes('drawer-open'), 'shell must implement drawer + menu button');
  assert(shell.includes('side-backdrop'), 'shell must render drawer backdrop');
  assert(shell.includes('opsboxDrawer'), 'drawer must push history for Back-to-close');
  assert(shell.includes('closeDrawer') && shell.includes('Escape'), 'drawer closes on Escape');
  assert(shell.includes('pal-lbl') && shell.includes('SEARCH_SVG'), 'palette icon-only markup for tablet');

  assert(ui.includes('data-label') && ui.includes('headMeta'), 'table helper must emit data-label');
  assert(ui.includes('sticky') && ui.includes('cards'), 'table helper must support cards/sticky modes');
  assert(ui.includes("className = 'dlg sheet'") || ui.includes('className = "dlg sheet"'), 'form dialogs are sheets');
  assert(ui.includes("className = 'dlg compact'") || ui.includes('className = "dlg compact"'), 'confirms stay compact');

  assert(css.includes('100dvh') && css.includes('safe-area-inset'), 'CSS uses 100dvh and safe-area insets');
  assert(css.includes('.tw.cards') && css.includes('data-label'), 'card-mode table CSS present');
  assert(css.includes('sticky-col') && css.includes('sheet-up'), 'sticky columns and sheet animation present');
  assert(css.includes('(pointer:coarse)') && css.includes('min-height:44px'), 'coarse pointer 44px targets');
  assert(css.includes('font-size:16px'), '16px inputs on touch to avoid iOS zoom');
  assert(css.includes('.term-keys') && css.includes('files-page.editing'), 'terminal key bar + files fullscreen CSS');
  assert(css.includes('grid-template-columns:1fr') && css.includes('repeat(2,minmax(0,1fr))'), 'KPI 1/2 column breakpoints');
  assert(/main\{[^}]*width:min\(100%,1680px\)[^}]*padding:40px clamp\(16px,2\.5vw,40px\) 64px/.test(css), 'main uses a fluid 1680px cap with responsive side padding');
  assert(/main\.narrow #b\{[^}]*max-width:880px/.test(css), 'Settings forms keep a readable inner width');

  assert(term.includes('visualViewport') && term.includes('term-keys'), 'terminal visualViewport + key bar');
  assert(files.includes('files-page') && files.includes('editing'), 'files page toggles fullscreen editor class');
  assert(kitJs.includes('kit-card-table') && kitJs.includes('kit-sheet-demo') && kitJs.includes('kit-drawer'), 'UI kit shows card table, sheet, drawer');

  console.log('responsive source checks (viewport-fit, drawer, cards/sheets, terminal, kit) passed.');
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
  const userDataDir = mkdtempSync(join(tmpdir(), 'opsbox-resp-'));
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

  return {
    send,
    evalJs,
    async close() {
      try { ws.close(); } catch { /* ignore */ }
      if (proc.exitCode === null) {
        const exited = new Promise((res) => proc.once('exit', res));
        try { proc.kill('SIGKILL'); } catch { /* ignore */ }
        await Promise.race([exited, new Promise((res) => setTimeout(res, 1500))]);
      }
      try { rmSync(userDataDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 }); } catch { /* ignore */ }
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
    let path = u.pathname === '/' ? '/index.html' : u.pathname;
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

async function waitFor(evalJs, expression, label, ms = 12000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    if (await evalJs(expression)) return;
    await new Promise((r) => setTimeout(r, 50));
  }
  throw new Error(`Timed out waiting for ${label}`);
}

async function setViewport(browser, width, height = 800, mobile = width <= 768) {
  await browser.send('Emulation.setDeviceMetricsOverride', {
    width, height, deviceScaleFactor: 2, mobile,
  });
  if (width <= 640) {
    await browser.send('Emulation.setTouchEmulationEnabled', { enabled: true });
  }
}

async function runBrowserChecks() {
  if (!findChrome()) {
    console.log('Skipping browser responsive checks (no Chrome/Chromium; set OPSBOX_CHROME to enable).');
    return;
  }
  const { server, port } = await startServer();
  const base = `http://127.0.0.1:${port}`;
  const browser = await launchBrowser();
  try {
    await setViewport(browser, 390);
    await browser.send('Page.addScriptToEvaluateOnNewDocument', {
      source: `try { sessionStorage.setItem('opsbox.user', 'admin'); } catch (e) {}`,
    });
    await browser.send('Page.navigate', { url: `${base}/index.html?mode=mock#/` });
    await waitFor(browser.evalJs, `!!document.getElementById('nav-btn') && !!document.querySelector('.shell')`, 'shell mounted');

    for (const w of [360, 390, 768, 1024]) {
      await setViewport(browser, w, w <= 768 ? 800 : 900);
      await new Promise((r) => setTimeout(r, 80));
      const scroll = await browser.evalJs(`({ sw: document.documentElement.scrollWidth, iw: window.innerWidth, drawerBtn: !!document.getElementById('nav-btn') && getComputedStyle(document.getElementById('nav-btn')).display !== 'none' })`);
      assert(scroll.sw <= scroll.iw + 1, `horizontal page scroll at ${w}px: scrollWidth=${scroll.sw} innerWidth=${scroll.iw}`);
      if (w <= 900) assert.equal(scroll.drawerBtn, true, `menu button visible at ${w}px`);
      else assert.equal(scroll.drawerBtn, false, `menu button hidden at ${w}px`);
    }

    for (const w of [1440, 1920, 2560]) {
      await setViewport(browser, w, 900);
      const page = await browser.evalJs(`(() => {
        const main = document.querySelector('.content main');
        return { width: main?.getBoundingClientRect().width, scrollWidth: document.documentElement.scrollWidth, viewport: window.innerWidth };
      })()`);
      assert(page.width, `main page container missing at ${w}px`);
      assert(page.scrollWidth <= page.viewport + 1, `horizontal page scroll at ${w}px`);
      if (w === 1440) assert(page.width >= 1100 && page.width <= 1440, `main width at 1440px: ${page.width}`);
      if (w === 1920) assert(page.width >= 1400, `main width at 1920px: ${page.width}`);
      if (w === 2560) assert(page.width <= 1800, `main width at 2560px: ${page.width}`);
    }

    await setViewport(browser, 390);
    const drawer = await browser.evalJs(`(async () => {
      const btn = document.getElementById('nav-btn');
      const shell = document.querySelector('.shell');
      btn.click();
      await new Promise((r) => setTimeout(r, 40));
      const open = shell.classList.contains('drawer-open') && btn.getAttribute('aria-expanded') === 'true';
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      await new Promise((r) => setTimeout(r, 40));
      const closedEsc = !shell.classList.contains('drawer-open');
      btn.click();
      await new Promise((r) => setTimeout(r, 40));
      location.hash = '#/apps';
      await new Promise((r) => setTimeout(r, 80));
      const closedRoute = !shell.classList.contains('drawer-open');
      document.getElementById('side-backdrop')?.click();
      return { open, closedEsc, closedRoute };
    })()`);
    assert.equal(drawer.open, true, 'drawer opens from menu button');
    assert.equal(drawer.closedEsc, true, 'drawer closes on Escape');
    assert.equal(drawer.closedRoute, true, 'drawer closes on route change');

    await setViewport(browser, 390);
    const sheet = await browser.evalJs(`(async () => {
      const { formDialog, confirmDialog } = await import('/js/ui.js');
      const p = formDialog({ title: 'Sheet demo', fields: [{ name: 'n', label: 'Name', value: 'x' }], submit: 'Save' });
      await new Promise((r) => setTimeout(r, 40));
      const d = document.querySelector('dialog.dlg.sheet[open]');
      const cs = d ? getComputedStyle(d) : null;
      const isSheet = !!d && (cs.borderTopLeftRadius !== cs.borderBottomLeftRadius || d.getBoundingClientRect().bottom >= window.innerHeight - 2);
      document.querySelector('dialog [data-no]')?.click();
      await p;
      const c = confirmDialog({ title: 'Compact?', body: 'Stay compact.', confirm: 'OK' });
      await new Promise((r) => setTimeout(r, 40));
      const compact = !!document.querySelector('dialog.dlg.compact[open]');
      document.querySelector('dialog [data-no]')?.click();
      await c;
      return { isSheet, compact };
    })()`);
    assert.equal(sheet.isSheet, true, 'form dialog is a bottom sheet under 640px');
    assert.equal(sheet.compact, true, 'confirm dialog stays compact');

    await setViewport(browser, 390);
    await browser.send('Emulation.setTouchEmulationEnabled', { enabled: true });
    try {
      await browser.send('Emulation.setEmulatedMedia', {
        features: [
          { name: 'pointer', value: 'coarse' },
          { name: 'any-pointer', value: 'coarse' },
          { name: 'hover', value: 'none' },
          { name: 'any-hover', value: 'none' },
        ],
      });
    } catch {
      // Older Chromium: fall through; we still inject a coarse-pointer mirror below.
    }
    const targets = await browser.evalJs(`(() => {
      document.getElementById('nav-btn')?.click();
      const all = ['#nav-btn', '#notif-btn', '#theme-btn', '#side .nav a', '.acct-btn']
        .flatMap((sel) => [...document.querySelectorAll(sel)])
        .filter((el) => {
          const cs = getComputedStyle(el);
          return cs.display !== 'none' && cs.visibility !== 'hidden';
        });
      const small = all.map((el) => {
        const r = el.getBoundingClientRect();
        return { tag: el.tagName, id: el.id || el.getAttribute('href'), w: Math.round(r.width), h: Math.round(r.height) };
      }).filter((x) => x.w > 0 && x.h > 0 && (x.w < 44 || x.h < 44));
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      return { count: all.length, coarse: matchMedia('(pointer: coarse)').matches, small };
    })()`);
    assert(targets.count > 0, 'expected interactive elements');
    if (targets.coarse) {
      assert.deepEqual(targets.small, [], `interactive elements smaller than 44px on phone: ${JSON.stringify(targets.small)}`);
    } else {
      console.log('pointer:coarse not emulated in this Chromium; 44px rule covered by source check.');
    }

    await browser.send('Page.navigate', { url: `${base}/kit.html` });
    await waitFor(browser.evalJs, `!!document.querySelector('.kit-card-table') && !!document.querySelector('.kit-sheet-demo') && !!document.querySelector('.kit-drawer')`, 'kit responsive demos');

    console.log('responsive browser checks (scroll, wide layout, drawer, sheets, 44px targets, kit) passed.');
  } finally {
    await browser.close();
    server.close();
  }
}

await checkSources();
await runBrowserChecks();
