#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
globalThis.location = { search: '?mode=http', protocol: 'https:', port: '8443' };
globalThis.document = { cookie: 'opsbox_csrf=csrf-test' };
const calls = [];
globalThis.fetch = async (url, options) => {
  calls.push({ url, ...options, body: options.body && JSON.parse(options.body) });
  return { ok: true, json: async () => ({ ok: true }) };
};
const dataUrl = (source) => `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
const apiUrl = dataUrl(await readFile(new URL('../web/js/api.js', import.meta.url), 'utf8'));
const api = await import(apiUrl);
await api.getProxyNetwork();
await api.takeProxyPort(80, { kind: 'container', id: 'a'.repeat(64) }, true);
await api.undoProxyTakeover('receipt', true);
await api.keepProxyPorts(true);
assert.deepEqual(calls.map(({ method, url }) => [method, url]), [
  ['GET', '/api/v1/network/proxy'], ['POST', '/api/v1/network/proxy/takeover'],
  ['POST', '/api/v1/network/proxy/undo'], ['POST', '/api/v1/network/proxy/keep'],
]);
for (const c of calls.slice(1)) { assert.equal(c.headers['X-Opsbox-CSRF'], 'csrf-test'); assert.equal(c.body.confirm, true); }
for (const page of ['overview', 'settings']) assert(!api.usageFor(page).sample.includes('getProxyNetwork'));
const uiUrl = dataUrl(await readFile(new URL('../web/js/ui.js', import.meta.url), 'utf8'));
const networkSource = (await readFile(new URL('../web/js/network.js', import.meta.url), 'utf8')).replace("'./ui.js'", JSON.stringify(uiUrl)).replace("'./api.js'", JSON.stringify(apiUrl));
const { networkCard, networkBanner, adoptedDomains } = await import(dataUrl(networkSource));
const data = { network: { port_conflict: true, listen_http: '0.0.0.0:8080', listen_https: '0.0.0.0:8445', conflict_ports: [80], holders: [{ port: 80, pid: 123, process: 'docker-proxy', container: { id: 'a'.repeat(64), name: '<web>', image: 'nginx:1.27' } }] }, undo: [{ id: 'receipt', target: { kind: 'container', id: 'a'.repeat(64) }, previous_policy: 'always' }] };
const card = networkCard(data).toString();
assert(card.includes('webMove') && card.includes('Move behind opsbox'));
assert(card.includes('proxyTakeover') && card.includes('proxyUndo') && card.includes('proxyKeep'));
assert(card.includes('&lt;web&gt;') && !card.includes('<web>'));
assert(networkBanner(data).toString().includes('#/settings?section=network'));
data.network.holders = [{ port: 80, pid: 123, process: 'unknown' }];
assert(!networkCard(data).toString().includes('proxyTakeover'));
data.network.holders[0].unit = 'nginx.service';
assert(networkCard(data).toString().includes('Stop and disable nginx.service'));
data.network.port_conflict = false;
assert.equal(networkBanner(data), '');
assert(!networkCard(data).toString().includes('proxyTakeover'));
console.log('Network API mapping, CSRF, escaped holder names and action visibility passed.');

const adopted = adoptedDomains([{ domain: 'old.test', served_by: '<nginx>', terminate_tls: false }]).toString();
assert(adopted.includes('passthrough') && adopted.includes('adoptedTls') && adopted.includes('&lt;nginx&gt;'));
for (const name of ['discoverWeb', 'moveWeb', 'undoWebMove', 'adoptedTls']) assert(api.isReal(name));
await api.discoverWeb({ kind: 'unit', id: 'nginx.service' });
assert.equal(calls.at(-1).url, '/api/v1/network/proxy/discover');
assert.equal(calls.at(-1).headers['X-Opsbox-CSRF'], 'csrf-test');
globalThis.fetch = async (url, options) => {
  calls.push({ url, ...options, body: options.body && JSON.parse(options.body) });
  return { ok: true, json: async () => url.startsWith('/api/v1/tasks/') ? { status: 'ok', log: [], result: {} } : { task: 'adopt-task' } };
};
await api.moveWeb({ target: { kind: 'unit', id: 'nginx.service' }, domains: ['old.test'], everything_else: false, confirmation: 'move' });
await api.undoWebMove('move-receipt');
await api.adoptedTls('old.test');
const mutations = calls.filter((c) => c.url.endsWith('/move') || c.url.endsWith('/undo-move') || c.url.endsWith('/adopted-tls'));
assert.equal(mutations.length, 3);
for (const c of mutations) assert.equal(c.headers['X-Opsbox-CSRF'], 'csrf-test');
assert.equal(mutations[0].body.confirmation, 'move');
assert.equal(mutations[1].body.confirm, true);
assert.equal(calls.filter((c) => c.url === '/api/v1/tasks/adopt-task').length, 3);
console.log('Adoption task polling, typed confirmation, Undo and TLS API mappings passed.');
