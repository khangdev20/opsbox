import { html, raw, esc, ago, st, btn, tabs, table, section, header, bindActions, confirmDialog, askForm, toast } from '../ui.js';
import * as api from '../api.js';
import { listeners } from '../store.js';

const KIND = { active: ['ok', 'running'], building: ['build', 'building'], failed: ['fail', 'failed'] };
const DST = { live: ['ok', 'live'], replaced: ['off', 'replaced'], failed: ['fail', 'failed'], building: ['build', 'building'] };

export async function view({ query }) {
  let proj = await api.getProject(); let sel = query.s || 'web'; let tab = 0; let logDeploy = null; let root;
  const svc = () => proj.services.find((s) => s.id === sel) || proj.services[0];

  const graph = () => {
    const edges = [['web', 'api'], ['web', 'postgres'], ['api', 'postgres'], ['api', 'redis'], ['worker', 'redis'], ['worker', 'postgres']]
      .map(([a, b]) => [proj.services.find((s) => s.id === a), proj.services.find((s) => s.id === b)]).filter(([a, b]) => a && b);
    const ed = edges.map(([a, b]) => {
      const x1 = a.x + 220, y1 = a.y + 52, x2 = b.x, y2 = b.y + 52, mx = (x1 + x2) / 2;
      const seg = (l, t, w, h) => `<div class="edge" style="left:${l}px;top:${t}px;width:${Math.max(1, w)}px;height:${Math.max(1, h)}px"></div>`;
      return seg(x1, y1, mx - x1, 1) + seg(mx, Math.min(y1, y2), 1, Math.abs(y2 - y1)) + seg(mx, y2, x2 - mx, 1);
    }).join('');
    const nodes = proj.services.map((s) => { const [k, t] = KIND[s.state] || KIND.active;
      return html`<button type="button" class="node ${s.id === sel ? 'on' : ''}" style="left:${s.x}px;top:${s.y}px" data-act="select" data-id="${s.id}" aria-pressed="${s.id === sel}"><b>${s.id}</b><span class="sm muted">${s.repo}</span>${st(k, s.kind === 'db' ? 'running' : t)}</button>`; });
    return html`<div style="overflow-x:auto"><div class="graph">${raw(ed)}${nodes}</div></div>`;
  };

  const detail = () => {
    const s = svc();
    const items = s.kind === 'db' ? ['Overview'] : ['Deploys', 'Variables', 'Settings'];
    const t = Math.min(tab, items.length - 1);
    let body;
    if (s.kind === 'db') body = html`<p class="muted">${s.repo}</p><p class="sm muted">${s.note}. Connection string is available to services in this project as an environment variable.</p>`;
    else if (t === 0) body = html`${table('Deploys', [['Commit'], ['Message'], ['Status'], ['When'], ['', 'r']], s.deploys.map((d) => { const [k, l] = DST[d.status];
        return [html`<span class="mono">${d.id}</span>`, d.msg, st(k, l), ago(d.at), html`<span class="row" style="justify-content:flex-end">${btn('Log', { act: 'log', data: { id: d.id }, kind: 'link' })}${d.status === 'replaced' ? btn('Roll back', { act: 'rollback', data: { id: d.id }, kind: 'link' }) : ''}</span>`]; }))}
      ${logDeploy ? html`<pre class="code" aria-label="Build log">${api.buildLog(s, s.deploys.find((d) => d.id === logDeploy)).join('\n')}</pre>` : ''}`;
    else if (t === 1) body = html`${s.vars.length ? table('Variables', [['Name'], ['Value'], ['', 'r']], s.vars.map(([k, v]) => [html`<span class="mono">${k}</span>`, html`<span class="mono muted">${/URL|SECRET|KEY|PASS/.test(k) ? '••••••••' : v}</span>`, btn('Remove', { act: 'rmvar', data: { k }, kind: 'link' })])) : html`<p class="muted">No variables yet.</p>`}<div style="margin-top:12px">${btn('Add variable', { act: 'addvar' })}</div>`;
    else body = html`<dl class="sm" style="display:grid;grid-template-columns:140px 1fr;gap:8px"><dt class="muted">Repository</dt><dd class="mono">${s.repo}</dd><dt class="muted">Branch</dt><dd class="mono">${s.branch}</dd><dt class="muted">Build</dt><dd>${s.build}</dd><dt class="muted">Domain</dt><dd class="mono">${s.domain || '—'}</dd><dt class="muted">Port</dt><dd class="mono">${s.port || '—'}</dd></dl><div style="margin-top:16px">${btn('Delete service', { act: 'delsvc', kind: 'danger' })}</div>`;
    return section(s.id, html`${tabs(items, t, 'tab')}<div style="padding-top:16px">${body}</div>`, s.kind === 'app' ? html`<span class="row">${s.domain ? html`<a class="btn link mono" href="https://${s.domain}" target="_blank" rel="noopener">${s.domain} ↗</a>` : ''}${btn('Redeploy', { act: 'redeploy', kind: 'primary' })}</span>` : '');
  };
  const paint = () => { root.querySelector('#g').innerHTML = graph().toString(); root.querySelector('#d').innerHTML = detail().toString(); };
  const reload = async () => { proj = await api.getProject(); paint(); };

  return {
    nav: 'apps', title: 'Apps',
    html: html`<main>${header('Apps', `${proj.name} · ${proj.env} · ${proj.services.length} services`, html`<a class="btn primary" href="#/apps/new">New service</a>`)}<div id="g"></div><div id="d"></div></main>`,
    mount(r) {
      root = r; paint();
      bindActions(root, {
        select: (el) => { sel = el.dataset.id; tab = 0; logDeploy = null; paint(); },
        tab: (el) => { tab = +el.dataset.i; paint(); },
        log: (el) => { logDeploy = logDeploy === el.dataset.id ? null : el.dataset.id; paint(); },
        redeploy: async () => { await api.redeploy(sel); toast('Deploy started.'); tab = 0; await reload(); },
        rollback: async (el) => { if (await confirmDialog({ title: `Roll back ${sel}?`, body: `Traffic moves to ${el.dataset.id}. The current version stays available.`, confirm: 'Roll back' })) { await api.rollback(sel, el.dataset.id); toast('Rolled back.'); await reload(); } },
        addvar: async () => { if (await askForm({ title: 'Add variable', fields: [{ name: 'k', label: 'Name', placeholder: 'API_KEY' }, { name: 'v', label: 'Value' }], submit: 'Add' }, (o) => api.addVar(sel, o.k.trim(), o.v))) await reload(); },
        rmvar: async (el) => { await api.removeVar(sel, el.dataset.k); await reload(); },
        delsvc: async () => { if (await confirmDialog({ title: `Delete ${sel}?`, body: 'The service stops and its container is removed. This cannot be undone.', confirm: 'Delete', danger: true, typed: sel })) { await api.removeService(sel); sel = proj.services[0]?.id; await reload(); toast('Service deleted.'); } },
      });
      this._t = () => { api.getProject().then((p) => { if (p.services.some((s, i) => s.state !== proj.services[i]?.state)) { proj = p; paint(); } }); };
      listeners.add(this._t);
    },
    unmount() { listeners.delete(this._t); },
  };
}
