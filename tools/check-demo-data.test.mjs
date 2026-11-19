// Run: node --experimental-vm-modules --test tools/check-demo-data.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { createContext, SourceTextModule } from 'node:vm';

const samples = ['srv-sgn-01', 'acme/shop', 'acme-co', 'opsbox-demo', 'panel.example.com'];
function noSamples(markup) {
  for (const sample of samples) assert(!String(markup).includes(sample), `Leaked sample: ${sample}`);
}

// Only the DOM surface these views use; templates and API calls execute unchanged.
function element() {
  const attrs = new Map();
  return {
    innerHTML: '', textContent: '', dataset: {},
    classList: { toggle() {}, contains() { return false; } },
    addEventListener() {}, querySelector() { return null; },
    setAttribute(k, v) { attrs.set(k, v); }, getAttribute(k) { return attrs.get(k); },
  };
}
async function panel(mode, respond = async () => { throw new Error('API unavailable'); }) {
  const nodes = new Map(['helper-banner', 'theme-btn', 'open-pal', 'side-toggle', 'view'].map((id) => [id, element()]));
  const shellNode = element(), crumb = element();
  const document = {
    cookie: '', documentElement: { dataset: {} },
    addEventListener() {}, removeEventListener() {},
    getElementById: (id) => nodes.get(id) || null,
    querySelector: (s) => s === '.shell' ? shellNode : s === '.crumb' ? crumb : null,
  };
  const requests = [], sockets = [];
  const context = createContext({
    document, window: { addEventListener() {} },
    location: { search: `?mode=${mode}`, protocol: 'https:', port: '8443', host: 'real-panel.test', hash: '#/' },
    URLSearchParams, console, setTimeout, clearTimeout,
    setInterval() { return 0; }, clearInterval() {},
    WebSocket: class {
      constructor(url) { this.url = url; sockets.push(this); }
      close() { this.closed = true; }
    },
    fetch: async (url, options) => {
      requests.push(url);
      const data = await respond(url, options);
      return { ok: true, json: async () => data };
    },
  });
  const modules = new Map();
  async function load(url) {
    if (!modules.has(url.href)) {
      modules.set(url.href, new SourceTextModule(await readFile(url, 'utf8'), { context, identifier: url.href }));
      await modules.get(url.href).link((specifier, parent) => load(new URL(specifier, parent.identifier)));
    }
    return modules.get(url.href);
  }
  async function get(path) {
    const module = await load(new URL(`../web/js/${path}`, import.meta.url));
    if (module.status !== 'evaluated') await module.evaluate();
    return module.namespace;
  }
  return { get, requests, sockets, crumb, mountShell(shell) {
    const app = element(); app.querySelector = () => shellNode;
    shell.mountShell(app);
    return app;
  } };
}

test('HTTP shell has no sample name before or after hostname request failure', async () => {
  const p = await panel('http');
  const shell = await p.get('shell.js');
  const app = p.mountShell(shell);
  noSamples(app.innerHTML);
  assert.match(app.innerHTML, /This server/);
  await new Promise(setImmediate);
  noSamples(app.innerHTML + p.crumb.textContent);
  assert(p.requests.includes('/api/v1/host'));
  shell.unmountShell();
});

test('HTTP configure without a repository requires choosing a source', async () => {
  const p = await panel('http');
  const configure = await p.get('pages/configure.js');
  const page = await configure.view({ query: {} });
  noSamples(page.html);
  assert.match(String(page.html), /Choose a repository/);
  assert(!String(page.html).includes('type="submit"'));
  const selected = await configure.view({ query: { repo: 'real/project' } });
  assert.match(String(selected.html), /Configure real\/project/);
});

test('HTTP settings rejects API failure without sample fallback', async () => {
  const p = await panel('http');
  const settings = await p.get('pages/settings.js');
  await assert.rejects(settings.view(), /Cannot reach the opsbox server/);
});

for (const github of [true, 'connected', 1, null, [], { connected: true, installed: true, account: 'real-owner', appSlug: 'real-app' }]) {
  test(`HTTP settings renders only real GitHub state: ${JSON.stringify(github)}`, async () => {
    const p = await panel('http', async (url) => {
      if (url === '/api/v1/settings') return { hostname: 'real-host', tz: 'UTC', tokens: [], github };
      throw new Error('API unavailable');
    });
    const settings = await p.get('pages/settings.js');
    const page = await settings.view();
    const root = element(), body = element(); root.querySelector = () => body;
    page.mount(root);
    noSamples(String(page.html) + body.innerHTML);
    assert.match(body.innerHTML, typeof github === 'object' && github?.account ? /real-owner/ : /Not connected/);
    page.unmount();
  });
}

for (const name of ['buildLog', 'termExec', 'rotateOutKey']) {
  test(`HTTP ${name} refuses to simulate even when API is unavailable`, async () => {
    const p = await panel('http'), api = await p.get('api.js');
    const args = name === 'buildLog' ? [{ repo: 'real/repo' }, { at: 0, id: 'd1' }]
      : name === 'termExec' ? ['local', '/home/deploy', 'hostname'] : ['unused'];
    await assert.rejects(async () => api[name](...args), /needs a real server/i);
    assert.equal(p.requests.length, 0);
  });
}

test('HTTP deploy log wrapper still requests real logs and propagates failure', async () => {
  const p = await panel('http'), api = await p.get('api.js');
  assert.equal(api.isReal('buildLog'), false);
  assert.equal(api.isReal('fetchBuildLog'), true);
  assert.equal(api.usageFor('apps').sample.length, 0);
  await assert.rejects(api.fetchBuildLog({}, { id: 'd1' }), /Cannot reach the opsbox server/);
  assert.deepEqual(p.requests, ['/api/v1/deploys/d1/log']);
});

test('HTTP deploy log stream still opens the real server endpoint', async () => {
  const p = await panel('http'), api = await p.get('api.js');
  const lines = [];
  const close = api.streamDeployLog('d1', (line) => lines.push(line.line));
  assert.equal(p.sockets[0].url, 'wss://real-panel.test/api/v1/stream/deploys/d1');
  p.sockets[0].onmessage({ data: JSON.stringify({ type: 'log', line: 'real build output' }) });
  assert.deepEqual(lines, ['real build output']);
  close();
  assert.equal(p.sockets[0].closed, true);
});

test('demo mode keeps sample host, repository, GitHub settings and mock operations', async () => {
  const p = await panel('mock'), api = await p.get('api.js');
  const shell = await p.get('shell.js');
  assert.match(p.mountShell(shell).innerHTML, /srv-sgn-01/);
  const configure = await p.get('pages/configure.js');
  assert.match(String((await configure.view({ query: {} })).html), /acme\/shop/);
  const settings = await p.get('pages/settings.js'), page = await settings.view();
  const root = element(), body = element(); root.querySelector = () => body;
  page.mount(root);
  assert.match(String(page.html) + body.innerHTML, /acme-co/);
  assert.match(body.innerHTML, /panel\.example\.com/);
  const logs = api.buildLog({ repo: 'acme/shop', id: 'web', build: 'Dockerfile' }, { at: 0, id: 'd1' });
  assert(logs.some((line) => line.includes('acme/shop')));
  assert.equal((await api.termExec('local', '/', 'hostname')).out, 'srv-sgn-01');
  await api.rotateOutKey('o1');
  assert.equal(p.requests.length, 0);
  page.unmount(); shell.unmountShell();
});
