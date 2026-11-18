#!/usr/bin/env node
// Exercise the real API boundary without a browser or external services.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
globalThis.location = { search: '?mode=http', protocol: 'http:', port: '8443' };
globalThis.document = { cookie: 'opsbox_csrf=csrf-test' };
const calls = [];
globalThis.fetch = async (url, options) => {
  calls.push({ url, ...options, body: options.body && JSON.parse(options.body) });
  return { ok: true, json: async () => ({ ok: true }) };
};
const source = await readFile(new URL('../web/js/api.js', import.meta.url), 'utf8');
const api = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
assert.deepEqual(api.usageFor('alerts').sample, []);
await api.getAlerts();
await api.setRule('disk', false);
await api.connectChannel('c4', { kind: 'webhook', url: 'https://example.com', secret: 'signing-secret' });
await api.testAlert('c4');
await api.silenceAlert('disk:/', 3600);
await api.setQuiet({ from: '23:00', to: '07:00' });
await api.disconnectChannel('c4');
assert.deepEqual(calls.map(({ url, method }) => [method, url]), [
  ['GET', '/api/v1/alerts'], ['PUT', '/api/v1/alerts/rules/disk'],
  ['PUT', '/api/v1/alerts/channels/c4'], ['POST', '/api/v1/alerts/test'],
  ['POST', '/api/v1/alerts/silence'], ['PUT', '/api/v1/alerts/quiet'],
  ['DELETE', '/api/v1/alerts/channels/c4'],
]);
assert.deepEqual(calls[1].body, { on: false });
assert.deepEqual(calls[3].body, { channel: 'c4' });
assert.deepEqual(calls[4].body, { id: 'disk:/', seconds: 3600 });
for (const call of calls.slice(1)) assert.equal(call.headers['X-Opsbox-CSRF'], 'csrf-test');
console.log('Alerts UI API contracts and live-data badge passed.');
