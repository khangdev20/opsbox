import { html, raw, ago, st, btn, tabs, table, section, header, bindActions, confirmDialog, toast, gb } from '../ui.js';
import * as api from '../api.js';
import { listeners } from '../store.js';

const KINDS = ['containers', 'images', 'volumes', 'networks'];
export async function view() {
  let tab = 0, data = await api.listContainers(), logs = null, root;
  const load = async () => { data = tab === 0 ? await api.listContainers() : await api.listDocker(KINDS[tab]); };
  const body = () => {
    if (tab === 0) return html`${table('Containers', [['Name'], ['Image'], ['State'], ['CPU', 'r'], ['Mem', 'r'], ['Ports'], ['', 'r']], data.map((c) => [
      html`<span class="mono">${c.name}</span>`, html`<span class="mono muted">${c.image}</span>`,
      c.state === 'running' ? st(c.restarts ? 'warn' : 'ok', c.restarts ? `running · ${c.restarts} restarts` : `running ${ago(c.since).replace(' ago', '')}`) : st('off', 'exited'),
      c.state === 'running' ? `${c.cpu.toFixed(1)}%` : '–', c.state === 'running' ? `${c.mem} MB` : '–', c.ports,
      html`<span class="row" style="justify-content:flex-end">${btn('Logs', { act: 'logs', data: { id: c.id }, kind: 'link' })}${c.state === 'running' ? html`${btn('Restart', { act: 'do', data: { id: c.id, a: 'restart' }, kind: 'link' })}${btn('Stop', { act: 'do', data: { id: c.id, a: 'stop' }, kind: 'link' })}` : html`${btn('Start', { act: 'do', data: { id: c.id, a: 'start' }, kind: 'link' })}${btn('Delete', { act: 'del', data: { id: c.id }, kind: 'link' })}`}</span>`]))}
      ${logs ? html`<pre class="code" aria-label="Container logs">${logs.join('\n')}</pre>` : ''}`;
    if (tab === 1) return table('Images', [['Image'], ['Tag'], ['Size', 'r'], ['Used by']], data.map((i) => [html`<span class="mono">${i.name}</span>`, i.tag, gb(i.size, 2), i.used ? st('ok', 'in use') : st('warn', 'unused')]));
    if (tab === 2) return table('Volumes', [['Volume'], ['Size', 'r'], ['Attached']], data.map((v) => [html`<span class="mono">${v.name}</span>`, gb(v.size, 2), v.used ? st('ok', 'attached') : st('warn', 'not attached')]));
    return table('Networks', [['Network'], ['Driver']], data.map((n) => [html`<span class="mono">${n.name}</span>`, n.driver]));
  };
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); root.querySelector('#t').innerHTML = tabs(['Containers', 'Images', 'Volumes', 'Networks'], tab).toString(); };
  return {
    nav: 'containers', title: 'Containers',
    html: html`<main>${header('Containers', `Docker 27.3 · ${data.filter((c) => c.state === 'running').length} running`, html`<a class="btn" href="#/cleanup">Clean up unused</a>`)}<div id="t"></div><div id="b" style="padding-top:16px"></div></main>`,
    mount(r) {
      root = r; paint();
      bindActions(root, {
        tab: async (el) => { tab = +el.dataset.i; logs = null; await load(); paint(); },
        logs: async (el) => { logs = await api.containerLogs(el.dataset.id); paint(); },
        do: async (el) => { await api.containerAction(el.dataset.id, el.dataset.a); toast(`${el.dataset.a} done.`); await load(); paint(); },
        del: async (el) => { const c = data.find((x) => x.id === el.dataset.id); if (await confirmDialog({ title: `Delete ${c.name}?`, body: 'The container is removed. Volumes are kept.', confirm: 'Delete', danger: true })) { await api.containerAction(c.id, 'delete'); await load(); paint(); } },
      });
      this._t = async () => { if (tab === 0 && !logs) { data = await api.listContainers(); paint(); } };
      listeners.add(this._t);
    },
    unmount() { listeners.delete(this._t); },
  };
}
