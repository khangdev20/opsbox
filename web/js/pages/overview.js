import { html, raw, chk, gb, legend, section, header, table, bindActions, num, bar, patch, createChart, poll, refreshing } from '../ui.js';
import * as api from '../api.js';
import { capacityCard, addSwap } from '../capacity.js';
import { networkBanner, networkActions } from '../network.js';
import { listeners } from '../store.js';

const kpi = (l, v, u, p, extra = '', thresholds = true) => html`<div class="kpi"><span class="l">${l}</span><span class="v">${v}<small>${u}</small></span>${bar(p / 100, l, { thresholds })}${extra}</div>`;
// Rows of live tables are keyed (updated in place) and never flash: only their numbers move.
const row = (cells, key) => Object.assign(cells, { meta: { key, sig: '-' } });

export async function view() {
  const [h, m, att] = await Promise.all([api.getHost(), api.getMetrics(), api.getAttention()]);
  const capacity = await api.getCapacity().catch(() => null);
  const network = await api.getProxyNetwork().catch(() => null);
  const m0 = m;
  const body = (m) => html`
    <div class="kpis" id="kpis">
      ${kpi('CPU', num(m.cpuNow, 'int'), '%', m.cpuNow, html`<span class="sm muted mono">load ${m.load.map((x, i) => html`${i ? ' ' : ''}${num(x, 'dec2')}`)}</span>`)}
      ${kpi('Memory', num(m.memUsed, 'dec1'), ` / ${m.memTotal} GB`, m.memUsed / m.memTotal * 100, html`<span class="sm muted mono">cache ${num(m.cache, 'dec1')} GB</span>`)}
      ${kpi('Disk /', num(m.disk, 'int'), ` / ${m.diskTotal} GB`, m.disk / m.diskTotal * 100, html`<span class="sm muted mono">${num(m.diskTotal - m.disk, 'gb0')} free</span>`)}
      ${kpi('Network', num(m.netIn, 'dec1'), ' MB/s in', Math.min(100, m.netIn * 8), html`<span class="sm muted mono">out ${num(m.netOut, 'dec1')} MB/s · ${num(m.conns, 'int')} conns</span>`, false)}
    </div>
    <div class="cols">
      <div class="w2">${section('CPU and memory, last 48 s', html`<div class="chart" id="cpu-chart" data-keep></div>${legend([['var(--accent)', 'CPU %'], ['var(--muted)', 'Memory %']])}`)}</div>
      <div>${section('Top processes', html`
        ${table('Top processes', [['Process'], ['PID', 'r'], ['CPU', 'r'], ['Mem', 'r']], m.procs.map((p) => row([html`${p.name}${p.self ? raw(' <span class="muted">· this panel</span>') : ''}`, p.pid, num(p.cpu, 'pct1'), num(p.mem, 'mb')], `p${p.pid}`)))}
        ${m.containers && m.containers.length ? html`<div style="margin-top:16px">${table('Running containers', [['Container'], ['CPU', 'r'], ['Mem', 'r']], m.containers.map((c) => row([html`<span class="mono">${c.name}</span>`, num(c.cpu ?? 0, 'pct1'), num(c.mem ?? 0, 'mb')], `c${c.name}`)))}</div>` : ''}
      `, m.containers && m.containers.length ? html`<a class="btn link" href="#/containers">Containers →</a>` : '')}</div>
    </div>`;
  return {
    nav: 'overview', title: 'Overview',
    html: html`<main>${header('Overview', `${h.name} · ${h.os} · kernel ${h.kernel} · up ${h.upDays} days · ${h.cores} cores`, html`<a class="btn primary" href="#/apps/new">New service</a>`)}
      <div id="proxy-network-banner">${networkBanner(network)}</div>
      ${section('Needs attention', att.length ? html`<div>${att.map((a) => html`<div class="list-row">${chk('warn', a.text)}<a class="btn link" href="${a.alertId ? `#/alerts?id=${encodeURIComponent(a.alertId)}` : a.link[1]}">${a.link[0]}</a></div>`)}</div>` : chk('ok', 'Nothing needs attention.'))}
      <div id="capacity-card">${capacityCard(capacity)}</div>
      <div id="live">${body(m)}</div></main>`,
    mount(root) {
      const card = () => root.querySelector('#capacity-card');
      const refreshNetwork = async () => { const b = root.querySelector('#proxy-network-banner'); if (b) patch(b, networkBanner(await api.getProxyNetwork()).toString()); };
      bindActions(root, { ...networkActions(refreshNetwork, () => {}), 'capacity-swap': async (el) => { await addSwap(Number(el.dataset.mb)); const c = card(); if (c) patch(c, capacityCard(await api.getCapacity()).toString()); } });
      const live = root.querySelector('#live');
      const chart = createChart(root.querySelector('#cpu-chart'), { label: 'CPU and memory over time', series: [{ color: 'var(--muted)', width: 1.5 }, { color: 'var(--accent)', width: 2 }], cap: 100, floor: 20 });
      const draw = (m) => chart.update([m.mem, m.cpu]);
      draw(m0);
      const f = async () => { if (!live.isConnected) return; const m = await api.getMetrics(); patch(live, body(m).toString()); draw(m); };
      listeners.add(f); this._f = f;
      this._stopPoll = poll(async () => {
        const [data, cap] = await Promise.all([api.getProxyNetwork().catch(() => null), api.getCapacity().catch(() => null)]);
        const c = card(), b = root.querySelector('#proxy-network-banner');
        await refreshing(c, async () => { if (c) patch(c, capacityCard(cap).toString()); if (b) patch(b, networkBanner(data).toString()); });
      }, 5000);
    },
    unmount() { listeners.delete(this._f); if (this._stopPoll) this._stopPoll(); },
  };
}
