import { html, st, btn, table, section, header, bindActions, confirmDialog, askForm, formDialog, toast, gb } from '../ui.js';
import * as api from '../api.js';

export async function view() {
  let d = await api.getBackups(), root;
    const body = () => html`
    ${d.jobs.some((j) => j.dest === 'Local disk') ? html`<div class="callout"><span class="tag">▲</span><span>Backups sent to <b>/var/backups</b> sit on the same disk as your data. If the instance is lost, they are lost too. Add an off-server destination.</span>${btn('Add destination', { act: 'adddest', kind: 'primary' })}</div>` : ''}
    <div class="cols"><div class="w3">
      ${section('Schedules', table('Backup schedules', [['What'], ['Schedule'], ['Keep'], ['Destination'], ['Last run'], ['', 'r']], d.jobs.map((j) => [html`<b>${j.what}</b><div class="sm muted">${j.note}</div>`, html`<span class="mono">${j.cron}</span>`, html`<span class="mono">${j.keep}</span>`, html`<span class="mono">${j.dest}</span>`, st(j.state, j.last), btn('Run now', { act: 'run', data: { id: j.id }, kind: 'link' })])))}
      ${section('Available backups', table('Backups', [['Taken'], ['What'], ['Size', 'r'], ['Where'], ['Check'], ['', 'r']], d.items.map((k) => [html`<span class="mono">${k.at}</span>`, html`<span class="mono">${k.what}</span>`, gb(k.size), k.where, st('ok', k.check), html`<span class="row" style="justify-content:flex-end">${btn('Restore', { act: 'restore', data: { id: k.id }, kind: 'link' })}${btn('Download', { act: 'dl', data: { id: k.id }, kind: 'link' })}</span>`])))}
    </div><div>
      ${section('Destinations', table('Destinations', [['Name'], ['Location'], ['Status']], d.dests.map((x) => [x.name, html`<span class="mono sm">${x.loc}</span>`, x.ok ? st('ok', 'verified') : st('warn', 'not off-server')])), html`<a class="btn link" href="#/backups/new">+ Add</a>`)}
      ${section('Restore, safely', html`<p class="muted sm">Restoring never overwrites live data by default. opsbox restores into a copy next to the original; you check it, then swap it in or delete it.</p>`)}
      ${section('Encryption', html`<p class="muted sm">Backups are encrypted before they leave this server. Without the recovery key a backup cannot be opened.</p><div class="row">${btn('Download recovery key', { act: 'key' })}</div>`)}
    </div></div>`;
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  const reload = async () => { d = await api.getBackups(); paint(); };
  return {
    nav: 'backups', title: 'Backups',
    html: html`<main>${header('Backups', 'Scheduled, encrypted, restore-tested', html`<span class="row"><a class="btn" href="#/backups/new">Add destination</a>${btn('Back up now', { act: 'run', data: { id: '' }, kind: 'primary' })}</span>`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      bindActions(root, {
        run: async (el) => { await api.runBackup(el.dataset.id); toast('Backup finished.'); await reload(); },
        adddest: () => { location.hash = '#/backups/new'; },
        restore: async (el) => {
          const o = await formDialog({ title: 'Restore this backup', body: 'Restoring to a copy is safe: nothing live changes.', fields: [{ name: 'mode', label: 'Restore as', options: ['copy', 'replace'], value: 'copy', hint: 'replace overwrites the live data' }], submit: 'Restore' });
          if (!o) return;
          if (o.mode === 'replace' && !await confirmDialog({ title: 'Replace live data?', body: 'The current data is overwritten. This cannot be undone.', confirm: 'Replace', danger: true, typed: 'replace' })) return;
          const r = await api.restoreBackup(el.dataset.id, o.mode); toast(r.msg);
        },
        dl: () => toast('Download starts in the real build.'),
        key: () => toast('The recovery key downloads in the real build.'),
      });
    },
  };
}
