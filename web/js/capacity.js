import { html, section, confirmDialog, formDialog, toast, numberField, segBar, st, table, btn, bar } from './ui.js';
import * as api from './api.js';
import { server } from './store.js';

export const CAPACITY_CONFIRMATION = 'I understand this server may run out of memory';

/** Human labels for every known capacity feature key. Unknown keys title-case. */
export const FEATURE_LABELS = {
  install: { label: 'opsbox itself', blurb: 'Panel, files, terminal and the privileged helper' },
  docker: { label: 'Docker engine', blurb: 'Container runtime on this server' },
  app: { label: 'New app (default 128 MB)', blurb: 'A new app container with the default memory limit' },
  build: { label: 'Build on this server', blurb: 'Dockerfile or Nixpacks builds run here' },
  database: { label: 'New database', blurb: 'Postgres and other databases (swap does not count)' },
  template: { label: 'Templates', blurb: 'One-click templates from the catalogue' },
  backup: { label: 'Backup runs', blurb: 'Scheduled and manual restic backups' },
  image_pull: { label: 'Image pulls', blurb: 'Pulling images from a registry' },
};

const WORKLOAD_COLORS = ['var(--c1)', 'var(--c2)', 'var(--c4)', 'var(--c5)', 'var(--c6)', 'var(--accent)'];
const FIXED_COLORS = { OS: 'var(--muted)', opsbox: 'var(--ctl)', Docker: 'var(--text)', Free: 'var(--line)' };

export function featureLabel(key) {
  const k = String(key || '');
  if (FEATURE_LABELS[k]) return FEATURE_LABELS[k].label;
  return k.replace(/_/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()) || 'Unknown';
}

export function featureBlurb(key) {
  return FEATURE_LABELS[String(key || '')]?.blurb || '';
}

/** Map a fit object (or status string) to badge kind + label. Covers ok / tight / blocked. */
export function fitStatus(fit) {
  const s = typeof fit === 'string' ? fit : (fit?.status || 'ok');
  if (s === 'ok') return { kind: 'ok', label: 'Fits', status: 'ok' };
  if (s === 'tight') return { kind: 'warn', label: 'Tight', status: 'tight' };
  return { kind: 'fail', label: 'Blocked', status: 'blocked' };
}

export function fitBadge(fit) {
  const { kind, label } = fitStatus(fit);
  return st(kind, label);
}

export function fitReasons(fit) {
  if (!fit || fit.status === 'ok') return [];
  return [...(fit.reasons || []), ...(fit.suggestions || [])];
}

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
    const status = last ? fitStatus(last.fit).label : '';
    const reason = last ? [status, ...fitReasons(last.fit)].join(' · ') : 'Checking server capacity…';
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

export function memorySegments(c) {
  if (!c) return [];
  const b = c.allocatable || {};
  const segs = [
    { id: 'os', name: 'OS', mb: b.os_mb || 0, color: FIXED_COLORS.OS },
    { id: 'opsbox', name: 'opsbox', mb: b.opsbox_mb || 0, color: FIXED_COLORS.opsbox },
    { id: 'docker', name: 'Docker', mb: b.docker_mb || 0, color: FIXED_COLORS.Docker },
  ];
  (c.reservations || []).forEach((r, i) => {
    segs.push({
      id: `w-${r.name}`,
      name: r.name,
      mb: r.memory_mb || 0,
      color: WORKLOAD_COLORS[i % WORKLOAD_COLORS.length],
      href: r.database ? `#/databases/${encodeURIComponent(r.name)}` : `#/apps?s=${encodeURIComponent(r.name)}`,
      workload: true,
    });
  });
  segs.push({ id: 'free', name: 'Free', mb: b.allocatable_mb || 0, color: FIXED_COLORS.Free });
  return segs.filter((s) => s.mb > 0 || s.id === 'free');
}

