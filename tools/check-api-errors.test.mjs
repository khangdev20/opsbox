// Run: node --experimental-vm-modules --test tools/check-api-errors.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';

const source = (path) => readFile(new URL(`../web/js/${path}`, import.meta.url), 'utf8');
const flush = () => new Promise(setImmediate);

function timers() {
  const pending = new Map(); let id = 0;
  return {
    pending,
    setTimeout(fn, ms) { pending.set(++id, { fn, ms }); return id; },
    clearTimeout(id) { pending.delete(id); },
    fire(ms) {
      for (const [id, timer] of [...pending]) if (timer.ms === ms) {
        pending.delete(id); timer.fn();
      }
    },
  };
}

async function apiPanel(fetch) {
  const clock = timers();
  const context = createContext({ ...clock, AbortController, URLSearchParams,
    location: { search: '?mode=http' }, document: { cookie: '' }, fetch });
  const mod = new SourceTextModule(await source('api.js'), { context });
  await mod.link(() => { throw new Error('Unexpected import'); });
  await mod.evaluate();
  return { api: mod.namespace, clock };
}

test('a fetch that never resolves fails at 30 seconds with a readable error and aborts', async () => {
  let signal;
  const { api, clock } = await apiPanel((url, options) => {
    signal = options.signal;
    return new Promise(() => {});
  });
  const result = api.getHost();
  let settled = false; result.then(() => { settled = true; }, () => { settled = true; });
  await flush();
  assert.equal(settled, false);
  assert([...clock.pending.values()].some((t) => t.ms === 30000), 'missing 30-second deadline');
  const rejected = assert.rejects(result, /The server did not answer in time/);
  clock.fire(30000);
  await rejected;
  assert.equal(signal.aborted, true);
  assert.equal(clock.pending.size, 0);
});

for (const [name, args] of [['getUpdates', []], ['checkOpsboxUpdate', []], ['scanCleanup', [true]], ['getBackups', []]]) {
  test(`${name} has a longer, bounded 120-second deadline`, async () => {
    const { api, clock } = await apiPanel(() => new Promise(() => {}));
    const result = api[name](...args);
    assert([...clock.pending.values()].some((t) => t.ms === 120000), 'missing slow-request deadline');
    const rejected = assert.rejects(result, /The server did not answer in time/);
    clock.fire(120000);
    await rejected;
  });
}

test('deadline covers a stalled response body', async () => {
  const { api, clock } = await apiPanel(async () => ({ ok: true, json: () => new Promise(() => {}) }));
  const result = api.getHost();
  await flush();
  assert([...clock.pending.values()].some((t) => t.ms === 30000));
  const rejected = assert.rejects(result, /The server did not answer in time/);
  clock.fire(30000); await rejected;
});

test('success and HTTP/network errors clear the deadline and preserve messages', async () => {
  for (const [respond, error] of [
    [async () => ({ ok: true, json: async () => ({ name: 'real-host' }) }), null],
    [async () => ({ ok: false, status: 503, json: async () => ({ error: { message: 'Helper unavailable' } }) }), /Helper unavailable/],
    [async () => { throw new Error('offline'); }, /Cannot reach the opsbox server/],
  ]) {
    const { api, clock } = await apiPanel(respond);
    if (error) await assert.rejects(api.getHost(), error);
    else assert.equal((await api.getHost()).name, 'real-host');
    assert.equal(clock.pending.size, 0);
  }
});

function element() {
  const events = new Map();
  return {
    innerHTML: '', style: {}, dataset: {}, classList: { add() {} },
    addEventListener(name, fn) { events.set(name, [...(events.get(name) || []), fn]); },
    async fire(name, event = {}) { for (const fn of events.get(name) || []) await fn(event); },
    replaceChildren(child) { this.innerHTML = child.innerHTML; },
    querySelectorAll() { return []; }, querySelector() { return null; }, contains() { return true; },
  };
}

async function synthetic(context, values) {
  const mod = new SyntheticModule(Object.keys(values), function () {
    for (const [key, value] of Object.entries(values)) this.setExport(key, value);
  }, { context });
  await mod.link(() => {}); await mod.evaluate(); return mod;
}

