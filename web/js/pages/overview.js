import { html, raw, chk, bytes, gb, steps, legend, section, header, table, st, bindActions } from '../ui.js';
import * as api from '../api.js';
import { listeners } from '../store.js';

const kpi = (l, v, u, p, extra = '') => html`<div class="kpi"><span class="l">${l}</span><span class="v">${v}<small>${u}</small></span><div class="bar" role="img" aria-label="${l} ${Math.round(p)} percent"><i style="width:${Math.min(100, p)}%"></i></div>${extra}</div>`;

export async function view() {
  const [h, m, att] = await Promise.all([api.getHost(), api.getMetrics(), api.getAttention()]);
  const body = (m) => html`
    <div class="kpis" id="kpis">
      ${kpi('CPU', Math.round(m.cpuNow), '%', m.cpuNow, html`<span class="sm muted mono">load ${m.load.map((x) => x.toFixed(2)).join(' ')}</span>`)}
      ${kpi('Memory', m.memUsed.toFixed(1), ` / ${m.memTotal} GB`, m.memUsed / m.memTotal * 100, html`<span class="sm muted mono">cache ${m.cache.toFixed(1)} GB</span>`)}
      ${kpi('Disk /', Math.round(m.disk), ` / ${m.diskTotal} GB`, m.disk / m.diskTotal * 100, html`<span class="sm muted mono">${gb(m.diskTotal - m.disk, 0)} free</span>`)}
      ${kpi('Network', m.netIn.toFixed(1), ' MB/s in', Math.min(100, m.netIn * 8), html`<span class="sm muted mono">out ${m.netOut.toFixed(1)} MB/s · ${m.conns} conns</span>`)}
    </div>
    <div class="cols">
      <div class="w2">${section('CPU and memory, last 48 s', html`<div class="chart"><svg viewBox="0 0 640 180" preserveAspectRatio="none" role="img" aria-label="CPU and memory over time" style="width:100%;height:180px"><path d="${steps(m.mem)}" fill="none" stroke="var(--muted)" stroke-width="1.5" vector-effect="non-scaling-stroke"/><path d="${steps(m.cpu)}" fill="none" stroke="var(--accent)" stroke-width="2" vector-effect="non-scaling-stroke"/></svg></div>${legend([['var(--accent)', 'CPU %'], ['var(--muted)', 'Memory %']])}`)}</div>
      <div>${section('Top processes', html`
        ${table('Top processes', [['Process'], ['PID', 'r'], ['CPU', 'r'], ['Mem', 'r']], m.procs.map((p) => [html`${p.name}${p.self ? raw(' <span class="muted">· this panel</span>') : ''}`, p.pid, `${p.cpu.toFixed(1)}%`, `${p.mem} MB`]))}
        ${m.containers && m.containers.length ? html`<div style="margin-top:16px">${table('Running containers', [['Container'], ['CPU', 'r'], ['Mem', 'r']], m.containers.map((c) => [html`<span class="mono">${c.name}</span>`, `${(c.cpu ?? 0).toFixed(1)}%`, `${Math.round(c.mem ?? 0)} MB`]))}</div>` : ''}
      `, m.containers && m.containers.length ? html`<a class="btn link" href="#/containers">Containers →</a>` : '')}</div>
    </div>`;
  return {
    nav: 'overview', title: 'Overview',
    html: html`<main>${header('Overview', `${h.name} · ${h.os} · kernel ${h.kernel} · up ${h.upDays} days · ${h.cores} cores`, html`<a class="btn primary" href="#/apps/new">New service</a>`)}
      ${section('Needs attention', att.length ? html`<div>${att.map((a) => html`<div class="list-row">${chk('warn', a.text)}<a class="btn link" href="${a.alertId ? `#/alerts?id=${encodeURIComponent(a.alertId)}` : a.link[1]}">${a.link[0]}</a></div>`)}</div>` : chk('ok', 'Nothing needs attention.'))}
      <div id="live">${body(m)}</div></main>`,
    mount(root) {
      const f = async () => { const live = root.querySelector('#live'); if (!live) return; live.innerHTML = body(await api.getMetrics()).toString(); };
      listeners.add(f); this._f = f;
    },
    unmount() { listeners.delete(this._f); },
  };
}