export function memoryMap(c) {
  const segs = memorySegments(c);
  if (!segs.length) return html`<p class="muted">No memory map yet.</p>`;
  const parts = segs.map((s) => [s.mb, s.color]);
  return html`<div class="cap-map" data-key="cap-map">
    <div class="cap-seg">${segBar(parts, 'Memory reservations')}</div>
    <div class="cap-chips" role="list">${segs.map((s, i) => {
      const inner = html`<i style="background:${s.color}"></i><b>${s.name}</b> ${s.mb} MB`;
      return s.href
        ? html`<a class="cap-chip" role="listitem" href="${s.href}" data-seg="${i}">${inner}</a>`
        : html`<button type="button" class="cap-chip" role="listitem" data-seg="${i}">${inner}</button>`;
    })}</div>
  </div>`;
}

function fmtMb(n) {
  const v = Number(n) || 0;
  if (v >= 1024 && v % 1024 === 0) return `${v / 1024} GB`;
  if (v >= 1024) return `${(v / 1024).toFixed(1)} GB`;
  return `${v} MB`;
}

function pressureLabel(c) {
  if (!c) return { label: 'Unknown', kind: 'off' };
  if (c.pressure || c.allocatable?.pressure) return { label: 'High', kind: 'fail' };
  const free = c.allocatable?.allocatable_mb ?? 0;
  if (free < 128) return { label: 'Elevated', kind: 'warn' };
  return { label: 'Normal', kind: 'ok' };
}

function reservedTotal(c) {
  const b = c?.allocatable || {};
  return (b.os_mb || 0) + (b.opsbox_mb || 0) + (b.docker_mb || 0) + (b.reserved_mb || 0);
}

export function capacitySummary(c) {
  if (!c) return html`<p class="muted">Capacity is unavailable. Check the privileged helper.</p>`;
  const b = c.allocatable || {};
  const p = c.profile || {};
  const press = pressureLabel(c);
  const swapMb = p.swap_mb || 0;
  const swapCredit = b.swap_credit_mb || 0;
  return html`<div class="kpis cap-summary" data-key="cap-sum">
    <div class="kpi" data-key="alloc"><span class="l">Allocatable memory</span><span class="v" data-tween>${b.allocatable_mb ?? 0}<small> MB</small></span><span class="sm muted mono">of ${p.memory_mb || 0} MB RAM${b.allocatable_cpu_millis != null ? ` · ${((b.allocatable_cpu_millis || 0) / 1000).toFixed(2)} CPU` : ''}</span></div>
    <div class="kpi" data-key="res"><span class="l">Reserved</span><span class="v" data-tween>${reservedTotal(c)}<small> MB</small></span><span class="sm muted mono">OS ${b.os_mb || 0} · opsbox ${b.opsbox_mb || 0} · Docker ${b.docker_mb || 0} · workloads ${b.reserved_mb || 0}</span></div>
    <div class="kpi" data-key="press"><span class="l">Memory pressure</span><span class="v cap-press ${press.kind}">${press.label}</span><span class="sm muted mono">${press.kind === 'fail' ? 'Wait before adding workloads' : 'From memory pressure (PSI)'}</span></div>
  </div>
  <div class="cap-swap row spread" data-key="cap-swap">
    <p class="sm muted">${swapMb ? `${fmtMb(swapMb)} swap, counts as ${fmtMb(swapCredit)} for apps, never for databases` : 'No swap file. Swap helps apps and builds; it never increases database capacity.'}</p>
    ${c.swap_offer_mb && server.role === 'admin' ? btn(`Add a ${fmtMb(c.swap_offer_mb)} swap file`, { act: 'capacity-swap', data: { mb: c.swap_offer_mb }, kind: 'primary' }) : ''}
  </div>`;
}

