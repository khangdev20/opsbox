// Run: node --experimental-vm-modules tools/check-touch-targets.test.mjs
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';

const css = await readFile(new URL('../web/css/app.css', import.meta.url), 'utf8');
const source = (name) => readFile(new URL(`../web/js/${name}`, import.meta.url), 'utf8');

// Render the real table templates with populated API results, without a server.
async function tables() {
  const context = createContext({ location: { hash: '' }, history: { replaceState() {} } });
  async function load(name, deps = {}) {
    const mod = new SourceTextModule(await source(name), { context });
    await mod.link(async (name) => {
      assert(deps[name], `Unexpected import ${name}`);
      const values = deps[name];
      return new SyntheticModule(Object.keys(values), function () {
        for (const [key, value] of Object.entries(values)) this.setExport(key, value);
      }, { context });
    });
    await mod.evaluate(); return mod.namespace;
  }
  const ui = await load('ui.js');
  async function render(name, api, target) {
    const mod = await load(`pages/${name}.js`, {
      '../ui.js': { ...ui, patch(node, markup) { node.innerHTML = markup; }, bindActions() {} },
      '../api.js': { MODE: 'http', ...api }, '../store.js': { server: { role: 'admin' } },
      '../vendor-loader.js': { loadAmd() {} },
      '../capacity.js': { capacityPage() {}, addSwap() {}, editMemory() {}, bindMemoryMap() {} },
    });
    const nodes = new Map();
    const root = { classList: { toggle() {} }, addEventListener() {}, querySelectorAll() { return []; },
      querySelector(s) {
        if (s === '#mon') return null;
        if (!nodes.has(s)) nodes.set(s, { innerHTML: '', addEventListener() {} });
        return nodes.get(s);
      } };
    const page = await mod.view({ query: { p: '/' }, path: '/system/updates' });
    await page.mount(root);
    return nodes.get(target).innerHTML;
  }
  const files = await render('files', {
    listDir: async () => ({ entries: ['one.txt', 'two.txt'].map((name) => ({
      name, path: `/${name}`, type: 'f', mode: '644', size: 12, mtime: 'today',
    })) }), downloadUrl: (path) => `/download${path}`,
  }, '#l');
  const system = await render('system', { getUpdates: async () => ({
    list: ['curl', 'openssl'].map((pkg) => ({ pkg, cur: '1', next: '2', sec: true })),
    total: 2, security: 2, checked: 'now', running: 'test',
  }) }, '#b');
  return `<main class="files-page">${files}</main>${system}`;
}

function chromePath() {
  if (process.env.OPSBOX_CHROME) return process.env.OPSBOX_CHROME;
  const cache = join(homedir(), 'Library/Caches/ms-playwright');
  const candidates = existsSync(cache) ? readdirSync(cache).filter((p) => p.startsWith('chromium_headless_shell-'))
    .sort().reverse().map((p) => join(cache, p, 'chrome-headless-shell-mac-arm64/chrome-headless-shell')) : [];
  return [...candidates, '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    '/usr/bin/chromium', '/usr/bin/chromium-browser', '/usr/bin/google-chrome'].find(existsSync);
}

async function browser() {
  const exe = chromePath();
  assert(exe, 'Chrome/Chromium required; set OPSBOX_CHROME');
  const profile = mkdtempSync(join(tmpdir(), 'opsbox-touch-'));
  // Pipe transport avoids a local HTTP listener; single-process supports restricted macOS hosts.
  const proc = spawn(exe, ['--headless=new', '--no-sandbox', '--single-process', '--no-zygote',
    '--in-process-gpu', '--no-first-run', '--remote-debugging-pipe', `--user-data-dir=${profile}`],
  { stdio: ['ignore', 'ignore', 'pipe', 'pipe', 'pipe'] });
  let seq = 0, buffer = '', stderr = '';
  const pending = new Map();
  proc.stderr.on('data', (d) => { stderr += d; });
  const fail = (err) => { for (const p of pending.values()) p.reject(err); pending.clear(); };
  proc.on('error', fail);
  proc.on('exit', (code, signal) => fail(new Error(`Chrome exited ${code || signal}: ${stderr}`)));
  proc.stdio[3].on('error', fail);
  proc.stdio[4].on('data', (d) => {
    buffer += d;
    let end;
    while ((end = buffer.indexOf('\0')) >= 0) {
      const msg = JSON.parse(buffer.slice(0, end)); buffer = buffer.slice(end + 1);
      const p = pending.get(msg.id);
      if (p) { pending.delete(msg.id); msg.error ? p.reject(new Error(msg.error.message)) : p.resolve(msg.result); }
    }
  });
  let sessionId;
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++seq, timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 10000);
    pending.set(id, { resolve(v) { clearTimeout(timer); resolve(v); }, reject(e) { clearTimeout(timer); reject(e); } });
    proc.stdio[3].write(JSON.stringify({ id, method, params, sessionId }) + '\0');
  });
  const evaluate = async (expression) => {
    const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    assert(!r.exceptionDetails, r.exceptionDetails?.exception?.description || 'Browser evaluation failed');
    return r.result.value;
  };
  const close = async () => {
    if (proc.exitCode === null && proc.signalCode === null) {
      const exited = new Promise((resolve) => proc.once('exit', resolve)); proc.kill('SIGKILL'); await exited;
    }
    rmSync(profile, { recursive: true, force: true });
  };
  try {
    const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
    ({ sessionId } = await send('Target.attachToTarget', { targetId, flatten: true }));
    await send('Page.enable');
    return { send, evaluate, close };
  } catch (err) { await close(); throw err; }
}

