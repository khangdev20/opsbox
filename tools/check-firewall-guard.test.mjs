// Run: node --experimental-vm-modules --test tools/check-firewall-guard.test.mjs
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import { createContext, runInContext, SourceTextModule } from 'node:vm';

function element() {
  const handlers = new Map();
  return {
    value: '', disabled: true,
    addEventListener(name, fn) { handlers.set(name, [...(handlers.get(name) || []), fn]); },
    fire(name) { for (const fn of handlers.get(name) || []) fn({}); },
    focus() {}, setAttribute() {}, remove() {}, showModal() {},
  };
}

async function panel(accept) {
  const requests = [], dialogs = [], reverts = [];
  const document = {
    cookie: '', activeElement: null,
    body: { append(d) { dialogs.push(d); } },
    createElement() {
      const d = element(), input = element(), yes = element(), no = element();
      d.input = input; d.yes = yes;
      d.querySelector = (s) => s === '#cf' ? input : s === '[data-no]' ? no : yes;
      d.close = (value) => { d.returnValue = value; d.fire('close'); };
      return d;
    },
  };
  const context = createContext({
    document, location: { search: '?mode=http' }, URLSearchParams, console, AbortController,
    setTimeout, clearTimeout, setInterval, clearInterval,
    fetch: async (path, options) => {
      requests.push({ path, method: options.method });
      const ok = path.endsWith('?confirm=true');
      return { ok, status: ok ? 200 : 409, json: async () => ok
        ? { ok: true, revertAt: 123456 }
        : { error: { code: 'confirmation_required', message: 'Removing port 8443 could lock you out.' } } };
    },
  });
  async function module(path) {
    const m = new SourceTextModule(await readFile(new URL(`../web/js/${path}`, import.meta.url), 'utf8'), { context });
    await m.link(() => { throw new Error('Unexpected import'); });
    await m.evaluate();
    return m.namespace;
  }
  const api = await module('api.js'), ui = await module('ui.js');
  const main = await readFile(new URL('../web/js/main.js', import.meta.url), 'utf8');
  // Execute the real boot hook registration; routing is unrelated to this check.
  Object.assign(context, {
    api, ...ui, applyTheme() {}, initialTheme() {}, initUi() {},
    keepOrUndoDialog(options) { reverts.push(options); },
  });
  runInContext(main.slice(0, main.indexOf('const BARE')).replace(/^import .*;$/gm, ''), context);
  const removal = api.removeRule('tcp:8443:anywhere');
  for (let n = 0; n < 20 && !dialogs.length; n++) await new Promise(setImmediate);
  assert.equal(dialogs.length, 1);
  const dialog = dialogs[0];
  assert.match(dialog.innerHTML, /Type <b class="mono">continue<\/b> to confirm/);
  assert.match(dialog.innerHTML, /Removing port 8443/);
  assert.match(dialog.innerHTML, /value="yes" disabled/);
  assert.equal(dialog.yes.disabled, true);
  dialog.input.value = 'wrong'; dialog.input.fire('input');
  assert.equal(dialog.yes.disabled, true);
  dialog.input.value = 'continue'; dialog.input.fire('input');
  assert.equal(dialog.yes.disabled, false);
  dialog.close(accept ? 'yes' : 'no');
  if (accept) await removal;
  else await assert.rejects(removal, /Removing port 8443/);
  return { requests, reverts };
}

test('panel-port deletion requires typed confirmation and opens timed keep dialog', async () => {
  const { requests, reverts } = await panel(true);
  assert.deepEqual(requests, [
    { path: '/api/v1/firewall/rules/tcp%3A8443%3Aanywhere', method: 'DELETE' },
    { path: '/api/v1/firewall/rules/tcp%3A8443%3Aanywhere?confirm=true', method: 'DELETE' },
  ]);
  assert.equal(reverts.length, 1);
  assert.equal(reverts[0].until, 123456);
});

test('cancelling panel-port confirmation never retries the deletion', async () => {
  const { requests, reverts } = await panel(false);
  assert.equal(requests.length, 1);
  assert.equal(reverts.length, 0);
});