function featureAction(f) {
  const s = f.fit?.status;
  if (f.feature === 'build' && s === 'blocked') {
    return html`<a class="btn link" href="#/apps/new">Build in GitHub Actions</a>`;
  }
  if (s === 'blocked' && f.feature !== 'install') {
    return html`<span class="row end">${btn('Add swap', { act: 'capacity-swap-offer', kind: 'link' })}${btn('See what uses memory', { act: 'cap-scroll-res', kind: 'link' })}</span>`;
  }
  if (s === 'tight') return btn('See what uses memory', { act: 'cap-scroll-res', kind: 'link' });
  return '';
}

export function featureTable(c) {
  const features = c?.features || [];
  if (!features.length) return emptyFeatures();
  const keyed = (cells, key, sig = '') => Object.assign(cells, { meta: { key, sig } });
  return table('What this server can run', [['Feature'], ['Status'], ['Needs'], ['Why'], ['', 'r']], features.map((f) => {
    const reasons = fitReasons(f.fit);
    const why = reasons[0] || (f.fit?.status === 'ok' ? 'Enough headroom right now.' : '—');
    const needs = [
      f.minimum_mb ? `min ${fmtMb(f.minimum_mb)}` : null,
      f.recommended_mb ? `rec ${fmtMb(f.recommended_mb)}` : null,
      f.disk_mb ? `${fmtMb(f.disk_mb)} disk` : null,
    ].filter(Boolean).join(' · ') || '—';
    const blurb = featureBlurb(f.feature);
    return keyed([
      html`<b>${featureLabel(f.feature)}</b>${blurb ? html`<div class="sm muted">${blurb}</div>` : ''}`,
      fitBadge(f.fit),
      html`<span class="mono sm">${needs}</span>`,
      html`<details class="cap-why"><summary title="${reasons.join(' · ') || why}">${why}</summary>${reasons.length > 1 ? html`<ul class="sm muted">${reasons.slice(1).map((r) => html`<li>${r}</li>`)}</ul>` : ''}</details>`,
      featureAction(f),
    ], `f-${f.feature}`, `${f.fit?.status}|${why}`);
  }));
}

function emptyFeatures() {
  return html`<p class="muted">No feature fit data yet.</p>`;
}

export function reservationsTable(c, usageByName = {}, { sortBy = 'name', sortDir = 1 } = {}) {
  const rows = [...(c?.reservations || [])];
  const cmp = (a, b) => {
    let av, bv;
    if (sortBy === 'limit') { av = a.memory_mb; bv = b.memory_mb; }
    else if (sortBy === 'usage') { av = usageByName[a.name] ?? -1; bv = usageByName[b.name] ?? -1; }
    else { av = a.name; bv = b.name; }
    if (av < bv) return -1 * sortDir;
    if (av > bv) return 1 * sortDir;
    return 0;
  };
  rows.sort(cmp);
  if (!rows.length) return html`<p class="muted">No workload reservations yet.</p>`;
  const keyed = (cells, key, sig = '') => Object.assign(cells, { meta: { key, sig } });
  const head = (id, label, align) => html`<button type="button" class="cap-sort" data-act="cap-sort" data-by="${id}">${label}</button>`;
  return table('Reservations', [
    [head('name', 'Workload')],
    [head('limit', 'Limit'), 'r'],
    [head('usage', 'Usage now'), 'r'],
    ['Within limit'],
    ['', 'r'],
  ], rows.map((r) => {
    const used = usageByName[r.name];
    const lim = Math.max(1, r.memory_mb || 1);
    const ratio = used == null ? null : Math.min(1, used / lim);
    const over = c?.over_capacity || (used != null && used > lim);
    return keyed([
      html`<span class="mono">${r.name}</span>${r.database ? html`<span class="sm muted"> · database</span>` : ''}${over ? st('warn', 'over capacity') : ''}`,
      html`<span class="mono">${r.memory_mb} MB</span>`,
      html`<span class="mono">${used == null ? '—' : `${Math.round(used)} MB`}</span>`,
      ratio == null ? html`<span class="muted sm">—</span>` : bar(ratio, `${r.name} usage`),
      btn('Edit limit', { act: 'cap-edit', data: { name: r.name, db: r.database ? '1' : '0', mb: r.memory_mb }, kind: 'link' }),
    ], `r-${r.name}`, `${r.memory_mb}|${used ?? ''}`);
  }), { flip: true });
}

