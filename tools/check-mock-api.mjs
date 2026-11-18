#!/usr/bin/env node
/**
 * Fail when a panel page calls an api.js export that is mock-only in http mode
 * (not listed in REAL), unless the name is on the explicit allowlist.
 */
import { readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const apiSource = await readFile(join(root, 'web/js/api.js'), 'utf8');

const realMatch = apiSource.match(/const REAL = new Set\(\[([\s\S]*?)\]\);/);
if (!realMatch) {
  console.error('Could not find REAL set in web/js/api.js');
  process.exit(1);
}
const REAL = new Set(
  [...realMatch[1].matchAll(/'([^']+)'/g)].map((m) => m[1]),
);

/** Non-capability exports (mode flags, helpers, constants). */
const HELPERS = new Set([
  'MODE', 'isReal', 'PAGE_USES', 'resetUsage', 'usageFor', 'quiet',
  'formatAuditIp', 'setUiHooks', 'downloadUrl', 'TEMPLATES', 'auth',
]);

/** Stream / wrapper names that ride on a REAL capability. */
const WRAPPERS = {
  streamJournal: 'getJournal',
  streamContainerLogs: 'containerLogs',
  streamDeployLog: 'buildLog',
  fetchBuildLog: 'buildLog',
  buildLog: 'buildLog',
  startMetricsStream: 'getMetrics',
};

/**
 * Page-referenced names that stay mock-only on purpose.
 * Seeded after 16h–16j: Desktop outgoing keys / other servers, and ?mode=mock demos.
 */
const ALLOWLIST = {
  simulateFailure: 'demo-only (?mode=mock Safe Mode)',
  recoveryAction: 'demo-only (?mode=mock Safe Mode)',
  rotateOutKey: 'Desktop-only (outgoing SSH keys / other servers)',
  HOSTS: 'demo-only (mock terminal remote hosts)',
  termExec: 'demo-only (mock terminal shell)',
};

const pagesDir = join(root, 'web/js/pages');
const pageFiles = (await readdir(pagesDir)).filter((f) => f.endsWith('.js'));
const used = new Map(); // name -> [files]

for (const file of pageFiles) {
  const src = await readFile(join(pagesDir, file), 'utf8');
  for (const m of src.matchAll(/\bapi\.auth\.(\w+)/g)) {
    const name = `auth.${m[1]}`;
    if (!used.has(name)) used.set(name, []);
    used.get(name).push(file);
  }
  for (const m of src.matchAll(/\bapi\.([A-Za-z_]\w*)/g)) {
    const name = m[1];
    if (name === 'auth' || name === 'js') continue;
    if (!used.has(name)) used.set(name, []);
    used.get(name).push(file);
  }
}

const failures = [];
for (const [name, files] of [...used.entries()].sort()) {
  if (HELPERS.has(name)) continue;
  if (ALLOWLIST[name]) continue;
  const via = WRAPPERS[name];
  if (via && REAL.has(via)) continue;
  if (REAL.has(name)) continue;
  failures.push({ name, files: [...new Set(files)] });
}

if (failures.length) {
  console.error('Mock-only API used by pages without an allowlist entry:\n');
  for (const f of failures) {
    console.error(`  ${f.name}  (${f.files.join(', ')})`);
  }
  console.error('\nAdd the name to REAL, stop calling it from pages, or allowlist it in tools/check-mock-api.mjs with a reason.');
  process.exit(1);
}

const allowListed = Object.entries(ALLOWLIST)
  .filter(([name]) => used.has(name))
  .map(([name, reason]) => `  ${name}: ${reason}`);
console.log(`Mock API check passed (${used.size} page references, ${REAL.size} REAL capabilities).`);
if (allowListed.length) {
  console.log('Allowlisted (intentional mock-only):\n' + allowListed.join('\n'));
}
