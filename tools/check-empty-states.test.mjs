// Run: node --experimental-vm-modules --test tools/check-empty-states.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';

const source = (path) => readFile(new URL(`../web/js/${path}`, import.meta.url), 'utf8');

// The views' DOM surface; real templates, empty() and delegated actions run unchanged.
function element() {
  const events = new Map();
  return {
    innerHTML: '', dataset: {}, classList: { toggle() {} },
    addEventListener(name, fn) { events.set(name, [...(events.get(name) || []), fn]); },
    async fire(name, event) { for (const fn of events.get(name) || []) await fn(event); },
    querySelector() { return null; }, querySelectorAll() { return []; }, contains() { return true; },
  };
}

async function panel(name, { api = {}, role = 'admin', args = {} } = {}) {
  const context = createContext({ location: { hash: '' }, history: { replaceState() {} } });
  async function load(path, deps = {}) {
    const mod = new SourceTextModule(await source(path), { context });
    await mod.link(async (name) => {
      assert(deps[name], `Unexpected import: ${name}`);
      const values = deps[name];
      const stub = new SyntheticModule(Object.keys(values), function () {
        for (const [key, value] of Object.entries(values)) this.setExport(key, value);
      }, { context });
      await stub.link(() => {}); await stub.evaluate(); return stub;
    });
    await mod.evaluate(); return mod.namespace;
  }
  const ui = await load('ui.js'), states = [], forms = [];
  const module = await load(`pages/${name}.js`, {
    '../ui.js': { ...ui, patch: (node, markup) => { node.innerHTML = markup; }, toast() {},
      empty(text, action) { const markup = ui.empty(text, action).toString(); states.push(markup); return ui.raw(markup); },
      async askForm(form) { forms.push(form); return false; },
    },
    '../api.js': { MODE: 'http', ...api },
    '../store.js': { server: { role } },
    '../vendor-loader.js': { loadAmd() { throw new Error('Editor should not load'); } },
    '../capacity.js': { capacityPage() {}, addSwap() {}, editMemory() {}, bindMemoryMap() {} },
  });
  const root = element(), nodes = new Map(['#b', '#t', '#c', '#l', '#e', '#up', '#del'].map((s) => [s, element()]));
  let uploads = 0;
  nodes.get('#up').click = () => { uploads++; };
  root.querySelector = (s) => nodes.get(s) || null;
  const page = await module.view({ query: {}, ...args });
  root.innerHTML = String(page.html);
  await page.mount(root);
  return {
    states, nodes, forms, uploads: () => uploads,
    async click(act, data = {}) {
      await root.fire('click', { target: { dataset: { act, ...data }, closest() { return this; },
        hasAttribute() { return false; }, getAttribute() { return null; } } });
    },
  };
}

function guidance(p, message, action, label, target = '#b') {
  const state = p.states.findLast((markup) => markup.includes(message));
  assert(state, `Expected empty() guidance: ${message}`);
  assert.match(state, /class="empty"/);
  assert.match(state, new RegExp(`<button[^>]*data-act="${action}"[^>]*>${label}</button>`));
  assert(p.nodes.get(target).innerHTML.includes(state), 'guidance must be mounted in the page');
}

for (const dir of ['/', '/empty-folder']) {
  test(`Files empty folder ${dir} explains absence and opens upload picker`, async () => {
    const p = await panel('files', { args: { query: { p: dir } }, api: { listDir: async () => ({ entries: [] }) } });
    guidance(p, 'This folder is empty.', 'upload', 'Upload a file', '#l');
    await p.click('upload');
    assert.equal(p.uploads(), 1);
    assert.match(p.nodes.get('#c').innerHTML, /aria-label="Path"/);
  });
}

test('System no updates explains success and can check again', async () => {
  let calls = 0;
  const p = await panel('system', { args: { path: '/system/updates' }, api: { getUpdates: async () => {
    calls++; return { list: [], total: 0, security: 0, checked: 'now', running: 'test' };
  } } });
  guidance(p, 'Everything is up to date.', 'retry', 'Check again');
  await p.click('retry'); assert.equal(calls, 2);
});

test('System no jobs offers the existing Add job form', async () => {
  const p = await panel('system', { args: { path: '/system/jobs' }, api: { getJobs: async () => [] } });
  guidance(p, 'No scheduled jobs yet.', 'addjob', 'Add job');
  await p.click('addjob'); assert.equal(p.forms[0].title, 'Add job');
});

test('System empty units result offers a working refresh', async () => {
  let units = [];
  const p = await panel('system', { args: { path: '/system/services' }, api: { getUnits: async () => units } });
  guidance(p, 'No services found.', 'retry', 'Refresh services');
  units = [{ name: 'example.service', state: 'active', boot: 'enabled' }];
  await p.click('retry');
  assert.match(p.nodes.get('#b').innerHTML, /example.service/);
  assert.doesNotMatch(p.nodes.get('#b').innerHTML, /class="empty"/);
});

test('System unit log filter with no match offers Clear filters and recovers', async () => {
  const queries = [];
  const p = await panel('system', { args: { path: '/system/services' }, api: {
    getUnits: async () => [{ name: 'example.service', state: 'active', boot: 'enabled' }],
    getJournal: async (q) => { queries.push({ ...q }); return q.unit ? [] : [{ t: 0, proc: 'example', msg: 'Started' }]; },
  } });
  await p.click('unitlog', { n: 'example.service' });
  guidance(p, 'No log entries match these filters.', 'clearfilters', 'Clear filters');
  await p.click('clearfilters');
  assert.deepEqual(queries.at(-1), { unit: '', prio: 6, q: '' });
  assert.match(p.nodes.get('#b').innerHTML, /Started/);
});

for (const role of ['admin', 'deployer']) {
  test(`Backups no snapshots gives ${role} a usable next action`, async () => {
    let loads = 0, runs = 0;
    const p = await panel('backups', { role, api: {
      getBackups: async () => { loads++; return { jobs: [], items: [], dests: [] }; },
      checkCapacity: async () => ({ fit: { status: 'ok' } }), runBackup: async () => { runs++; },
    } });
    if (role === 'admin') {
      guidance(p, 'No snapshots yet. Run a backup now.', 'run', 'Back up now');
      await p.click('run', { id: '' }); assert.equal(runs, 1);
    } else {
      guidance(p, 'No snapshots yet. Ask an administrator to run a backup.', 'refresh', 'Refresh');
      assert.doesNotMatch(p.nodes.get('#b').innerHTML, /data-act="run"/);
      await p.click('refresh'); assert.equal(runs, 0);
    }
    assert.equal(loads, 2);
  });
}
