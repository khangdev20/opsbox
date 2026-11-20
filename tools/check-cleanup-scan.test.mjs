// Run: node --experimental-vm-modules --test tools/check-cleanup-scan.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { createContext, SourceTextModule, SyntheticModule } from 'node:vm';

const source = (path) => readFile(new URL(`../web/js/${path}`, import.meta.url), 'utf8');
const flush = () => new Promise(setImmediate);
const scan = (disk = 12, pending = false) => ({ items: [], disk, total: 20, pending });

async function panel() {
  const requests = [], timers = new Map(), toasts = [];
  let timerId = 0;
  const context = createContext({ AbortController, URLSearchParams,
    location: { search: '?mode=http' }, document: { cookie: '' },
    setTimeout(fn, ms) { timers.set(++timerId, { fn, ms }); return timerId; },
    clearTimeout(id) { timers.delete(id); },
    fetch(url, options) {
      // Deliberately ignore abort: exercise late-response/cache protection too.
      return new Promise((resolve, reject) => requests.push({ url, signal: options.signal,
        respond: (data) => resolve({ ok: true, json: async () => data }), reject }));
    },
  });
  async function load(path, dependencies = {}) {
    const mod = new SourceTextModule(await source(path), { context });
    await mod.link(async (name) => {
      const values = dependencies[name];
      assert(values, `Unexpected import: ${name}`);
      const stub = new SyntheticModule(Object.keys(values), function () {
        for (const [key, value] of Object.entries(values)) this.setExport(key, value);
      }, { context });
      await stub.link(() => {}); await stub.evaluate(); return stub;
    });
    await mod.evaluate(); return mod.namespace;
  }
  const api = await load('api.js'), ui = await load('ui.js');
  async function open(overrides = {}) {
    const body = { innerHTML: 'Scanning disk…' };
    const root = { querySelector: (s) => s === '#b' ? body : null,
      querySelectorAll: () => [], addEventListener() {} };
    const cleanup = await load('pages/cleanup.js', {
      '../api.js': { ...api, ...overrides },
      '../ui.js': { ...ui, patch: (node, markup) => { node.innerHTML = markup; },
        toast: (...args) => toasts.push(args), setSwitch() {} },
    });
    const page = await cleanup.view(); page.mount(root);
    return { page, body };
  }
  return { api, requests, timers, toasts, open };
}

test('concurrent fresh scans share one request and preserve the completed cache', async () => {
  const p = await panel();
  const result = Promise.all([p.api.scanCleanup(true), p.api.scanCleanup(true)]);
  assert.equal(p.requests.length, 1, 'fresh scans must share an in-flight request');
  assert.match(p.requests[0].url, /\/cleanup\/scan\?fresh=1$/);
  p.requests[0].respond(scan());
  const [first, second] = await result;
  assert.equal(first.disk, 12); assert.equal(second.disk, 12);
  first.items.push({ id: 'local' });
  assert.equal(second.items.length, 0, 'callers must receive independent results');
  assert.equal((await p.api.scanCleanup()).items.length, 0);
  assert.equal(p.requests.length, 1);
  assert.equal(p.timers.size, 0);
});

test('a cached read joins an in-flight fresh scan instead of returning stale data', async () => {
  const p = await panel();
  const initial = p.api.scanCleanup(); p.requests[0].respond(scan()); await initial;
  const fresh = p.api.scanCleanup(true), read = p.api.scanCleanup();
  p.requests[1].respond(scan(15));
  assert.equal((await fresh).disk, 15);
  assert.equal((await read).disk, 15);
  assert.equal(p.requests.length, 2);
});

test('failed shared scans release the slot so a deliberate retry can succeed', async () => {
  const p = await panel();
  const results = Promise.allSettled([p.api.scanCleanup(true), p.api.scanCleanup(true)]);
  p.requests[0].reject(new Error('offline'));
  await flush();
  assert.equal(p.requests.length, 1);
  for (const result of await results) {
    assert.equal(result.status, 'rejected');
    assert.match(result.reason.message, /Cannot reach the opsbox server/);
  }
  const retry = p.api.scanCleanup(true);
  p.requests[1].respond(scan()); await retry;
  assert.equal(p.timers.size, 0);
});

test('leaving Cleanup aborts the scan; reopening is safe even if the old fetch resolves late', async () => {
  const p = await panel(), old = await p.open();
  assert.equal(p.requests.length, 1);
  old.page.unmount(); await flush();
  assert.equal(p.requests[0].signal.aborted, true, 'navigation must abort outstanding fetch');
  assert.equal(p.timers.size, 0, 'navigation must clear the request deadline too');
  const current = await p.open();
  assert.equal(p.requests.length, 2, 'reopening must start a usable request');
  p.requests[0].respond(scan(1, true)); await flush();
  const joined = p.api.scanCleanup(true);
  assert.equal(p.requests.length, 2, 'late settlement must not clear the newer in-flight slot');
  p.requests[1].respond(scan(15)); await joined; await flush();
  assert.equal(old.body.innerHTML, 'Scanning disk…');
  assert.match(current.body.innerHTML, /15 of 20 GB used/);
  assert.equal((await p.api.scanCleanup()).disk, 15);
  assert.equal(p.toasts.length, 0);
  assert.equal(p.timers.size, 0, 'late pending result must not schedule polling');
  current.page.unmount();
});

test('a completed late fetch cannot replace the cache from a newer mount', async () => {
  const p = await panel(), old = await p.open();
  old.page.unmount(); await flush();
  const current = await p.open();
  p.requests[1].respond(scan(15)); await flush();
  p.requests[0].respond(scan(1)); await flush();
  assert.equal((await p.api.scanCleanup()).disk, 15);
  assert.equal(p.requests.length, 2);
  assert.equal(old.body.innerHTML, 'Scanning disk…');
  current.page.unmount();
});

test('dead pages ignore late scan results and never schedule another poll', async () => {
  const p = await panel(); let resolve;
  const pending = new Promise((r) => { resolve = r; });
  const old = await p.open({ scanCleanup: () => pending, cancelCleanupScan() {} });
  old.page.unmount(); resolve(scan(1, true)); await flush();
  assert.equal(old.body.innerHTML, 'Scanning disk…');
  assert.equal(p.timers.size, 0);
  assert.equal(p.toasts.length, 0);
});