async function uiModule(context) {
  const mod = new SourceTextModule(await source('ui.js'), { context });
  await mod.link(() => {}); await mod.evaluate(); return mod.namespace;
}

test('a rejected async page mount renders the standard error with a working Retry', async () => {
  const root = element(), retry = element(); let attempts = 0;
  root.querySelector = () => retry;
  const context = createContext({ URLSearchParams, ...timers(),
    setInterval() {}, addEventListener() {}, console: { error() {} },
    location: { hash: '#/system', search: '' },
    window: { addEventListener() {}, scrollTo() {} },
    document: { addEventListener() {}, getElementById: () => root, createElement: element },
  });
  const ui = await uiModule(context);
  const dependencies = {
    './store.js': { applyTheme() {}, initialTheme() {}, session: { user: 'owner' }, server: {}, listeners: [], applySession() {} },
    './api.js': { MODE: 'mock', setUiHooks() {}, resetUsage() {}, tick() {} },
    './shell.js': { mountShell: () => root, unmountShell() {}, setNav() {}, refreshStatus() {} },
    './ui.js': { ...ui, initUi() {} },
  };
  const page = await synthetic(context, { view: async () => ({ title: 'System', html: '<main>Loaded</main>',
    async mount() { if (++attempts === 1) throw new Error('Mount <failed>'); },
  }) });
  const modules = new Map();
  for (const [name, values] of Object.entries(dependencies)) modules.set(name, await synthetic(context, values));
  const main = new SourceTextModule((await source('main.js')).replace(/\nboot\(\);/, '\n') + '\nexport { navigate };', {
    context, importModuleDynamically: () => page,
  });
  await main.link((name) => modules.get(name)); await main.evaluate();
  await main.namespace.navigate(); await flush();
  assert.match(root.innerHTML, /Something went wrong/);
  assert.match(root.innerHTML, /Mount &lt;failed&gt;/);
  assert.match(root.innerHTML, />Retry</);
  await retry.fire('click');
  assert.equal(attempts, 2);
  assert.match(root.innerHTML, /Loaded/);
});

test('System replaces failed initial and later panes with an error and Retry recovers', async () => {
  const root = element(), body = element(), tabbar = element();
  body.innerHTML = '<div class="sk-sec">Loading</div>';
  root.querySelector = (s) => s === '#b' ? body : s === '#t' ? tabbar : null;
  const context = createContext({ location: { hash: '#/system/updates' }, history: { replaceState() {} } });
  const ui = await uiModule(context);
  let failed = true;
  const api = { async getUpdates() {
    if (failed) throw new Error('The server <failed>');
    return { list: [], total: 0, security: 0, checked: 'now', running: 'test' };
  }, async getUnits() { throw new Error('Services unavailable'); },
  async getCapacity() { throw new Error('Capacity unavailable'); }, async listContainers() { return []; } };
  const modules = new Map([
    ['../ui.js', await synthetic(context, { ...ui, patch: (node, markup) => { node.innerHTML = markup; } })],
    ['../api.js', await synthetic(context, api)],
    ['../capacity.js', await synthetic(context, { capacityPage() { return 'Capacity'; }, addSwap() {}, editMemory() {}, bindMemoryMap() {} })],
  ]);
  const mod = new SourceTextModule(await source('pages/system.js'), { context });
  await mod.link((name) => modules.get(name)); await mod.evaluate();
  const page = await mod.namespace.view({ path: '/system/updates' });
  await page.mount(root);
  assert.match(body.innerHTML, /The server &lt;failed&gt;/);
  assert.match(body.innerHTML, />Retry</);
  assert(!body.innerHTML.includes('sk-sec'));
  const click = async (act, data = {}) => root.fire('click', { target: {
    dataset: { act, ...data }, closest() { return this; }, hasAttribute() { return false; }, getAttribute() { return null; },
  } });
  failed = false; await click('retry');
  assert.match(body.innerHTML, /Everything is up to date/);
  await click('tab', { i: '1' });
  assert.match(body.innerHTML, /Services unavailable/);
  assert.match(body.innerHTML, />Retry</);
  assert(!body.innerHTML.includes('Everything is up to date'));
  await click('tab', { i: '2' });
  assert.match(body.innerHTML, /Capacity unavailable/);
  assert.match(body.innerHTML, />Retry</);
});
