// Run: node --experimental-vm-modules --test tools/check-apps-domains.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';

const source = (path) => readFile(new URL(`../web/js/${path}`, import.meta.url), 'utf8');
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const flush = () => new Promise(setImmediate);

async function panel({ delay = 10, requestLimit = Infinity } = {}) {
  const events = new Map(), detail = { innerHTML: '' }, graph = { innerHTML: '' };
  const root = {
    querySelector: (s) => s === '#d' ? detail : s === '#g' ? graph : null,
    addEventListener: (name, fn) => events.set(name, fn), contains: () => true,
  };
  const requests = [], listeners = new Set();
  let failing = true;
  const context = createContext({ URLSearchParams, AbortController,
    location: { search: '?mode=http' }, document: { cookie: '' },
    setTimeout: (fn, ms) => setTimeout(fn, ms).unref(), clearTimeout,
    fetch: async (url) => {
      requests.push(url);
      // Bound the unfixed loop so a regression cannot flood the test process.
      if (requests.length > requestLimit) return new Promise(() => {});
      await wait(delay);
      const service = url.split('/').at(-2);
      return { ok: !failing, status: failing ? 500 : 200,
        json: async () => failing ? { error: { message: 'Domain <unavailable>' } }
          : { service, domains: [], port: 3000, internalHost: service } };
    },
  });
  async function load(path, deps = {}) {
    const mod = new SourceTextModule(await source(path), { context });
    await mod.link(async (name) => {
      const values = deps[name];
      assert(values, `Unexpected import: ${name}`);
      const stub = new SyntheticModule(Object.keys(values), function () {
        for (const [key, value] of Object.entries(values)) this.setExport(key, value);
      }, { context });
      await stub.link(() => {}); await stub.evaluate(); return stub;
    });
    await mod.evaluate(); return mod.namespace;
  }
  const ui = await load('ui.js'), api = await load('api.js');
  const project = { name: 'test', env: 'test', services: ['web', 'api'].map((id, x) => ({
    id, x: x * 300, y: 0, kind: 'app', state: 'active', port: 3000, vars: [], deploys: [],
  })) };
  const apps = await load('pages/apps.js', {
    '../api.js': { ...api, getProject: async () => project },
    '../ui.js': { ...ui, patch: (node, markup) => { node.innerHTML = markup; } },
    '../network.js': { adoptedDomains: () => '' },
    '../capacity.js': { editMemory() {}, withCapacityOverride() {} },
    '../store.js': { listeners },
    '../secrets.js': { withReveal() {}, copyPlain() {}, hideRevealed() {} },
  });
  const page = await apps.view({ query: { tab: 'domains' } });
  page.mount(root);
  return {
    page, requests, detail, listeners, recover: () => { failing = false; },
    async click(act, data = {}) {
      await events.get('click')({ target: { dataset: { act, ...data },
        closest() { return this; }, hasAttribute: () => false, getAttribute: () => null } });
      await wait(30);
    },
    async poll() { for (const fn of listeners) await fn(); await wait(30); await flush(); },
  };
}

// Isolate the broken implementation's unhandled rejections so the request-count
// assertion still runs for the full three seconds during the test-first run.
if (process.argv.includes('--probe')) {
  const rejections = [];
  process.on('unhandledRejection', (err) => rejections.push(err.message));
  const p = await panel({ requestLimit: 5 });
  await wait(3000);
  p.page.unmount();
  console.log(JSON.stringify({ requests: p.requests, markup: p.detail.innerHTML, rejections }));
} else {
  test('HTTP 500 makes one domain request over 3 seconds and shows escaped error with Retry', async () => {
    const { stdout } = await promisify(execFile)(process.execPath,
      ['--experimental-vm-modules', import.meta.filename, '--probe'], { timeout: 10000 });
    const result = JSON.parse(stdout);
    assert.equal(result.requests.length, 1, 'domain failure must not cause an endless reload loop');
    assert.match(result.requests[0], /\/services\/web\/domains$/);
    assert.equal(result.rejections.length, 0, 'domain failures must be handled');
    assert.match(result.markup, /role="alert"/);
    assert.match(result.markup, /Domain &lt;unavailable&gt;/);
    assert.match(result.markup, />Retry</);
  });

  test('failure survives tab/service changes; Retry makes one request and recovers', async () => {
    const p = await panel();
    try {
      await wait(30);
      await p.click('tab', { i: '0' }); await p.click('tab', { i: '2' });
      assert.equal(p.requests.length, 1);
      await p.click('select', { id: 'api' }); await p.click('tab', { i: '2' });
      assert.equal(p.requests.length, 2);
      await p.click('select', { id: 'web' }); await p.click('tab', { i: '2' });
      assert.equal(p.requests.length, 2, 'failure is remembered per service');
      assert.match(p.detail.innerHTML, />Retry</);
      await p.click('retrydomains');
      assert.equal(p.requests.length, 3);
      assert.match(p.detail.innerHTML, />Retry</);
      p.recover(); await p.click('retrydomains');
      assert.equal(p.requests.length, 4);
      assert.match(p.detail.innerHTML, /No public domains yet/);
      assert.doesNotMatch(p.detail.innerHTML, /Domain &lt;unavailable&gt;/);
    } finally { p.page.unmount(); }
  });

  test('each poll retries a failed domain once even when project data is unchanged', async () => {
    const p = await panel();
    try {
      await wait(30); assert.equal(p.requests.length, 1);
      await p.poll(); assert.equal(p.requests.length, 2);
      assert.match(p.detail.innerHTML, />Retry</);
      await p.poll(); assert.equal(p.requests.length, 3);
      p.recover(); await p.poll(); assert.equal(p.requests.length, 4);
      assert.match(p.detail.innerHTML, /No public domains yet/);
      await p.poll(); assert.equal(p.requests.length, 4, 'successful domains are not needlessly refetched');
    } finally { p.page.unmount(); }
  });

  test('an in-flight domain request is not duplicated by Retry, polling or repainting', async () => {
    const p = await panel({ delay: 200 });
    try {
      await p.click('retrydomains');
      await p.poll();
      await p.click('tab', { i: '0' }); await p.click('tab', { i: '2' });
      assert.equal(p.requests.length, 1);
      await wait(200);
      assert.match(p.detail.innerHTML, />Retry</);
    } finally { p.page.unmount(); }
  });
}