test('coarse pointer: every checkbox/radio and label has a 44px hit area; selection covers rows', { timeout: 60000 }, async () => {
  const markup = `${await tables()}<section style="padding:24px;display:grid;gap:12px">
    <label class="opt"><input type="checkbox"> Option</label>
    <div class="field"><label class="opt"><input type="checkbox" checked> Form option</label></div>
    <div class="item-row"><label><input type="checkbox"><span><b>Item option</b></span></label><button>Action</button></div>
    <label class="opt"><input type="radio" name="choice"> Radio one</label>
    <label class="opt"><input type="radio" name="choice" checked> Radio two</label>
    <label class="opt"><input type="checkbox" disabled> Disabled</label>
    <div style="padding:12px"><input type="checkbox" aria-label="Standalone checkbox"></div>
    <div style="padding:12px"><input type="radio" aria-label="Standalone radio"></div>
  </section>`;
  const b = await browser();
  try {
    await b.send('Emulation.setDeviceMetricsOverride', { width: 390, height: 1600, deviceScaleFactor: 1, mobile: false });
    await b.send('Emulation.setTouchEmulationEnabled', { enabled: true });
    await b.send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-reduced-motion', value: 'reduce' }] });
    await b.send('Page.setDocumentContent', { frameId: (await b.send('Page.getFrameTree')).frameTree.frame.id,
      html: `<!doctype html><html data-theme="dark"><meta name="viewport" content="width=device-width"><style id="app-css">${css}</style><body>${markup}</body></html>` });
    assert.equal(await b.evaluate(`matchMedia('(pointer:coarse)').matches`), true, 'pointer:coarse must be active');
    const failures = await b.evaluate(`(() => {
      const errors = [], selector = 'input[type=checkbox],input[type=radio]';
      const inputs = [...document.querySelectorAll(selector)];
      for (const [i, input] of inputs.entries()) {
        input.scrollIntoView({block:'center',inline:'center'});
        const r = input.getBoundingClientRect(), p = getComputedStyle(input, '::before');
        if (r.width !== 22 || r.height !== 22) errors.push(i + ': visible box must stay 22px');
        const w = Math.max(r.width, parseFloat(p.width) || 0), h = Math.max(r.height, parseFloat(p.height) || 0);
        if (w < 44 || h < 44) errors.push(i + ': hit area ' + w + 'x' + h);
        // Hit-test the corners of a centered 44px square; dimensions alone do not prove activation.
        for (const dx of [-21,21]) for (const dy of [-21,21]) {
          const hit = document.elementFromPoint(r.x+r.width/2+dx,r.y+r.height/2+dy);
          if (hit !== input && !input.labels?.[0]?.contains(hit)) errors.push(i + ': missed hit corner');
        }
        for (const label of input.labels || []) {
          const lr = label.getBoundingClientRect();
          if (lr.width < 44 || lr.height < 44) errors.push(i + ': label ' + lr.width + 'x' + lr.height);
        }
      }
      return errors;
    })()`);
    assert.deepEqual(failures, []);

    // Actual pointer events must toggle the control at the label/row edge, including after horizontal scroll.
    const click = async (x, y) => {
      await b.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y }] });
      await b.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
    };
    const inputs = await b.evaluate(`document.querySelectorAll('input[type=checkbox],input[type=radio]').length`);
    for (let i = 0; i < inputs; i++) {
      const point = await b.evaluate(`(() => {
        const input = document.querySelectorAll('input[type=checkbox],input[type=radio]')[${i}];
        input.scrollIntoView({block:'center',inline:'center'}); input.checked = false;
        const r = input.getBoundingClientRect(); return {x:r.x+r.width/2+21,y:r.y+r.height/2+21,disabled:input.disabled};
      })()`);
      await click(point.x, point.y);
      assert.equal(await b.evaluate(`document.querySelectorAll('input[type=checkbox],input[type=radio]')[${i}].checked`), !point.disabled,
        `input ${i}: enlarged corner activates the control; disabled controls stay disabled`);
    }
    for (const selector of ['.files-page tbody tr', '.updates-panel tbody tr']) {
      for (const scroll of [0, 10000]) {
        const point = await b.evaluate(`(() => {
          const row = document.querySelector(${JSON.stringify(selector)}), input = row.querySelector('input');
          const wrapper = row.closest('.tw'); wrapper.scrollLeft = ${scroll}; row.scrollIntoView({block:'center'});
          const r = row.getBoundingClientRect(), w = wrapper.getBoundingClientRect(); input.checked = false;
          const x = Math.min(r.right,w.right)-20, y = r.y+r.height/2;
          return {x,y,hit:document.elementFromPoint(x,y)?.outerHTML};
        })()`);
        await click(point.x, point.y);
        assert.equal(await b.evaluate(`document.querySelector(${JSON.stringify(selector)}).querySelector('input').checked`), true,
          `${selector}: tapping across the row selects it (scroll=${scroll}, ${JSON.stringify(point)})`);
      }
    }
    const labels = await b.evaluate(`document.querySelectorAll('label:has(input:not(:disabled))').length`);
    for (let i = 0; i < labels; i++) {
      const point = await b.evaluate(`(() => {
        const label = document.querySelectorAll('label:has(input:not(:disabled))')[${i}];
        label.scrollIntoView({block:'center'}); label.querySelector('input').checked = false;
        const r = label.getBoundingClientRect(); return {x:r.right-2,y:r.y+r.height/2};
      })()`);
      await click(point.x, point.y);
      assert.equal(await b.evaluate(`document.querySelectorAll('label:has(input:not(:disabled))')[${i}].querySelector('input').checked`), true);
    }
    assert.equal(await b.evaluate(`(() => {
      const a = document.querySelector('.files-page tbody a'); a.scrollIntoView({block:'center',inline:'center'});
      const r = a.getBoundingClientRect(); return document.elementFromPoint(r.x+r.width/2,r.y+r.height/2) === a;
    })()`), true, 'file links remain independently tappable');

    await b.send('Emulation.setTouchEmulationEnabled', { enabled: false });
    await b.send('Emulation.setDeviceMetricsOverride', { width: 1440, height: 1600, deviceScaleFactor: 1, mobile: false });
    assert.equal(await b.evaluate(`matchMedia('(pointer:coarse)').matches`), false);
    await b.evaluate(`(async () => {
      document.activeElement?.blur(); scrollTo(0,0); document.querySelectorAll('.tw').forEach(w=>w.scrollLeft=0);
      await document.fonts.ready;
      document.getAnimations().forEach(a=>a.finish());
      await new Promise(r=>setTimeout(r,500)); // Let the browser's transient tap highlight fade.
      await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
    })()`);
    const desktop = (await b.send('Page.captureScreenshot')).data;
    // Remove coarse media rules via CSSOM: their presence must not change desktop pixels.
    await b.evaluate(`(() => {
      const sheet = document.getElementById('app-css').sheet;
      for(let i=sheet.cssRules.length-1;i>=0;i--) if(sheet.cssRules[i].conditionText?.includes('pointer: coarse')) sheet.deleteRule(i);
    })()`);
    await b.evaluate(`new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)))`);
    const comparison = (await b.send('Page.captureScreenshot')).data;
    assert(comparison === desktop, 'desktop screenshot unchanged by touch rules');
  } finally { await b.close(); }
});
