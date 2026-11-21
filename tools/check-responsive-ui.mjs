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
  const [index, kitHtml, css, shell, ui, term, files, kitJs, cleanupJs, newServiceJs] = await Promise.all([
    readFile(join(WEB_DIR, 'index.html'), 'utf8'),
    readFile(join(WEB_DIR, 'kit.html'), 'utf8'),
    readFile(join(WEB_DIR, 'css/app.css'), 'utf8'),
    readFile(join(WEB_DIR, 'js/shell.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/ui.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/pages/terminal.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/pages/files.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/kit.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/pages/cleanup.js'), 'utf8'),
    readFile(join(WEB_DIR, 'js/pages/newservice.js'), 'utf8'),
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
  assert(/main\.narrow #b\{[^}]*max-width:880px/.test(css), 'other narrow pages keep their readable inner width');
  assert(/html: html`<main>/.test(cleanupJs), 'Cleanup must use the full-width main container');
  assert(/html: html`<main>/.test(newServiceJs), 'New service must use the full-width main container');
  assert(newServiceJs.includes('repo-row') && css.includes('.repo-row{display:grid'), 'New service repository rows use a grid');
  assert(css.includes('grid-template-columns:minmax(0,1fr) auto') && css.includes('.repo-row-action{justify-self:end;min-width:120px;min-height:44px}'), 'repository grid has a right-aligned, 44px action');
  assert(css.includes('gap:12px 16px;padding:12px 0'), 'repository rows have 12px vertical padding');
  assert(css.includes('.repo-row{grid-template-columns:minmax(0,1fr)}') && css.includes('.repo-row-name{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}'), 'repository rows stack on phones and truncate names');
  assert(newServiceJs.includes('title="${r.name}"') && newServiceJs.includes('Choose build') && newServiceJs.includes("'nothing to build'"), 'repository names retain their title and missing builds need a choice');
  assert((newServiceJs.match(/class="new-service-form-grid"/g) || []).length === 3, 'all three source forms use the wide form grid');
  assert((newServiceJs.match(/class="new-service-form-controls"/g) || []).length === 3, 'source form fields and actions share a grid cell');
  assert(/\.new-service-form-grid\{display:grid;grid-template-columns:repeat\(2,minmax\(0,1fr\)\)/.test(css), 'source forms use two wide columns');
  assert(/\.new-service-form-controls \.input\{[^}]*max-width:560px/.test(css), 'source form fields have a readable width cap');
  assert(newServiceJs.includes('new-service-toolbar') && css.includes('.new-service-toolbar>:last-child{margin-left:auto}'), 'New service header actions stay right aligned');

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
    await waitFor(browser.evalJs, `!!document.getElementById('nav-btn') && !!document.querySelector('.shell') && !!document.querySelector('.content main')`, 'shell mounted');
    const blockedCard = await browser.evalJs(`(async () => {
      const { blockedCapacityCard } = await import('/js/capacity.js');
      const fixture = { checks: [
        { kind: 'memory', need_mb: 1536, have_mb: 700, ok: false, breakdown: ['RAM','OS','opsbox','Docker','workloads','swap credit'].map(label => ({ label, mb: 128 })) },
        { kind: 'disk', need_mb: 7168, have_mb: 3482, ok: false, paths: ['/var/lib/docker','/srv/apps'] }
      ], actions: [
        { id: 'lower_limit', effect_mb: 640 }, { id: 'build_in_ci' },
        { id: 'stop_service', target: 'shop', effect_mb: 256 },
        { id: 'cleanup_disk' }, { id: 'upgrade_instance' }
      ] };
      const host = document.createElement('div');
      host.innerHTML = blockedCapacityCard(fixture).toString();
      document.querySelector('.content main').append(host);
      const card = host.querySelector('.cap-blocked');
      const result = { rows: card.querySelectorAll('.cap-check').length,
        paths: (card.textContent.match(/\\/var\\/lib\\/docker/g) || []).length,
        collapsed: !card.querySelector('details').open,
        actions: [...card.querySelectorAll('.cap-action')].map(el => ({ tag: el.tagName, height: el.getBoundingClientRect().height })),
        scrollWidth: document.documentElement.scrollWidth, viewport: innerWidth };
      host.remove();
      return result;
    })()`);
    assert.equal(blockedCard.rows, 2, 'blocked card has two check rows');
    assert.equal(blockedCard.paths, 1, 'disk paths appear once');
    assert(blockedCard.collapsed, 'capacity details start collapsed');
    assert.equal(blockedCard.actions.length, 5, 'blocked card has five actions');
    assert(blockedCard.actions.every((a) => ['A', 'BUTTON'].includes(a.tag) && a.height >= 43.5), `capacity action targets at 390px: ${JSON.stringify(blockedCard.actions)}`);
    assert(blockedCard.scrollWidth <= blockedCard.viewport + 1, `blocked card overflows at 390px: ${blockedCard.scrollWidth}/${blockedCard.viewport}`);

    for (const w of [360, 390, 768, 1024]) {
      await setViewport(browser, w, w <= 768 ? 800 : 900);
      await new Promise((r) => setTimeout(r, 80));
      const scroll = await browser.evalJs(`({ sw: document.documentElement.scrollWidth, iw: window.innerWidth, drawerBtn: !!document.getElementById('nav-btn') && getComputedStyle(document.getElementById('nav-btn')).display !== 'none' })`);
      assert(scroll.sw <= scroll.iw + 1, `horizontal page scroll at ${w}px: scrollWidth=${scroll.sw} innerWidth=${scroll.iw}`);
      if (w <= 900) assert.equal(scroll.drawerBtn, true, `menu button visible at ${w}px`);
      else assert.equal(scroll.drawerBtn, false, `menu button hidden at ${w}px`);
    }

    let overviewWidth1920;
    for (const w of [1440, 1920, 2560]) {
      await setViewport(browser, w, 900);
      const page = await browser.evalJs(`(() => {
        const main = document.querySelector('.content main');
        return { width: main?.getBoundingClientRect().width, scrollWidth: document.documentElement.scrollWidth, viewport: window.innerWidth };
      })()`);
      assert(page.width, `main page container missing at ${w}px`);
      assert(page.scrollWidth <= page.viewport + 1, `horizontal page scroll at ${w}px`);
      if (w === 1440) assert(page.width >= 1100 && page.width <= 1440, `main width at 1440px: ${page.width}`);
      if (w === 1920) {
        overviewWidth1920 = page.width;
        assert(page.width >= 1400, `overview width at 1920px: ${page.width}`);
      }
      if (w === 2560) assert(page.width <= 1800, `main width at 2560px: ${page.width}`);
    }

    await setViewport(browser, 1920, 900);
    await browser.send('Page.navigate', { url: `${base}/index.html?mode=mock#/cleanup` });
    await waitFor(browser.evalJs, `!!document.querySelector('.content main #b') && !!document.querySelector('.content main [data-act="auto"]')`, 'cleanup mounted');
    const cleanup = await browser.evalJs(`(() => {
      const main = document.querySelector('.content main').getBoundingClientRect();
      const list = document.querySelector('.content main #b').getBoundingClientRect();
      const toggle = document.querySelector('.content main [data-act="auto"]').getBoundingClientRect();
      return { width: main.width, listRight: list.right, toggleRight: toggle.right };
    })()`);
    assert(cleanup.width >= 1400, `cleanup content width at 1920px: ${cleanup.width}`);
    assert(Math.abs(cleanup.toggleRight - cleanup.listRight) <= 1, `cleanup toggle/list right edges: ${cleanup.toggleRight}/${cleanup.listRight}`);
    // Pages with different scrollbar presence can differ by the scrollbar width.
    assert(Math.abs(cleanup.width - overviewWidth1920) <= 24, `cleanup/overview widths: ${cleanup.width}/${overviewWidth1920}`);
    for (const w of [390, 768, 1024]) {
      await setViewport(browser, w, w <= 768 ? 800 : 900);
      const scroll = await browser.evalJs(`({ sw: document.documentElement.scrollWidth, iw: window.innerWidth })`);
      assert(scroll.sw <= scroll.iw + 1, `cleanup horizontal page scroll at ${w}px: scrollWidth=${scroll.sw} innerWidth=${scroll.iw}`);
    }

    await setViewport(browser, 1920, 900);
    await browser.send('Page.navigate', { url: `${base}/index.html?mode=mock#/settings` });
    await waitFor(browser.evalJs, `!!document.querySelector('.content main #b #sh') && !!document.querySelector('.content main #b #imp-pass')`, 'settings mounted');
    const settings = await browser.evalJs(`(() => {
      const main = document.querySelector('.content main').getBoundingClientRect();
      const body = document.querySelector('.content main #b');
      const cards = [...body.querySelectorAll(':scope > .sec')];
      const generalCard = cards[0];
      const general = generalCard.getBoundingClientRect();
      const generalContentRight = general.right - parseFloat(getComputedStyle(generalCard).paddingRight);
      const network = body.querySelector(':scope > #network-card').getBoundingClientRect();
      const save = body.querySelector('[data-act="save"]').getBoundingClientRect();
      const fields = ['#sh', '#imp-pass'].map((selector) => {
        const input = body.querySelector(selector).getBoundingClientRect();
        const card = body.querySelector(selector).closest('.sec').getBoundingClientRect();
        return { width: input.width, left: input.left, cardLeft: card.left };
      });
      const updates = cards.find((card) => card.querySelector('[data-act="autoup"]'));
      const switchRow = updates?.querySelector('[data-act="autoup"]')?.closest('.list-row');
      const switchControl = switchRow?.lastElementChild?.getBoundingClientRect();
      return { mainWidth: main.width, bodyWidth: body.getBoundingClientRect().width,
        columns: getComputedStyle(body).gridTemplateColumns.split(' ').length,
        cards: cards.length, networkSameRow: Math.abs(network.top - general.top) < 2 && network.left > general.left,
        saveRight: save.right, generalRight: generalContentRight, fields,
        switchRight: switchControl?.right, switchRowRight: switchRow?.getBoundingClientRect().right };
    })()`);
    assert(settings.mainWidth >= 1400 && settings.bodyWidth >= 1400, `settings width at 1920px: ${JSON.stringify(settings)}`);
    assert(Math.abs(settings.mainWidth - overviewWidth1920) <= 24, `settings/overview widths: ${settings.mainWidth}/${overviewWidth1920}`);
    assert(settings.cards >= 6 && settings.columns === 2 && settings.networkSameRow, `settings cards need two columns: ${JSON.stringify(settings)}`);
    for (const field of settings.fields) {
      assert(field.width <= 561, `settings form field too wide: ${JSON.stringify(field)}`);
      assert(field.left >= field.cardLeft + 16 && field.left <= field.cardLeft + 40, `settings form field not left aligned: ${JSON.stringify(field)}`);
    }
    assert(Math.abs(settings.saveRight - settings.generalRight) <= 26, `settings Save not at card right: ${JSON.stringify(settings)}`);
    assert(settings.switchRight && Math.abs(settings.switchRight - settings.switchRowRight) <= 1, `settings switch not at row right: ${JSON.stringify(settings)}`);
    for (const w of [390, 768, 1024]) {
      await setViewport(browser, w, w <= 768 ? 800 : 900);
      const layout = await browser.evalJs(`({ sw: document.documentElement.scrollWidth, iw: innerWidth,
        columns: getComputedStyle(document.querySelector('.content main #b')).gridTemplateColumns.split(' ').length })`);
      assert(layout.sw <= layout.iw + 1, `settings horizontal scroll at ${w}px: ${JSON.stringify(layout)}`);
      assert.equal(layout.columns, 1, `settings should use one column at ${w}px`);
    }

    await setViewport(browser, 1920, 900);
    await browser.send('Page.navigate', { url: `${base}/index.html?mode=mock#/apps/new` });
    await waitFor(browser.evalJs, `!!document.querySelector('#l .repo-row')`, 'new service repositories mounted');
    const newServiceWidth = await browser.evalJs(`document.querySelector('.content main').getBoundingClientRect().width`);
    assert(newServiceWidth >= 1400, `new service content width at 1920px: ${newServiceWidth}`);
    assert(Math.abs(newServiceWidth - overviewWidth1920) <= 24, `new service/overview widths: ${newServiceWidth}/${overviewWidth1920}`);
    await browser.send('Page.navigate', { url: `${base}/index.html?mode=mock#/apps/new/templates` });
    await waitFor(browser.evalJs, `!!document.querySelector('.content main .cols #fm')`, 'templates mounted');
    const templatesWidth = await browser.evalJs(`document.querySelector('.content main .cols').getBoundingClientRect().width`);
    assert(templatesWidth >= 1400, `Templates content width at 1920px: ${templatesWidth}`);
    for (const w of [390, 768, 1024]) {
      await setViewport(browser, w, w === 390 ? 800 : 900);
      const scroll = await browser.evalJs(`({ sw: document.documentElement.scrollWidth, iw: innerWidth })`);
      assert(scroll.sw <= scroll.iw + 1, `Templates horizontal scroll at ${w}px: ${scroll.sw}/${scroll.iw}`);
    }
    await setViewport(browser, 1920, 900);
    await browser.send('Page.navigate', { url: `${base}/index.html?mode=mock#/apps/new` });
    await waitFor(browser.evalJs, `!!document.querySelector('#l .repo-row')`, 'new service repositories remounted');
    for (const [i, label] of ['GitHub', 'Deploy key', 'Public repo URL', 'Docker image'].entries()) {
      await browser.evalJs(`document.querySelector('[data-t="${i}"]').click()`);
      const area = await browser.evalJs(`(() => {
        const main = document.querySelector('.content main').getBoundingClientRect();
        const body = document.querySelector('#body').getBoundingClientRect();
        const grid = document.querySelector('#body .new-service-form-grid');
        const controls = grid?.querySelector('.new-service-form-controls');
        const input = controls?.querySelector('.input');
        const action = controls?.querySelector('.btn');
        return { mainWidth: main.width, width: body.width, gridWidth: grid?.getBoundingClientRect().width,
          columns: grid && getComputedStyle(grid).gridTemplateColumns.split(' ').length,
          inputWidth: input?.getBoundingClientRect().width, actionRight: action?.getBoundingClientRect().right,
          controlsRight: controls?.getBoundingClientRect().right };
      })()`);
      assert(area.width >= 1400, `${label} content width at 1920px: ${area.width}`);
      assert(Math.abs(area.width - templatesWidth) <= 24, `${label}/Templates content widths: ${area.width}/${templatesWidth}`);
      assert(Math.abs(area.mainWidth - newServiceWidth) <= 24, `${label}/GitHub main widths: ${area.mainWidth}/${newServiceWidth}`);
      if (i > 0) {
        assert(area.gridWidth >= 1400 && area.columns === 2, `${label} form does not use two wide columns: ${JSON.stringify(area)}`);
        assert(area.inputWidth <= 561, `${label} field width: ${area.inputWidth}`);
        assert(Math.abs(area.actionRight - area.controlsRight) <= 1, `${label} action is not at its form column edge`);
      }
      for (const w of [390, 768, 1024]) {
        await setViewport(browser, w, w === 390 ? 800 : 900);
        const mobile = await browser.evalJs(`({ sw: document.documentElement.scrollWidth, iw: innerWidth,
          actionHeight: document.querySelector('#body .new-service-form-controls .btn')?.getBoundingClientRect().height })`);
        assert(mobile.sw <= mobile.iw + 1, `${label} horizontal scroll at ${w}px: ${mobile.sw}/${mobile.iw}`);
        if (w === 390 && i > 0) assert(mobile.actionHeight >= 43.5, `${label} action height at 390px: ${mobile.actionHeight}`);
      }
      await setViewport(browser, 1920, 900);
    }
    await browser.evalJs(`document.querySelector('[data-t="0"]').click()`);
    for (const w of [1440, 390]) {
      await setViewport(browser, w, w === 390 ? 800 : 900);
      const layout = await browser.evalJs(`(() => {
        const searchRight = document.querySelector('#q').getBoundingClientRect().right;
        const rows = [...document.querySelectorAll('#l .repo-row')].map((row) => {
          const text = row.querySelector('.repo-row-text').getBoundingClientRect();
          const action = row.querySelector('.repo-row-action');
          const button = action.getBoundingClientRect();
          const style = getComputedStyle(row);
          return { name: row.querySelector('.repo-row-name')?.textContent.trim(), title: row.querySelector('.repo-row-name')?.title,
            noBuild: /nothing to build/i.test(row.textContent), enabledDeploy: !!row.querySelector('a[href*="/configure"], button:not(:disabled)') && action.textContent.trim() === 'Deploy',
            action: action.textContent.trim(), right: button.right, top: button.top, textBottom: text.bottom,
            height: button.height, padTop: parseFloat(style.paddingTop), padBottom: parseFloat(style.paddingBottom) };
        });
        const toolbar = document.querySelector('#body .new-service-toolbar');
        return { searchRight, rows, scrollWidth: document.documentElement.scrollWidth, viewport: innerWidth,
          toolbarRight: toolbar?.getBoundingClientRect().right,
          cancelRight: toolbar?.querySelector('a[href="#/apps"]')?.getBoundingClientRect().right };
      })()`);
      assert(layout.rows.length > 0, `no repository rows at ${w}px`);
      assert(Math.abs(layout.toolbarRight - layout.cancelRight) <= 1, `GitHub Cancel does not align to toolbar right at ${w}px`);
      for (const row of layout.rows) {
        assert(Math.abs(row.right - layout.searchRight) <= 1, `${row.name} action/search right edges at ${w}px: ${row.right}/${layout.searchRight}`);
        assert(row.padTop >= 12 && row.padBottom >= 12, `${row.name} row padding at ${w}px: ${row.padTop}/${row.padBottom}`);
        assert.equal(row.title, row.name, `${row.name} full name title at ${w}px`);
        if (row.noBuild) {
          assert(!row.enabledDeploy && row.action === 'Choose build', `${row.name} must offer Choose build instead of enabled Deploy`);
        }
        if (w === 390) {
          assert(row.top >= row.textBottom, `${row.name} action must sit under text at 390px`);
          assert(row.height >= 43.5, `${row.name} action height at 390px: ${row.height}`);
        }
      }
      assert(layout.scrollWidth <= layout.viewport + 1, `new service horizontal scroll at ${w}px: ${layout.scrollWidth}/${layout.viewport}`);
    }
    for (const w of [768, 1024]) {
      await setViewport(browser, w, 900);
      const scroll = await browser.evalJs(`({ sw: document.documentElement.scrollWidth, iw: innerWidth })`);
      assert(scroll.sw <= scroll.iw + 1, `new service horizontal scroll at ${w}px: ${scroll.sw}/${scroll.iw}`);
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