/** Full Capacity tab body (summary → map → features → reservations → explainer). */
export function capacityPage(c, { usageByName = {}, sortBy = 'name', sortDir = 1 } = {}) {
  if (!c) {
    return section('Capacity', html`<p class="muted">Capacity is unavailable. Check the privileged helper.</p>`);
  }
  return html`<div class="cap-page col gap16" data-key="cap-page">
    ${capacitySummary(c)}
    ${section('Memory map', memoryMap(c))}
    ${section('What this server can run', featureTable(c))}
    ${section('Reservations', html`<div id="cap-reservations">${reservationsTable(c, usageByName, { sortBy, sortDir })}</div>`)}
    <p class="sm muted">Reserves keep room for the OS (${c.allocatable?.os_mb || 0} MB), opsbox (${c.allocatable?.opsbox_mb || 0} MB) and Docker (${c.allocatable?.docker_mb || 0} MB) before any workload. Swap credit counts for apps and builds only — never for databases. <a href="#/settings">Settings</a></p>
  </div>`;
}

/**
 * One-line footer for the Overview Memory KPI.
 * Returns { html, warn } so the tile can tint when builds are blocked or pressure is high.
 */
export function capacityFooter(c) {
  if (!c) {
    return { warn: false, html: html`<a class="kpi-foot muted" href="#/system/capacity">Capacity unavailable · Capacity →</a>` };
  }
  const free = c.allocatable?.allocatable_mb ?? 0;
  const blocked = (c.features || []).filter((f) => f.fit?.status === 'blocked');
  const likely = blocked.find((f) => ['build', 'app', 'database', 'docker'].includes(f.feature));
  const press = c.pressure || c.allocatable?.pressure;
  let warn = false;
  let text = `${free} MB free to allocate`;
  if (press) { warn = true; text = 'memory pressure high'; }
  else if (likely) {
    warn = true;
    text = likely.feature === 'build' ? 'builds blocked'
      : likely.feature === 'database' ? 'databases blocked'
        : likely.feature === 'app' ? 'new apps blocked'
          : `${featureLabel(likely.feature)} blocked`;
  } else if (free < 128) {
    warn = true;
    text = `${free} MB free to allocate`;
  }
  return {
    warn,
    html: html`<a class="kpi-foot${warn ? ' warn' : ''}" href="#/system/capacity">${text} · Capacity →</a>`,
  };
}

export async function addSwap(mb) {
  if (await confirmDialog({ title: 'Add a swap file?', body: `Use ${mb} MB of disk for swap. Swap helps apps and builds; it never increases database capacity.`, confirm: 'Add swap' })) {
    await api.addCapacitySwap(mb); toast('Swap file added.');
  }
}

/** Bind hover/focus highlight between memory-map chips and bar segments. */
export function bindMemoryMap(root) {
  const map = root.querySelector('.cap-map');
  if (!map) return;
  const set = (i) => {
    map.querySelectorAll('.cap-chip,.cap-seg .seg i').forEach((el) => {
      const on = el.dataset.seg === String(i) || el.dataset.key === `s${i}`;
      el.classList.toggle('on', i != null && on);
      el.classList.toggle('dim', i != null && !on);
    });
  };
  map.addEventListener('pointerover', (e) => {
    const chip = e.target.closest('[data-seg]');
    if (chip && map.contains(chip)) set(chip.dataset.seg);
  });
  map.addEventListener('pointerleave', () => set(null));
  map.addEventListener('focusin', (e) => {
    const chip = e.target.closest('[data-seg]');
    if (chip) set(chip.dataset.seg);
  });
  map.addEventListener('focusout', () => set(null));
}
