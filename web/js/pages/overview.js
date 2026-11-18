import { html, raw, legend, section, header, bindActions, num, patch, createChart, poll, refreshing, segBar, st, empty, ago, clock, bar, liveState } from '../ui.js';
import * as api from '../api.js';
import { capacityFooter } from '../capacity.js';
import { networkBanner, networkActions } from '../network.js';
import { sharedAttention, onAttention, openNotifications } from '../shell.js';
import { listeners } from '../store.js';

const CRIT_ICO = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 8v5M12 16h.01"/></svg>';
function criticalBanner(att) {
  const crits = (att?.items || []).filter((a) => a.severity === 'critical' && !a.read);
  if (!crits.length) return '';
  const top = crits[0];
  const rest = crits.length - 1;
  return html`<div class="crit-banner" role="region" aria-label="Critical alert">
    <span class="crit-ico" aria-hidden="true">${raw(CRIT_ICO)}</span>
    <span class="crit-txt">${top.text}</span>
    <a class="btn link" href="${top.link?.[1] || '#/alerts'}">${top.link?.[0] || 'View →'}</a>
    ${rest > 0 ? html`<button type="button" class="btn link crit-more" data-act="open-bell">and ${rest} more</button>` : ''}
  </div>`;
}

const kpi = (l, v, u, p, extra = '', thresholds = true, foot = '', id = '') => html`<div class="kpi${foot && foot.warn ? ' warn-foot' : ''}"${raw(id ? ` id="${id}" data-key="${id}"` : '')}><span class="l">${l}</span><span class="v">${v}<small>${u}</small></span>${bar(p / 100, l, { thresholds })}${extra}${foot ? html`<span id="cap-foot" data-key="cap-foot">${foot.html}</span>` : ''}</div>`;

const avg = (a) => (a.length ? a.reduce((x, y) => x + y, 0) / a.length : 0);
const hue = (name) => { let h = 0; for (const c of String(name)) h = (h * 31 + c.charCodeAt(0)) >>> 0; return (h % 6) + 1; };
const TOP = 5;

// A bar that animates with a transform (data-bar), like every other meter; never changes colour with its level.
const pbar = (ratio, cls) => { const r = Math.min(1, Math.max(0, Number.isFinite(ratio) ? ratio : 0)); return html`<span class="bar sm ${cls}" aria-hidden="true"><i data-bar="${r.toFixed(4)}" data-th="off" style="transform:scaleX(${r.toFixed(4)})"></i></span>`; };

function procSubline(p) {
  if (p.kind === 'container') {
    const label = p.container?.name || p.container?.id;
    return label ? `container ${label} · pid ${p.pid}` : `container · pid ${p.pid}`;
  }
  if (p.kind === 'system' && p.unit) return `system ${p.unit} · pid ${p.pid}`;
  if (p.kind === 'system') return `system · pid ${p.pid}`;
  if (p.kind === 'user') return `user · pid ${p.pid}`;
  if (p.kind === 'unknown') return `pid ${p.pid}`;
  return `pid ${p.pid}`;
}

function procHref(p) {
  if (p.kind === 'container' && p.container?.service) return `#/apps?s=${encodeURIComponent(p.container.service)}`;
  if (p.kind === 'container' && (p.container?.id || p.container?.name)) return '#/containers';
  return '';
}

// One row of "Top processes": rank, coloured initial tile, name with kind + PID, CPU and memory with bars.
// Keyed by PID so rows move and update in place when the sort or the numbers change.
export function procRow(p, i, maxCpu, memTotalMb) {
  const tile = p.self ? 'self' : hue(p.name);
  const href = procHref(p);
  const body = html`<span class="pr-rank">${String(i + 1).padStart(2, '0')}</span>
    <span class="pr-id"><span class="pr-tile t${tile}" aria-hidden="true">${String(p.name || '?')[0].toUpperCase()}</span>
      <span class="pr-txt"><span class="pr-name">${p.name}${p.self ? html`<span class="pr-this">this</span>` : ''}</span><span class="pr-sub">${procSubline(p)}</span></span></span>
    <span class="pr-v"><span class="mono">${num(p.cpu, 'pct1')}</span>${pbar(p.cpu / maxCpu, 'cpu')}</span>
    <span class="pr-v"><span class="mono">${num(p.mem, 'mb')}</span>${pbar(p.mem / memTotalMb, 'mem')}</span>`;
  if (href) return html`<a class="pr pr-link" role="row" href="${href}" data-key="p${p.pid}" data-sig="-">${body}</a>`;
  return html`<div class="pr" role="row" data-key="p${p.pid}" data-sig="-">${body}</div>`;
}

