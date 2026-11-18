#!/usr/bin/env node
// Each reason the update check can give is rendered with its own text (prompt 03e).
// Pure functions only, so no browser is needed. Run: node tools/check-update-status.mjs

import assert from 'node:assert/strict';
import { checkToast, checkedLine, clock, noOfferText } from '../web/js/update-status.js';

const base = { current: '0.1.0-alpha.20', channel: 'beta', checkedChannel: 'beta', checkedAt: 1_700_000_000, latest: null, updateAvailable: false };
const text = (o) => noOfferText({ ...base, ...o }).text;

// A fixed local time: build the expectation the same way the page does.
const d = new Date(1_700_000_000 * 1000);
const hhmm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
assert.equal(clock(1_700_000_000), hhmm);
assert.match(clock(1_700_000_000), /^\d\d:\d\d$/);
assert.equal(clock(null), '');
assert.equal(clock(0), '');

// no release on the channel: stable points at beta, beta says none published
assert.equal(text({ reason: 'no_releases_on_channel', channel: 'stable', checkedChannel: 'stable' }),
  'No stable release yet; switch to beta to get alpha releases');
assert.match(text({ reason: 'no_releases_on_channel' }), /No release has been published on the beta channel yet/);

// newest is older or equal
const utd = text({ reason: 'up_to_date', latest: { version: '0.1.0-alpha.20' } });
assert.match(utd, /up to date/);
assert.match(utd, /newest on the beta channel: 0\.1\.0-alpha\.20/);

// network error, with the technical detail on its own line
const net = noOfferText({ ...base, reason: 'network_error', reasonDetail: 'connection refused' });
assert.match(net.text, /Couldn't reach GitHub/);
assert.equal(net.detail, 'connection refused');

// rate limited, with the time to retry
const rl = text({ reason: 'rate_limited', retryAt: 1_700_000_000 });
assert.match(rl, /GitHub is limiting update checks/);
assert.ok(rl.includes(`after ${hhmm}`), rl);
assert.match(text({ reason: 'rate_limited' }), /try again later/);

// development build, never checked, and an older server without reasons
assert.match(text({ reason: 'dev_build', current: '0.1.0-dev+abc' }), /development build/);
assert.match(text({ reason: null, checkedAt: null }), /not checked yet/);
assert.match(text({ reason: null }), /up to date/);

// every reason has a different text
const reasons = ['no_releases_on_channel', 'up_to_date', 'network_error', 'rate_limited', 'dev_build'];
assert.equal(new Set(reasons.map((reason) => text({ reason }))).size, reasons.length);

// "last checked HH:MM" and the channel
assert.equal(checkedLine(base), `last checked ${hhmm} · beta channel`);
assert.equal(checkedLine({ ...base, checkedAt: null }), 'beta channel');

// toast after "Check now"
assert.equal(checkToast({ ...base, updateAvailable: true, latest: { version: '0.1.0-alpha.21' } }), 'opsbox 0.1.0-alpha.21 is available.');
assert.match(checkToast({ ...base, reason: 'up_to_date' }), /^Up to date/);
assert.match(checkToast({ ...base, reason: 'network_error' }), /^Couldn't reach GitHub/);

console.log('update status text: ok');
