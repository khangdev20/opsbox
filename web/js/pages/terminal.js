import { html, esc } from '../ui.js';
import * as api from '../api.js';

export async function view() {
  const st = Object.fromEntries(api.HOSTS.map((h) => [h.id, { cwd: '/home/deploy', lines: [`Connected to ${h.name}. Type "help" for demo commands.`], hist: [], hi: 0 }]));
  let cur = 'local'; let root;
  const ps = (h) => `${h.user}@${h.name}:${st[h.id].cwd}$`;
  const paint = () => {
    const h = api.HOSTS.find((x) => x.id === cur), s = st[cur];
    root.querySelector('#hosts').innerHTML = api.HOSTS.map((x) => `<button type="button" class="host ${x.id === cur ? 'on' : ''}" data-h="${x.id}"><b>${esc(x.name)}</b><span class="sm muted">${esc(x.sub)}</span></button>`).join('');
    const body = root.querySelector('#out'); body.innerHTML = s.lines.map((l) => `<div class="term-line">${esc(l)}</div>`).join('');
    root.querySelector('#ps').textContent = ps(h); body.scrollTop = body.scrollHeight;
    root.querySelector('#in').focus();
  };
  return {
    nav: 'terminal', title: 'Terminal',
    html: html`<main><div class="hdr"><div><h1>Terminal</h1><div class="meta">Demo shell. The real build attaches a PTY to your server over WebSocket.</div></div></div>
      <div class="split"><div style="flex:0 0 240px"><div id="hosts" class="hosts" role="tablist"></div></div>
      <div class="term" style="flex:1 1 480px;min-width:0"><div class="term-body" id="out" aria-live="polite" style="min-height:360px;max-height:60vh;overflow:auto"></div>
        <form class="term-in" id="f"><span class="ps-u" id="ps"></span><input id="in" aria-label="Command" autocomplete="off" spellcheck="false" style="flex:1;background:transparent;border:0;outline:0;color:inherit;font:inherit"></form>
        <div class="term-foot">↑ ↓ history · type help</div></div></div></main>`,
    mount(r) {
      root = r; paint();
      root.querySelector('#hosts').addEventListener('click', (e) => { const b = e.target.closest('[data-h]'); if (b) { cur = b.dataset.h; paint(); } });
      const inp = root.querySelector('#in');
      inp.addEventListener('keydown', (e) => {
        const s = st[cur]; if (e.key !== 'ArrowUp' && e.key !== 'ArrowDown') return; e.preventDefault();
        s.hi = Math.min(s.hist.length, Math.max(0, s.hi + (e.key === 'ArrowUp' ? -1 : 1))); inp.value = s.hist[s.hi] ?? '';
      });
      root.querySelector('#f').addEventListener('submit', async (e) => {
        e.preventDefault(); const s = st[cur], h = api.HOSTS.find((x) => x.id === cur), line = inp.value; inp.value = '';
        s.lines.push(`${ps(h)} ${line}`); if (line.trim()) { s.hist.push(line); } s.hi = s.hist.length;
        const res = await api.termExec(cur, s.cwd, line);
        if (res.clear) s.lines = []; if (res.out) s.lines.push(...res.out.split('\n')); if (res.err) s.lines.push(res.err); if (res.cwd) s.cwd = res.cwd;
        paint();
      });
    },
  };
}