function topProcesses(m, sortBy) {
  const memTotalMb = (m.memTotal || 1) * 1024;
  const list = [...(m.procs || [])].sort((a, b) => b[sortBy] - a[sortBy]).slice(0, TOP);
  if (!list.length) return empty('No process data yet.');
  const maxCpu = Math.max(1, ...list.map((p) => p.cpu));
  const top = list[0];
  return html`<div class="procs">
    <div class="heavy"><span class="col"><span class="sm muted">Heaviest right now</span><b>${top.name}</b></span>
      <span class="heavy-v mono"><span><span class="muted">CPU </span>${num(top.cpu, 'pct1')}</span><span><span class="muted">mem </span>${num(top.mem, 'mb')}</span></span></div>
    <div class="pr pr-h" role="row"><span class="pr-rank">#</span><span>Process</span><span class="r">CPU</span><span class="r">Memory</span></div>
    <div class="pr-list" data-flip>${list.map((p, i) => procRow(p, i, maxCpu, memTotalMb))}</div>
  </div>`;
}
const sortSwitch = (sortBy) => html`<div class="seg-radio" role="radiogroup" aria-label="Sort by">${[['cpu', 'CPU'], ['mem', 'Memory']].map(([k, l]) => html`<button type="button" role="radio" aria-checked="${String(sortBy === k)}" data-act="ps-sort" data-by="${k}">${l}</button>`)}</div>`;

// Row 2 (Containers, Reclaimable space, Recent activity). Each source fails on its own: a missing one shows an empty state.
function containersSection(list) {
  const rows = (list || []).slice(0, 5);
  const chip = (c) => c.state === 'running' ? st('ok', c.since > 0 ? `up ${ago(c.since).replace(' ago', '')}` : 'running') : c.state === 'restarting' ? st('warn', 'restarting') : c.state === 'paused' ? st('warn', 'paused') : st('off', c.state || 'stopped');
  return section('Containers', rows.length ? html`<div><div class="flip-list" data-flip>${rows.map((c) => html`<div class="list-row mrow" data-key="c${c.id || c.name}" data-sig="-"><span class="mono">${c.name}</span>${chip(c)}</div>`)}</div>${list.length > rows.length ? html`<p class="sm muted" style="padding-top:10px">+${list.length - rows.length} more</p>` : ''}</div>` : empty('No containers yet.'), html`<a class="btn link" href="#/containers">View all</a>`, 'span2');
}
const SEG = ['var(--accent)', 'var(--text)', 'var(--warn)', 'var(--muted)', 'var(--ctl)'];
function reclaimSection(d) {
  const items = d ? d.items.filter((i) => !i.pending && (i.bytes ?? i.gb) > 0) : [];
  const by = (i) => i.bytes ?? (i.gb || 0) * 2 ** 30;
  const total = items.reduce((a, i) => a + by(i), 0);
  const body = !d ? empty('Could not scan this server.') : d.pending && !items.length ? html`<p class="muted">Scanning…</p>` : html`<div class="col gap12">
    <span class="big mono">${num(total / 2 ** 30, 'dec1')}<small> GB</small></span>
    ${segBar(items.map((i, k) => [by(i), SEG[k % SEG.length]]), 'Reclaimable space by kind')}
    ${legend(items.map((i, k) => [SEG[k % SEG.length], `${i.id} ${(by(i) / 2 ** 30).toFixed(1)}`]))}
    <div><a class="btn link" href="#/cleanup">Review and clean up →</a></div></div>`;
  return section('Reclaimable space', body, '', 'span2');
}
function activitySection(rows) {
  const body = rows && rows.length ? html`<div class="acts-log">${rows.slice(0, 5).map((a) => html`<div class="alog${a.warn ? ' warn' : ''}"><span class="muted">${clock(a.t).slice(0, 5)}</span><span>${[a.user, a.action].filter(Boolean).join(' ')}</span></div>`)}</div>` : empty('Nothing recorded yet.');
  return section('Recent activity', body, html`<a class="btn link" href="#/security">Audit log</a>`, 'span2');
}

