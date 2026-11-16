import { html, raw, gb, section, header, sw, btn, bindActions, confirmDialog, toast } from '../ui.js';
import * as api from '../api.js';

const COL = { accent: 'var(--accent)', text: 'var(--text)', warn: 'var(--warn)', muted: 'var(--muted)', ls: 'var(--ls)' };
export async function view() {
  let d = await api.scanCleanup(); const picked = new Set(d.items.filter((i) => i.on).map((i) => i.id)); let root;
  const sum = () => d.items.filter((i) => picked.has(i.id)).reduce((a, i) => a + i.gb, 0);
  const body = () => {
    const used = d.disk, free = d.total - used, rec = d.items.reduce((a, i) => a + i.gb, 0);
    const parts = [[d.system, 'var(--ls)', 'System'], ...d.items.map((i) => [i.gb, COL[i.color], i.id])];
    const other = Math.max(0, used - parts.reduce((a, p) => a + p[0], 0)); parts.push([other, 'var(--muted)', 'Apps and data']);
    return html`<div class="seg" role="img" aria-label="Disk usage breakdown">${parts.map(([g, c]) => html`<i style="flex:${g};background:${c}"></i>`)}<i style="flex:${free};background:var(--line)"></i></div>
      <p class="sm muted mono" style="margin:8px 0 24px">${Math.round(used)} of ${d.total} GB used · ${gb(rec)} reclaimable · scanned ${d.scanned}</p>
      ${d.items.length ? d.items.map((i) => html`<div class="item-row"><label><input type="checkbox" data-act="pick" data-id="${i.id}" ${raw(picked.has(i.id) ? 'checked' : '')}><span><b>${i.title}</b><span class="sm ${i.warn ? '' : 'muted'}" ${raw(i.warn ? 'style="color:var(--warn)"' : '')}>${i.desc}</span></span></label><span class="mono">${gb(i.gb)}</span></div>`) : html`<p class="st ok">■ Nothing to clean.</p>`}
      <div class="row" style="margin-top:20px">${btn(`Clean ${gb(sum())}`, { act: 'run', kind: 'primary', disabled: !picked.size })}<span class="sm muted">${picked.size} selected</span></div>`;
  };
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  return {
    nav: 'cleanup', title: 'Cleanup',
    html: html`<main class="narrow">${header('Cleanup', 'Free disk space without touching running apps.', html`<span class="row"><span class="sm muted">Weekly auto-clean</span>${sw('Weekly auto-clean', d.auto, 'auto')}</span>`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      bindActions(root, {
        pick: (el) => { el.checked ? picked.add(el.dataset.id) : picked.delete(el.dataset.id); paint(); },
        run: async () => {
          const sel = d.items.filter((i) => picked.has(i.id));
          if (!await confirmDialog({ title: `Free ${gb(sum())}?`, body: `This removes: ${sel.map((i) => i.title).join('; ')}.`, confirm: 'Clean now', danger: sel.some((i) => i.warn) })) return;
          const r = await api.runCleanup([...picked]); toast(`Freed ${gb(r.freed)}.`); d = await api.scanCleanup(); picked.clear(); d.items.filter((i) => i.on).forEach((i) => picked.add(i.id)); paint();
        },
        auto: async (el) => { const on = el.getAttribute('aria-checked') !== 'true'; await api.setAutoCleanup(on); el.setAttribute('aria-checked', on); el.textContent = on ? 'ON' : 'OFF'; },
      });
    },
  };
}
