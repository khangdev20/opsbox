import { html, section, confirmDialog, formDialog, toast, numberField, segBar } from './ui.js';
import * as api from './api.js';
import { server } from './store.js';

export const CAPACITY_CONFIRMATION = 'I understand this server may run out of memory';
export function capacityFields(memory = 128) {
  return html`<div class="field"><label for="cap-mem">Memory limit (MB)</label>${numberField({ id: 'cap-mem', name: 'memoryLimitMb', value: memory, min: 64, max: 1048576, step: 64, label: 'memory limit', attrs: 'data-capacity-memory' })}</div>
    <p data-capacity-result role="status" aria-live="polite">Checking server capacity…</p>
    ${server.role === 'admin' ? html`<div class="field"><label class="opt"><input type="checkbox" data-capacity-override> Override capacity (Owner only)</label></div><div class="field"><label for="cap-confirm">To override, type “${CAPACITY_CONFIRMATION}”</label><input class="input" id="cap-confirm" data-capacity-confirmation autocomplete="off"></div>` : ''}`;
}
export function capacityPayload(root) {
  return {
    memoryLimitMb: Number(root.querySelector('[data-capacity-memory]')?.value || 128),
    override: !!root.querySelector('[data-capacity-override]')?.checked,
    capacityConfirmation: root.querySelector('[data-capacity-confirmation]')?.value || '',
  };
}
export function bindCapacity(root, request, submit = 'button[type="submit"]') {
  let timer, generation = 0, last = null;
  const paint = () => {
    const out = root.querySelector('[data-capacity-result]');
    const override = capacityPayload(root);
    const allowed = override.override && override.capacityConfirmation === CAPACITY_CONFIRMATION && server.role === 'admin';
    const blocked = !last || last.fit.status === 'blocked';
    const reason = last ? [last.fit.status === 'ok' ? 'Fits' : last.fit.status === 'tight' ? 'Tight' : "Doesn’t fit", ...(last.fit.reasons || []), ...(last.fit.suggestions || [])].join(' · ') : 'Checking server capacity…';
    if (out) out.textContent = reason;
    root.querySelectorAll(submit).forEach((b) => { b.disabled = root.hasAttribute('data-capacity-busy') || (blocked && !allowed); b.title = blocked ? reason : ''; });
  };
  const refresh = async () => {
    const current = ++generation; last = null; paint();
    try {
      const r = await request();
      const results = await Promise.all((Array.isArray(r) ? r : [r]).map((q) => api.checkCapacity(q)));
      if (current !== generation) return;
      last = results.find((r) => r.fit.status === 'blocked') || results.find((r) => r.fit.status === 'tight') || results[0]; paint();
    } catch (e) { if (current === generation) { const out = root.querySelector('[data-capacity-result]'); if (out) out.textContent = e.message; } }
  };
  root.addEventListener('input', (e) => {
    if (!e.target.matches('[data-capacity-memory],[data-capacity-override],[data-capacity-confirmation]')) return;
    if (!e.target.matches('[data-capacity-memory]')) { paint(); return; }
    last = null; paint(); clearTimeout(timer); timer = setTimeout(refresh, 200);
  });
  refresh();
  // `repaint` re-applies the button state (for example after `data-capacity-busy` changed)
  // without a new check, so the result line and the override fields stay as they are.
  return { refresh, repaint: paint, dispose() { clearTimeout(timer); generation++; } };
}
export async function withCapacityOverride(run, body) {
  try { return await run(body); } catch (e) {
    if (!e.capacity || server.role !== 'admin') throw e;
    const fit = e.capacity.fit;
    const ok = await confirmDialog({ title: 'This server is over capacity', body: [...(fit.reasons || []), ...(fit.suggestions || [])].join(' '), confirm: 'Override capacity', typed: CAPACITY_CONFIRMATION, danger: true });
    if (!ok) throw e;
    return run({ ...body, override: true, capacityConfirmation: CAPACITY_CONFIRMATION });
  }
}
export async function editMemory(service) {
  // `run` keeps the dialog open on a failure, so the typed limit, the override box and the
  // confirmation sentence are still there next to the error.
  const out = await formDialog({ title: `Memory for ${service.id}`, fields: [], submit: 'Save limit', run: (values) => api.updateService(service.id, values.capacity), capacity: { memory: service.memoryLimitMb || (service.kind === 'db' ? 256 : 128), request: (root) => ({ feature: service.kind === 'db' ? 'database' : 'app', memory_mb: capacityPayload(root).memoryLimitMb, replacing: service.name || service.id }) } });
  if (!out) return;
  toast('Memory limit saved.');
}
export function capacityCard(c) {
  if (!c) return section('Capacity', html`<p class="muted">Capacity is unavailable. Check the privileged helper.</p>`);
  const b = c.allocatable, p = c.profile;
  const segments = [['OS', b.os_mb, 'var(--muted)'], ['opsbox', b.opsbox_mb, 'var(--accent)'], ['Docker', b.docker_mb, 'var(--warn)'], ...c.reservations.map((r) => [r.name, r.memory_mb, 'var(--accent)']), ['Free', b.allocatable_mb, 'var(--ls)']];
  const total = Math.max(1, p.memory_mb + b.swap_credit_mb);
  return section('Capacity', html`<p>${p.memory_mb} MB RAM · ${p.swap_mb} MB swap · ${p.cpu_millis / 1000} CPU cores available${p.below_minimum ? ' · Installed below minimum' : ''}</p>
    ${segBar([...segments.filter(([, n]) => n > 0).map(([, n, color]) => [n, color]), [Math.max(0, total - segments.reduce((a, [, n]) => a + n, 0)), 'var(--line)']], 'Memory reservations')}
    <p class="sm muted">${segments.map(([label, n]) => `${label}: ${n} MB`).join(' · ')}</p><p>${b.allocatable_mb} MB allocatable · databases: ${b.database_allocatable_mb} MB · memory pressure ${c.pressure ? 'high — wait before adding workloads' : 'normal'}</p>
    ${c.features.map((f) => html`<span class="tag" title="${(f.fit.reasons || []).join(' ')}">${f.feature}: ${f.fit.status}</span>`)}
    ${c.swap_offer_mb && server.role === 'admin' ? html`<p><button class="btn" data-act="capacity-swap" data-mb="${c.swap_offer_mb}">Add a ${c.swap_offer_mb / 1024} GB swap file</button></p>` : ''}`);
}
export async function addSwap(mb) {
  if (await confirmDialog({ title: 'Add a swap file?', body: `Use ${mb} MB of disk for swap. Swap helps apps and builds; it never increases database capacity.`, confirm: 'Add swap' })) {
    await api.addCapacitySwap(mb); toast('Swap file added.');
  }
}