export async function view() {
  const [h, m, att, capacity] = await Promise.all([api.getHost(), api.getMetrics(), sharedAttention(), api.getCapacity().catch(() => null)]);
  const network = await api.getProxyNetwork().catch(() => null);
  const m0 = m;
  let sortBy = 'cpu', lastM = m, lastCap = capacity;
  const extras = async () => {
    const [cs, cl, sec] = await Promise.all([api.listContainers().catch(() => null), api.scanCleanup().catch(() => null), api.getSecurity().catch(() => null)]);
    return { cs, cl, audit: sec && sec.audit };
  };
  const row2 = (x) => html`${containersSection(x.cs || (lastM.containers || []).map((c) => ({ ...c, state: 'running' })))}${reclaimSection(x.cl)}${activitySection(x.audit)}`;
  const x0 = await extras();
  const procBadge = () => liveState().ok ? html`<span class="live-badge">● live</span>` : html`<span class="live-badge stale">● paused</span>`;
  const memKpi = (m, cap = lastCap) => kpi('Memory', num(m.memUsed, 'dec1'), ` / ${m.memTotal} GB`, m.memUsed / m.memTotal * 100, html`<span class="sm muted mono">cache ${num(m.cache, 'dec1')} GB</span>`, true, capacityFooter(cap), 'mem-kpi');
  const body = (m, cap = lastCap) => html`
    <div class="kpis" id="kpis">
      ${kpi(`CPU · ${h.cores} cores`, num(m.cpuNow, 'int'), '%', m.cpuNow, html`<span class="sm muted mono">load ${m.load.map((x, i) => html`${i ? ' ' : ''}${num(x, 'dec2')}`)}</span>`)}
      ${memKpi(m, cap)}
      ${kpi('Disk /', num(m.disk, 'int'), ` / ${m.diskTotal} GB`, m.disk / m.diskTotal * 100, html`<span class="sm muted mono">${num(m.diskTotal - m.disk, 'gb0')} free</span>`)}
      ${kpi('Network', num(m.netIn, 'dec1'), ' MB/s in', Math.min(100, m.netIn * 8), html`<span class="sm muted mono">out ${num(m.netOut, 'dec1')} MB/s · ${num(m.conns, 'int')} conns</span>`, false)}
    </div>
    <div class="ov-grid">
      ${section('CPU and memory, last 48 s', html`<div class="chart tall" id="cpu-chart" data-keep></div><div class="chart-x mono muted" aria-hidden="true"><span>-48 s</span><span>-24 s</span><span>now</span></div>
        <div class="chart-legend mono"><span><i class="ln"></i> CPU <b>${num(m.cpuNow, 'pct')}</b></span><span><i class="ln dash"></i> Memory <b>${num(m.memUsed / m.memTotal * 100, 'pct')}</b></span><span class="grow"></span><span class="muted">avg CPU ${num(avg(m.cpu), 'pct')} · peak ${num(Math.max(0, ...m.cpu), 'pct')}</span></div>`, '', 'span3')}
      ${section(html`Top processes ${procBadge()}`, topProcesses(m, sortBy), html`<div class="row">${sortSwitch(sortBy)}<a class="more" href="#/system">All →</a></div>`, 'span3')}
    </div>`;
  return {
    nav: 'overview', title: 'Overview',
    html: html`<main>${header('Overview', `${h.name} · ${h.os} · kernel ${h.kernel} · up ${h.upDays} days · ${h.cores} cores`, html`<a class="btn primary" href="#/apps/new">New service</a>`)}
      <div id="crit-banner">${criticalBanner(att)}</div>
      <div id="proxy-network-banner">${networkBanner(network)}</div>
      <div class="ov"><div id="live">${body(m)}</div><div class="ov-grid" id="ov-row2">${row2(x0)}</div></div></main>`,
    mount(root) {
      const refreshNetwork = async () => { const b = root.querySelector('#proxy-network-banner'); if (b) patch(b, networkBanner(await api.getProxyNetwork()).toString()); };
      bindActions(root, {
        ...networkActions(refreshNetwork, () => {}),
        'open-bell': () => openNotifications(),
        'ps-sort': (el) => { sortBy = el.dataset.by; repaint(lastM); },
      });
      this._unsubAttn = onAttention((next) => { const b = root.querySelector('#crit-banner'); if (b) patch(b, criticalBanner(next).toString()); });
      const live = root.querySelector('#live');
      const chart = createChart(root.querySelector('#cpu-chart'), { label: 'CPU and memory over time', series: [{ color: 'var(--muted)', width: 1.5, dash: '5 4' }, { color: 'var(--text)', width: 2 }], cap: 100, floor: 20, h: 300 });
      const draw = (m) => chart.update([m.mem, m.cpu]);
      const repaint = (m, cap = lastCap) => { lastM = m; patch(live, body(m, cap).toString()); draw(m); };
      draw(m0);
      const f = async () => { if (!live.isConnected) return; repaint(await api.getMetrics()); };
      listeners.add(f); this._f = f;
      this._stopPoll = poll(async () => {
        // One capacity fetch per poll; Memory KPI footer shares it (no second request).
        const [data, cap, x] = await Promise.all([api.getProxyNetwork().catch(() => null), api.getCapacity().catch(() => null), extras()]);
        lastCap = cap;
        const b = root.querySelector('#proxy-network-banner'), r2 = root.querySelector('#ov-row2');
        const footHost = root.querySelector('#cap-foot'), memTile = root.querySelector('#mem-kpi');
        await refreshing(footHost || r2, async () => {
          if (footHost && memTile) {
            const foot = capacityFooter(cap);
            memTile.classList.toggle('warn-foot', !!foot.warn);
            patch(footHost, foot.html.toString());
          }
          if (b) patch(b, networkBanner(data).toString());
          if (r2) patch(r2, row2(x).toString());
        });
      }, 5000);
    },
    unmount() { listeners.delete(this._f); if (this._stopPoll) this._stopPoll(); if (this._unsubAttn) this._unsubAttn(); },
  };
}
