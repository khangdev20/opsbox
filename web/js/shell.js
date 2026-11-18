// App shell: top bar, section nav, status bar, command palette.
import { html, raw, esc, toast, num, patch, withBusy, reportPoll, liveState, onLive, clock, logo, initials } from './ui.js';
import * as api from './api.js';
import { theme, toggleTheme, session, server } from './store.js';

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
// The sidebar: Overview on its own, then labelled groups (labels become thin dividers when collapsed).
// Access and Settings live in the account menu; every route is still reachable from here or there.
const GROUPS = [
  ['', ['overview']], ['Deploy', ['apps', 'containers']], ['Server', ['system', 'terminal', 'files', 'cleanup']], ['Protect', ['backups', 'alerts', 'security']],
];
const svg = (paths, n = 20) => `<svg width="${n}" height="${n}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
const MENU_ICONS = {
  profile: '<circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 4-6 8-6s8 2 8 6"/>',
  settings: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.6 1.6 0 00.3 1.8l.1.1a2 2 0 11-2.8 2.8l-.1-.1a1.6 1.6 0 00-1.8-.3 1.6 1.6 0 00-1 1.5V21a2 2 0 11-4 0v-.1a1.6 1.6 0 00-1-1.5 1.6 1.6 0 00-1.8.3l-.1.1a2 2 0 11-2.8-2.8l.1-.1a1.6 1.6 0 00.3-1.8 1.6 1.6 0 00-1.5-1H3a2 2 0 110-4h.1a1.6 1.6 0 001.5-1 1.6 1.6 0 00-.3-1.8l-.1-.1a2 2 0 112.8-2.8l.1.1a1.6 1.6 0 001.8.3H9a1.6 1.6 0 001-1.5V3a2 2 0 114 0v.1a1.6 1.6 0 001 1.5 1.6 1.6 0 001.8-.3l.1-.1a2 2 0 112.8 2.8l-.1.1a1.6 1.6 0 00-.3 1.8V9a1.6 1.6 0 001.5 1H21a2 2 0 110 4h-.1a1.6 1.6 0 00-1.5 1z"/>',
  tokens: '<rect x="3" y="11" width="18" height="10" rx="2"/><path d="M7 11V7a5 5 0 0110 0v4"/>',
  out: '<path d="M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4M16 17l5-5-5-5M21 12H9"/>',
  chev: '<path d="M6 9l6 6 6-6"/>',
};
const ROLES = { admin: 'Owner', deployer: 'Deployer', viewer: 'Viewer' };
// The panel only knows the sign-in name and role. Every browser session passed a 2FA code (sign-in requires one).
export const roleLabel = () => ROLES[server.role] || (api.MODE === 'mock' ? 'Owner' : '');

// `demo` (UI kit): fixed identity, menu shown open, no ids so the page-level handlers ignore it.
export function accountMenu({ user = session.user || 'admin', role = roleLabel(), demo = false } = {}) {
  const item = (href, icon, label, extra = '') => html`<a href="${href}" role="menuitem" tabindex="-1" class="menu-item${extra}"><span class="mi">${raw(svg(MENU_ICONS[icon], 16))}</span>${label}</a>`;
  return html`<div class="acct">
    <button type="button" class="acct-btn" ${raw(demo ? '' : 'id="acct-btn" aria-controls="acct-menu"')} aria-haspopup="menu" aria-expanded="${String(demo)}" aria-label="Account menu for ${user}">
      <span class="avatar" aria-hidden="true">${initials(user)}</span>
      <span class="acct-name"><span>${user}</span>${role ? html`<span class="muted">${role}</span>` : ''}</span>${raw(svg(MENU_ICONS.chev, 16))}
    </button>
    <div class="menu" ${raw(demo ? '' : 'id="acct-menu" hidden')} role="menu" aria-label="Account">
      <div class="menu-head"><div class="menu-name">${user}</div><div class="muted sm">${[role, '2FA on'].filter(Boolean).join(' · ')}</div></div>
      ${item('#/security/access', 'profile', 'Profile and security')}${item('#/settings', 'settings', 'Server settings')}${item('#/settings?section=tokens', 'tokens', 'API tokens')}
      <div class="menu-sep" role="separator"></div>
      <button type="button" role="menuitem" tabindex="-1" class="menu-item danger" ${raw(demo ? '' : 'id="sign-out"')}><span class="mi">${raw(svg(MENU_ICONS.out, 16))}</span>Sign out</button>
    </div></div>`;
}

// Account menu behaviour: opens under the pill, arrow keys move between items, Escape / outside
// click / route change close it. Listeners are registered once and look the elements up each time,
// because the shell is mounted again after the sign-in pages.
function closeMenu(focusBtn = false) {
  const m = document.getElementById('acct-menu'), b = document.getElementById('acct-btn');
  if (!m || m.hidden) return;
  m.hidden = true; b.setAttribute('aria-expanded', 'false');
  if (focusBtn) b.focus();
}
function openMenu(focusFirst) {
  const m = document.getElementById('acct-menu'), b = document.getElementById('acct-btn'); if (!m) return;
  m.hidden = false; b.setAttribute('aria-expanded', 'true');
  if (focusFirst) m.querySelector('[role=menuitem]').focus();
}
document.addEventListener('click', (e) => {
  const b = e.target.closest?.('#acct-btn');
  if (b) { document.getElementById('acct-menu').hidden ? openMenu(false) : closeMenu(); return; }
  const item = e.target.closest?.('#acct-menu [role=menuitem]');
  if (item) { closeMenu(); if (item.id === 'sign-out') withBusy(item, signOut); return; }
  if (!e.target.closest?.('#acct-menu')) closeMenu();
});
document.addEventListener('keydown', (e) => {
  const m = document.getElementById('acct-menu'), b = document.getElementById('acct-btn'); if (!m || !b) return;
  if (m.hidden) { if (e.target === b && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) { e.preventDefault(); openMenu(true); } return; }
  const items = [...m.querySelectorAll('[role=menuitem]')], at = items.indexOf(document.activeElement);
  if (e.key === 'Escape') { e.preventDefault(); closeMenu(true); }
  else if (e.key === 'ArrowDown') { e.preventDefault(); items[(at + 1) % items.length].focus(); }
  else if (e.key === 'ArrowUp') { e.preventDefault(); items[(at - 1 + items.length) % items.length].focus(); }
  else if (e.key === 'Home') { e.preventDefault(); items[0].focus(); }
  else if (e.key === 'End') { e.preventDefault(); items[items.length - 1].focus(); }
  else if (e.key === 'Tab') closeMenu();
});
window.addEventListener('hashchange', () => closeMenu());

const sunIcon = '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>';
const moonIcon = '<path d="M21 12.8A9 9 0 1111.2 3a7 7 0 009.8 9.8z"/>';
function paintTheme() {
  const b = document.getElementById('theme-btn'); if (!b) return;
  const l = theme() === 'light';
  b.innerHTML = svg(l ? moonIcon : sunIcon, 18); b.setAttribute('aria-label', l ? 'Switch to dark theme' : 'Switch to light theme'); b.title = b.getAttribute('aria-label');
}

export function mountShell(app) {
  app.innerHTML = `
  <header class="topbar"><div class="wrap">
    ${logo({ href: '#/' })}
    <span class="crumb">${api.MODE === 'http' ? esc(hostName) : 'srv-sgn-01<s>/</s>shop-platform<s>/</s><b>production</b>'}</span>
    <span class="grow"></span>
    <button type="button" class="btn pal-btn" id="open-pal">Search or run a command<kbd class="kbd">Ctrl K</kbd></button>
    <button type="button" class="btn icon" id="theme-btn"></button>
    ${accountMenu()}
  </div></header>
  <div class="shell">
    <aside class="side" aria-label="Main"><nav id="nav" class="nav" aria-label="Sections"></nav>
      <div class="side-foot"><button type="button" class="side-toggle" id="side-toggle" aria-label="Collapse sidebar" title="Collapse sidebar">${svg('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 10l-2 2 2 2"/>')}<span class="lbl">Collapse</span></button></div></aside>
    <div class="content">
      <div id="view" style="flex:1;display:flex;flex-direction:column"></div>
      <footer class="statusbar" aria-label="Server status"><div class="wrap" id="status"></div></footer>
    </div>
  </div>`;
  paintTheme(); loadHostName();
  document.getElementById('theme-btn').addEventListener('click', () => { toggleTheme(); paintTheme(); });
  document.getElementById('open-pal').addEventListener('click', openPalette);
  const shell = app.querySelector('.shell'), tg = document.getElementById('side-toggle');
  const setC = (c) => { shell.classList.toggle('collapsed', c); const l = c ? 'Expand sidebar' : 'Collapse sidebar'; tg.setAttribute('aria-label', l); tg.title = l; try { localStorage.setItem('opsbox.side', c ? '1' : '0'); } catch {} };
  let c0 = false; try { c0 = localStorage.getItem('opsbox.side') === '1'; } catch {} setC(c0);
  tg.addEventListener('click', () => setC(!shell.classList.contains('collapsed')));
  return document.getElementById('view');
}

export function navMarkup(active) {
  const by = Object.fromEntries(NAV.map(([n, l, h, id]) => [id, [l, h]]));
  const link = (id) => `<a href="${by[id][1]}" title="${by[id][0]}"${id === active ? ' aria-current="page"' : ''}>${svg(ICONS[id])}<span class="lbl">${by[id][0]}</span></a>`;
  return GROUPS.map(([label, ids], i) => `<div class="nav-group"${label ? ` role="group" aria-label="${label}"` : ''}>${label ? `<div class="nav-label" aria-hidden="true">${label}</div>` : ''}${i ? '<div class="nav-sep" aria-hidden="true"></div>' : ''}${ids.map(link).join('')}</div>`).join('');
}
export function setNav(active) {
  const nav = document.getElementById('nav'); if (!nav) return;
  nav.innerHTML = navMarkup(active);
  const b = document.getElementById('acct-btn');
  if (b) b.classList.toggle('on', /^#\/(settings|security\/access)/.test(location.hash));
}

// Steady dot while updates arrive, amber "reconnecting" after a failed poll; the last
// successful update shows on hover. Updated in place so nothing in the status bar blinks.
const liveText = () => (liveState().ok ? 'live' : 'reconnecting…');
const liveTip = () => (liveState().at ? `Last updated ${clock(liveState().at)}` : 'Waiting for the first update');
function paintLive() {
  const el = document.getElementById('live-ind'); if (!el) return;
  el.classList.toggle('bad', !liveState().ok);
  el.dataset.tip = liveTip();
  const t = el.querySelector('.lt'); if (t && t.textContent !== liveText()) t.textContent = liveText();
}
onLive(paintLive);

export async function refreshStatus() {
  const el = document.getElementById('status'); if (!el) return;
  let m;
  try { m = await api.quiet(() => api.getMetrics()); } catch { reportPoll(false); return; }
  reportPoll(true);
  const d = m.disk / m.diskTotal * 100;
  const source = api.MODE === 'http' ? (api.isReal('getMetrics') ? '' : '<span>sample metrics</span>') : '<span>mock data</span>';
  patch(el, `<span class="${liveState().ok ? 'ok' : 'bad'}">■ ${esc(hostName)}</span><span>cpu <b>${num(m.cpuNow, 'pct')}</b></span><span>mem <b>${num(m.memUsed, 'dec1')}/${m.memTotal.toFixed(1)} GB</b></span><span>disk / <b class="${d > 75 ? 'w' : ''}">${num(d, 'pct')}</b></span><span>net <b>↓${num(m.netIn, 'dec1')} ↑${num(m.netOut, 'dec1')} MB/s</b></span><span class="grow"></span>${source}<span id="live-ind" class="live tip${liveState().ok ? '' : ' bad'}" data-tip="${esc(liveTip())}"><i class="ld"></i><span class="lt">${liveText()}</span></span><span>opsbox 0.1.0</span>`);
}

// ---------------- command palette ----------------
const ACTIONS = () => [
  ...NAV.map(([n, l, h]) => ({ label: `Go to ${l}`, hint: n, run: () => { location.hash = h; } })),
  { label: 'Go to Settings', hint: 'settings', run: () => { location.hash = '#/settings'; } },
  { label: 'API tokens', hint: 'settings', run: () => { location.hash = '#/settings?section=tokens'; } },
  { label: 'Domains and SSL for web', hint: 'apps', run: () => { location.hash = '#/apps?s=web&tab=domains'; } },
  { label: 'Add backup destination (S3, R2, B2…)', hint: 'backups', run: () => { location.hash = '#/backups/new'; } },
  { label: 'Add a database or template', hint: 'apps', run: () => { location.hash = '#/apps/new/templates'; } },
  { label: 'Users and SSH keys', hint: 'security', run: () => { location.hash = '#/security/access'; } },
  { label: 'New service from GitHub', hint: 'apps', run: () => { location.hash = '#/apps/new'; } },
  { label: 'Toggle light / dark theme', hint: 'theme', run: () => { toggleTheme(); paintTheme(); } },
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
