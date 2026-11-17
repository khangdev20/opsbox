// App shell: top bar, section nav, status bar, command palette.
import { html, raw, esc, toast } from './ui.js';
import * as api from './api.js';
import { theme, toggleTheme, session } from './store.js';

const ICONS = {"overview": "<rect x=\"3\" y=\"3\" width=\"7\" height=\"7\" rx=\"1.5\"/><rect x=\"14\" y=\"3\" width=\"7\" height=\"7\" rx=\"1.5\"/><rect x=\"3\" y=\"14\" width=\"7\" height=\"7\" rx=\"1.5\"/><rect x=\"14\" y=\"14\" width=\"7\" height=\"7\" rx=\"1.5\"/>", "apps": "<path d=\"M12 3l9 5-9 5-9-5 9-5z\"/><path d=\"M3 13l9 5 9-5\"/>", "containers": "<path d=\"M21 8l-9-5-9 5v8l9 5 9-5V8z\"/><path d=\"M3 8l9 5 9-5M12 13v8\"/>", "system": "<circle cx=\"12\" cy=\"12\" r=\"3\"/><path d=\"M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2\"/>", "backups": "<ellipse cx=\"12\" cy=\"6\" rx=\"8\" ry=\"3\"/><path d=\"M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6\"/>", "alerts": "<path d=\"M6 9a6 6 0 0112 0c0 6 2 7 2 7H4s2-1 2-7z\"/><path d=\"M10 20a2 2 0 004 0\"/>", "cleanup": "<path d=\"M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13\"/><path d=\"M10 11v6M14 11v6\"/>", "terminal": "<rect x=\"3\" y=\"4\" width=\"18\" height=\"16\" rx=\"2\"/><path d=\"M7 9l3 3-3 3M12 15h5\"/>", "files": "<path d=\"M3 6a2 2 0 012-2h4l2 2h8a2 2 0 012 2v10a2 2 0 01-2 2H5a2 2 0 01-2-2V6z\"/>", "security": "<path d=\"M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6l8-3z\"/><path d=\"M9 12l2 2 4-4\"/>"};
// End the session on the server first, so the cookie is really gone, then leave.
async function signOut() {
  try { await api.auth.logout(); } catch { /* already signed out elsewhere */ }
  session.clear(); location.hash = '#/login';
}

// Name shown in the top bar and status bar. In http mode it is the real hostname.
let hostName = 'srv-sgn-01';
async function loadHostName() {
  if (api.MODE !== 'http') return;
  try { hostName = (await api.quiet(() => api.getHost())).name; } catch { return; }
  const crumb = document.querySelector('.crumb'); if (crumb) crumb.textContent = hostName;
  refreshStatus();
}
export const NAV = [
  ['01', 'Overview', '#/', 'overview'], ['02', 'Apps', '#/apps', 'apps'], ['03', 'Containers', '#/containers', 'containers'],
  ['04', 'System', '#/system', 'system'], ['05', 'Backups', '#/backups', 'backups'], ['06', 'Alerts', '#/alerts', 'alerts'],
  ['07', 'Cleanup', '#/cleanup', 'cleanup'], ['08', 'Terminal', '#/terminal', 'terminal'], ['09', 'Files', '#/files', 'files'], ['10', 'Security', '#/security', 'security'],
];

export function mountShell(app) {
  app.innerHTML = `
  <header class="topbar"><div class="wrap">
    <a class="logo" href="#/" aria-label="opsbox home">opsbox<i>_</i></a>
    <span class="crumb">${api.MODE === 'http' ? esc(hostName) : 'srv-sgn-01<s>/</s>shop-platform<s>/</s><b>production</b>'}</span>
    <span class="grow"></span>
    <button type="button" class="btn" id="open-pal" style="color:var(--muted);gap:28px;font-weight:400;padding:0 10px 0 14px">Search or run a command<kbd class="kbd">Ctrl K</kbd></button>
    <button type="button" class="btn icon" id="theme-btn"></button>
    <a class="btn link" href="#/settings" id="settings-link" style="color:var(--muted)">Settings</a>
    <button type="button" class="btn link" id="sign-out"></button>
  </div></header>
  <div class="shell">
    <aside class="side" aria-label="Main"><button type="button" class="side-toggle" id="side-toggle" aria-label="Collapse sidebar" title="Collapse sidebar"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 10l-2 2 2 2"/></svg></button><nav id="nav" aria-label="Sections"></nav><nav id="nav2" aria-label="More" class="nav2"></nav></aside>
    <div class="content">
      <div id="view" style="flex:1;display:flex;flex-direction:column"></div>
      <footer class="statusbar"><div class="wrap" id="status"></div></footer>
    </div>
  </div>`;
  const label = () => { const b = document.getElementById('theme-btn'); const l = theme() === 'light'; b.innerHTML = l ? '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.8A9 9 0 1111.2 3a7 7 0 009.8 9.8z"/></svg>' : '<svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg>'; b.setAttribute('aria-label', l ? 'Switch to dark theme' : 'Switch to light theme'); b.title = b.getAttribute('aria-label'); };
  label(); loadHostName();
  document.getElementById('theme-btn').addEventListener('click', () => { toggleTheme(); label(); });
  document.getElementById('sign-out').textContent = `${session.user || 'admin'} · sign out`;
  document.getElementById('sign-out').addEventListener('click', signOut);
  document.getElementById('open-pal').addEventListener('click', openPalette);
  const shell = app.querySelector('.shell'), tg = document.getElementById('side-toggle');
  const setC = (c) => { shell.classList.toggle('collapsed', c); const l = c ? 'Expand sidebar' : 'Collapse sidebar'; tg.setAttribute('aria-label', l); tg.title = l; try { localStorage.setItem('opsbox.side', c ? '1' : '0'); } catch {} };
  let c0 = false; try { c0 = localStorage.getItem('opsbox.side') === '1'; } catch {} setC(c0);
  tg.addEventListener('click', () => setC(!shell.classList.contains('collapsed')));
  return document.getElementById('view');
}

