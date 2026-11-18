import { html, raw, bytes, header, sw, btn, bindActions, confirmDialog, toast, patch, num, segBar, sk, setSwitch, empty, refreshing } from '../ui.js';
import * as api from '../api.js';

const COL = { accent: 'var(--accent)', text: 'var(--text)', warn: 'var(--warn)', muted: 'var(--muted)', ls: 'var(--ls)' };

function ageLabel(scannedAt, fallback) {
  if (!scannedAt) return fallback || 'just now';
  const s = Math.max(0, Math.floor(Date.now() / 1000) - scannedAt);
  if (s < 5) return 'just now';
  if (s < 60) return `${s} s ago`;
  return `${Math.floor(s / 60)} min ago`;
}

export async function view() {
  let d = null, root = null, timer = null, dead = false;
  const picked = new Set();
  const by = (i) => i.bytes ?? (i.gb || 0) * 2 ** 30;
  const sum = () => (d ? d.items.filter((i) => !i.pending && picked.has(i.id)).reduce((a, i) => a + by(i), 0) : 0);

  // Same shape as the real content, so nothing shifts when the scan result arrives.
  const skeleton = () => html`
    <span class="sk" style="height:10px;border-radius:5px"></span>
    <p class="sm muted mono" style="margin:8px 0 24px">Scanning disk…</p>
    ${[1, 2, 3].map(() => html`<div class="item-row"><span class="col gap6">${sk(16, '180px')}${sk(12, '280px')}</span>${sk(16, '60px')}</div>`)}`;

  const body = () => {
    if (!d) return skeleton();
    const used = d.disk || 0, total = d.total || 0, free = Math.max(0, total - used);
    const rec = d.items.filter((i) => !i.pending).reduce((a, i) => a + by(i), 0);
    const parts = [];
    if (d.system > 0) parts.push([d.system, 'var(--ls)', 'System']);
    for (const i of d.items) {
      if (!i.pending && i.gb > 0) parts.push([i.gb, COL[i.color] || 'var(--ls)', i.id]);
    }
    const other = Math.max(0, used - parts.reduce((a, p) => a + p[0], 0));
    if (other > 0 || !parts.length) parts.push([other, 'var(--muted)', 'Apps and data']);

    return html`${segBar([...parts.map(([g, c]) => [g, c]), ...(free > 0 ? [[free, 'var(--line)']] : [])], 'Disk usage breakdown')}
      <p class="sm muted mono" style="margin:8px 0 24px">${Math.round(used)} of ${total} GB used · ${num(rec, 'bytes')} reclaimable · scanned ${ageLabel(d.scannedAt, d.scanned)} · ${btn('Rescan', { act: 'rescan', kind: 'link' })}${d.pending ? ' · scanning…' : ''}</p>
      ${d.items.length ? d.items.map((i) => html`<div class="item-row" data-key="${i.id}" data-sig="${i.pending}"><label><input type="checkbox" data-act="pick" data-id="${i.id}" ${raw(picked.has(i.id) ? 'checked' : '')} ${raw(i.pending ? 'disabled' : '')}><span><b>${i.title}</b><span class="sm ${i.warn ? '' : 'muted'}" ${raw(i.warn ? 'style="color:var(--warn)"' : '')}>${i.desc}</span></span></label><span class="mono">${i.pending ? 'scanning…' : (i.id === 'docker-volumes' && !by(i) ? btn('Calculate size', { act: 'volSize', kind: 'link' }) : num(by(i), 'bytes'))}</span></div>`) : empty('Nothing to clean. Your disk is tidy.')}
      <div class="row" style="margin-top:20px">${btn(html`Clean ${num(sum(), 'bytes')}`, { act: 'run', kind: 'primary', disabled: !picked.size })}<span class="sm muted">${picked.size} selected</span></div>`;
  };

  const paint = () => {
    if (!root || dead) return;
    patch(root.querySelector('#b'), body().toString());
    if (d) setSwitch(root.querySelector('[data-act="auto"]'), !!d.auto);
  };

  const load = async (fresh = false) => {
    clearTimeout(timer);
    try {
      const next = await api.scanCleanup(fresh);
      if (dead) return;
      const firstLoad = !d;
      d = next;
      if (firstLoad || fresh) {
        picked.clear();
        d.items.filter((i) => !i.pending && i.on).forEach((i) => picked.add(i.id));
      } else {
        for (const i of d.items) {
          if (!i.pending && i.on && !picked.has(i.id)) picked.add(i.id);
        }
      }
      paint();
      if (d.pending && !dead) {
        timer = setTimeout(() => load(false), 400);
      }
    } catch (e) {
      if (!dead) toast(e.message, 'err');
    }
  };

  return {
    nav: 'cleanup', title: 'Cleanup',
    html: html`<main class="narrow">${header('Cleanup', 'Free disk space without touching running apps.', html`<span class="row"><span class="sm muted">Weekly auto-clean</span>${sw('Weekly auto-clean', false, 'auto')}</span>`)}<div id="b">${skeleton()}</div></main>`,
    mount(r) {
      root = r;
      load(false);
      bindActions(root, {
        pick: (el) => { el.checked ? picked.add(el.dataset.id) : picked.delete(el.dataset.id); paint(); },
        rescan: () => refreshing(root.querySelector('#b'), () => load(true)),
        volSize: async () => {
          try {
            const v = await api.getCleanupVolumes();
            const item = d?.items.find((i) => i.id === 'docker-volumes');
            if (item) {
              item.bytes = v.bytes;
              item.gb = v.gb;
              paint();
            }
          } catch (e) { toast(e.message, 'err'); }
        },
        run: async () => {
          const sel = d.items.filter((i) => picked.has(i.id));
          if (!await confirmDialog({ title: `Free ${bytes(sum())}?`, body: `This removes: ${sel.map((i) => i.title).join('; ')}.`, confirm: 'Clean now', danger: sel.some((i) => i.warn) })) return;
          const res = await api.runCleanup([...picked]);
          const bad = (res.items || []).filter((x) => x.error);
          const got = res.bytes ?? res.freed * 2 ** 30;
          toast(bad.length ? `Freed ${bytes(got)}. ${bad.length} could not finish: ${bad.map((x) => `${x.id}: ${x.error}`).join('; ')}` : `Freed ${bytes(got)}.`, bad.length ? 'err' : '');
          // Update cleaned items in place immediately, then refresh in background.
          const okIds = new Set((res.items || []).filter((x) => !x.error).map((x) => x.id));
          d.items = d.items.filter((i) => !okIds.has(i.id));
          for (const id of okIds) picked.delete(id);
          paint();
          load(true);
        },
        auto: async (el) => { const on = el.getAttribute('aria-checked') !== 'true'; await api.setAutoCleanup(on); setSwitch(el, on); },
      });
    },
    unmount() { dead = true; clearTimeout(timer); },
  };
}
