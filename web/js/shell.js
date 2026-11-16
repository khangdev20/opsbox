// App shell: top bar, section nav, status bar, command palette.
import { html, raw, esc, toast } from './ui.js';
import * as api from './api.js';
import { theme, toggleTheme, session } from './store.js';

export const NAV = [
  ['01', 'Overview', '#/', 'overview'], ['02', 'Apps', '#/apps', 'apps'], ['03', 'Containers', '#/containers', 'containers'],
  ['04', 'System', '#/system', 'system'], ['05', 'Backups', '#/backups', 'backups'], ['06', 'Alerts', '#/alerts', 'alerts'],
  ['07', 'Cleanup', '#/cleanup', 'cleanup'], ['08', 'Terminal', '#/terminal', 'terminal'], ['09', 'Files', '#/files', 'files'], ['10', 'Security', '#/security', 'security'],
];

export function mountShell(app) {
  app.innerHTML = `
  <header class="topbar"><div class="wrap">
    <a class="logo" href="#/" aria-label="opsbox home">opsbox<i>_</i></a>
    <span class="crumb">srv-sgn-01<s>/</s>shop-platform<s>/</s><b>production</b></span>
    <span class="grow"></span>
    <button type="button" class="btn" id="open-pal" style="color:var(--muted);gap:28px;font:400 12px var(--mono);padding:0 10px 0 14px">Search or run a command<span style="border:1px solid var(--ctl);border-radius:2px;padding:1px 6px;font-size:11px">Ctrl K</span></button>
    <button type="button" class="btn" id="theme-btn"></button>
    <a class="btn link mono" href="#/settings" id="settings-link" style="font-size:13px;color:var(--muted)">Settings</a>
    <button type="button" class="btn link mono" id="sign-out" style="font-size:13px"></button>
  </div></header>
  <nav class="tabs-nav" aria-label="Sections"><div class="wrap" id="nav"></div></nav>
  <div id="view" style="flex:1;display:flex;flex-direction:column"></div>
  <footer class="statusbar"><div class="wrap" id="status"></div></footer>`;
  const label = () => { document.getElementById('theme-btn').textContent = theme() === 'light' ? 'Dark' : 'Light'; };
  label();
  document.getElementById('theme-btn').addEventListener('click', () => { toggleTheme(); label(); });
  document.getElementById('sign-out').textContent = `${session.user || 'admin'} · sign out`;
  document.getElementById('sign-out').addEventListener('click', () => { session.clear(); location.hash = '#/login'; });
  document.getElementById('open-pal').addEventListener('click', openPalette);
  return document.getElementById('view');
}

export function setNav(active) {
  const nav = document.getElementById('nav'); if (!nav) return;
  nav.innerHTML = NAV.map(([n, l, h, id]) => `<a href="${h}"${id === active ? ' aria-current="page"' : ''}><span>${n}</span>${l}</a>`).join('');
}

export async function refreshStatus() {
  const el = document.getElementById('status'); if (!el) return;
  const m = await api.getMetrics(); const d = m.disk / m.diskTotal * 100;
  el.innerHTML = `<span style="color:var(--ok)">■ srv-sgn-01</span><span>cpu <b>${Math.round(m.cpuNow)}%</b></span><span>mem <b>${m.memUsed.toFixed(1)}/${m.memTotal.toFixed(1)} GB</b></span><span>disk / <b style="color:${d > 75 ? 'var(--warn)' : 'var(--text)'}">${Math.round(d)}%</b></span><span>net <b>↓${m.netIn.toFixed(1)} ↑${m.netOut.toFixed(1)} MB/s</b></span><span class="grow"></span><span>${api.MODE} data</span><span>opsbox 0.1.0</span>`;
}

// ---------------- command palette ----------------
const ACTIONS = () => [
  ...NAV.map(([n, l, h]) => ({ label: `Go to ${l}`, hint: n, run: () => { location.hash = h; } })),
  { label: 'Go to Settings', hint: 'settings', run: () => { location.hash = '#/settings'; } },
  { label: 'Add a database or template', hint: 'apps', run: () => { location.hash = '#/apps/new/templates'; } },
  { label: 'Users and SSH keys', hint: 'security', run: () => { location.hash = '#/security/access'; } },
  { label: 'New service from GitHub', hint: 'apps', run: () => { location.hash = '#/apps/new'; } },
  { label: 'Toggle light / dark theme', hint: 'theme', run: () => { toggleTheme(); document.getElementById('theme-btn').textContent = theme() === 'light' ? 'Dark' : 'Light'; } },
  { label: 'Open emergency SSH for 1 hour', hint: 'security', run: async () => { await api.openSsh(); toast('Port 22 is open to your IP for one hour.'); } },
  { label: 'Open safe mode page', hint: 'recovery', run: () => { location.hash = '#/recovery'; } },
  { label: 'Sign out', hint: 'session', run: () => { session.clear(); location.hash = '#/login'; } },
];
export function openPalette() {
  if (document.getElementById('pal')) return;
  const d = document.createElement('dialog'); d.id = 'pal'; d.className = 'pal';
  d.innerHTML = '<input aria-label="Search commands" placeholder="Type a command or page…" autocomplete="off"><ul role="listbox"></ul>';
  document.body.append(d);
  const inp = d.querySelector('input'), ul = d.querySelector('ul'); let items = [], sel = 0;
  const paint = () => {
    const q = inp.value.toLowerCase(); items = ACTIONS().filter((a) => a.label.toLowerCase().includes(q));
    sel = Math.min(sel, Math.max(0, items.length - 1));
    ul.innerHTML = items.map((a, i) => `<li role="option" data-i="${i}" aria-selected="${i === sel}">${esc(a.label)}<small>${esc(a.hint)}</small></li>`).join('') || '<li class="muted">No match</li>';
  };
  const run = (i) => { const a = items[i]; if (!a) return; d.close(); a.run(); };
  inp.addEventListener('input', () => { sel = 0; paint(); });
  inp.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') { sel = (sel + 1) % Math.max(1, items.length); paint(); e.preventDefault(); }
    else if (e.key === 'ArrowUp') { sel = (sel - 1 + items.length) % Math.max(1, items.length); paint(); e.preventDefault(); }
    else if (e.key === 'Enter') { run(sel); e.preventDefault(); }
  });
  ul.addEventListener('click', (e) => { const li = e.target.closest('li[data-i]'); if (li) run(+li.dataset.i); });
  d.addEventListener('close', () => d.remove()); d.addEventListener('click', (e) => { if (e.target === d) d.close(); });
  paint(); d.showModal(); inp.focus();
}
document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k' && document.getElementById('open-pal')) { e.preventDefault(); openPalette(); }
});