export function setNav(active) {
  const nav = document.getElementById('nav'); if (!nav) return;
  nav.innerHTML = NAV.map(([n, l, h, id]) => `<a href="${h}" title="${l}"${id === active ? ' aria-current="page"' : ''}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[id]}</svg><span class="lbl">${l}</span></a>`).join('');
  const more = [['Access', '#/security/access', 'access', '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M17 6l3 3M14 9l2 2"/>'], ['Settings', '#/settings', 'settings', '<path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0"/><circle cx="16" cy="6" r="2"/><circle cx="10" cy="12" r="2"/><circle cx="18" cy="18" r="2"/>']];
  const n2 = document.getElementById('nav2'); const hash = location.hash;
  if (n2) n2.innerHTML = more.map(([l, h, id, ic]) => `<a href="${h}" title="${l}"${hash.startsWith(h) ? ' aria-current="page"' : ''}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ic}</svg><span class="lbl">${l}</span></a>`).join('');
}

export async function refreshStatus() {
  const el = document.getElementById('status'); if (!el) return;
  const m = await api.quiet(() => api.getMetrics()); const d = m.disk / m.diskTotal * 100;
  el.innerHTML = `<span style="color:var(--ok)">■ ${esc(hostName)}</span><span>cpu <b>${Math.round(m.cpuNow)}%</b></span><span>mem <b>${m.memUsed.toFixed(1)}/${m.memTotal.toFixed(1)} GB</b></span><span>disk / <b style="color:${d > 75 ? 'var(--warn)' : 'var(--text)'}">${Math.round(d)}%</b></span><span>net <b>↓${m.netIn.toFixed(1)} ↑${m.netOut.toFixed(1)} MB/s</b></span><span class="grow"></span><span>${api.MODE === 'http' && !api.isReal('getMetrics') ? 'sample metrics' : api.MODE === 'http' ? 'live' : 'mock data'}</span><span>opsbox 0.1.0</span>`;
}

// ---------------- command palette ----------------
const ACTIONS = () => [
  ...NAV.map(([n, l, h]) => ({ label: `Go to ${l}`, hint: n, run: () => { location.hash = h; } })),
  { label: 'Go to Settings', hint: 'settings', run: () => { location.hash = '#/settings'; } },
  { label: 'Domains and SSL for web', hint: 'apps', run: () => { location.hash = '#/apps?s=web&tab=domains'; } },
  { label: 'Add backup destination (S3, R2, B2…)', hint: 'backups', run: () => { location.hash = '#/backups/new'; } },
  { label: 'Add a database or template', hint: 'apps', run: () => { location.hash = '#/apps/new/templates'; } },
  { label: 'Users and SSH keys', hint: 'security', run: () => { location.hash = '#/security/access'; } },
  { label: 'New service from GitHub', hint: 'apps', run: () => { location.hash = '#/apps/new'; } },
  { label: 'Toggle light / dark theme', hint: 'theme', run: () => { toggleTheme(); document.getElementById('theme-btn').textContent = theme() === 'light' ? 'Dark' : 'Light'; } },
  { label: 'Open emergency SSH for 1 hour', hint: 'security', run: async () => { await api.openSsh(); toast('Port 22 is open to your IP for one hour.'); } },
  { label: 'Open safe mode page', hint: 'recovery', run: () => { location.hash = '#/recovery'; } },
  { label: 'Sign out', hint: 'session', run: signOut },
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
