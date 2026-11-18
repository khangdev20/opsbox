// App shell: top bar, section nav, notification bell, status bar, command palette.
import { html, raw, esc, toast, num, patch, withBusy, reportPoll, liveState, onLive, clock, logo, initials, poll, ago, announce, reducedMotion } from './ui.js';
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
  closeNotifPop(false);
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

const paintBell = () => {
  const btn = document.getElementById('notif-btn');
  const badge = document.getElementById('notif-badge');
  const readAll = document.getElementById('notif-read-all');
  const list = document.getElementById('notif-list');
  const { items, unread, critical } = attnData;
  if (btn) btn.setAttribute('aria-label', `Notifications, ${unread} unread`);
  if (badge) {
    badge.hidden = !unread;
    badge.textContent = unread > 9 ? '9+' : String(unread);
    badge.classList.toggle('crit', critical > 0);
  }
  if (readAll) readAll.disabled = !unread;
  if (list) {
    const activeItem = document.activeElement?.closest?.('.notif-item');
    const activeKey = activeItem?.getAttribute('data-key');
    patch(list, renderNotifItems(items).toString());
    if (activeItem && !activeItem.isConnected && !document.getElementById('notif-pop')?.hidden) {
      const next = (activeKey && list.querySelector(`.notif-item[data-key="${CSS.escape(activeKey)}"]`)) || list.querySelector('.notif-item') || document.getElementById('notif-pop');
      next?.focus?.();
    }
  }
};

const SEV_ICONS = {
  critical: '<circle cx="12" cy="12" r="9"/><path d="M12 8v5M12 16h.01"/>',
  warning: '<path d="M10.3 3.9L1.8 18a2 2 0 001.7 3h17a2 2 0 001.7-3L13.7 3.9a2 2 0 00-3.4 0z"/><path d="M12 9v4M12 17h.01"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 16v-4M12 8h.01"/>',
};

function renderNotifItems(items = [], demo = false) {
  if (!items.length) return html`<div class="notif-empty muted">All clear</div>`;
  return items.map((it) => {
    const tsMs = it.since > 1e12 ? it.since : it.since * 1000;
    const href = it.link?.[1] || '#/';
    const linkLbl = it.link?.[0] || 'View →';
    return html`<div class="notif-item${it.read ? '' : ' unread'}${it.snoozed ? ' snoozed' : ''} sev-${it.severity || 'warning'}" data-key="n-${it.id}" data-id="${it.id}" data-sig="${it.severity}|${!!it.read}|${!!it.snoozed}|${it.since}|${it.text}|${href}" tabindex="-1">
      <span class="notif-ico" aria-hidden="true">${raw(svg(SEV_ICONS[it.severity] || SEV_ICONS.warning, 16))}</span>
      <div class="notif-body">
        <div class="notif-txt">${it.read ? '' : html`<span class="notif-dot" aria-label="Unread"></span>`}${it.text}</div>
        <div class="notif-meta sm muted"><span>${it.since ? ago(tsMs) : 'just now'}</span>${it.snoozed ? html`<span>· snoozed</span>` : ''}<a class="notif-go" href="${href}">${linkLbl}</a></div>
      </div>
      <div class="notif-acts">
        <button type="button" class="btn icon row-act notif-more" aria-haspopup="menu" aria-expanded="false" aria-label="Notification actions"${raw(demo ? '' : ` data-nact="menu" data-id="${esc(it.id)}"`)}>⋯</button>
        <div class="notif-menu" role="menu" hidden>
          ${!it.read ? html`<button type="button" role="menuitem" tabindex="-1" class="menu-item"${raw(demo ? '' : ` data-nact="read" data-id="${esc(it.id)}"`)}>Mark read</button>` : ''}
          <button type="button" role="menuitem" tabindex="-1" class="menu-item"${raw(demo ? '' : ` data-nact="snooze" data-min="60" data-id="${esc(it.id)}"`)}>Snooze 1 h</button>
          <button type="button" role="menuitem" tabindex="-1" class="menu-item"${raw(demo ? '' : ` data-nact="snooze" data-min="1440" data-id="${esc(it.id)}"`)}>Snooze 24 h</button>
        </div>
      </div>
    </div>`;
  });
}

// `demo` (UI kit): static preview states without ids so shell listeners ignore them.
export function notificationBell({ data = { items: [], unread: 0, critical: 0 }, demo = false, open = false } = {}) {
  const items = data?.items || [];
  const unread = data?.unread ?? 0;
  const critical = data?.critical ?? 0;
  const titleId = demo ? '' : 'notif-title';
  return html`<div class="notif"${raw(demo ? '' : ' id="notif"')}>
    <button type="button" class="btn icon notif-btn"${raw(demo ? '' : ' id="notif-btn" aria-controls="notif-pop"')} aria-haspopup="dialog" aria-expanded="${String(open)}" aria-label="Notifications, ${unread} unread">
      ${raw(svg(ICONS.alerts, 18))}
      <span class="notif-badge${critical > 0 ? ' crit' : ''}"${raw(demo ? '' : ' id="notif-badge"')}${raw(unread ? '' : ' hidden')} aria-hidden="true">${unread > 9 ? '9+' : unread}</span>
    </button>
    <div class="notif-pop"${raw(demo ? '' : ' id="notif-pop"')} role="dialog" ${raw(demo ? 'aria-label="Notifications"' : `aria-labelledby="${titleId}"`)} tabindex="-1"${raw(open ? '' : ' hidden')}>
      <div class="notif-head"><h2${raw(demo ? '' : ` id="${titleId}"`)}>Notifications</h2><button type="button" class="btn link"${raw(demo ? '' : ' id="notif-read-all"')}${raw(unread ? '' : ' disabled')}>Mark all read</button></div>
      <div class="notif-list"${raw(demo ? '' : ' id="notif-list"')} data-flip>${renderNotifItems(items, demo)}</div>
    </div>
  </div>`;
}

// Shared attention state & single poller across header bell + Overview.
const attnSubs = new Set();
let attnData = { items: [], unread: 0, critical: 0 };
let attnInflight = null;
let attnBooted = false;
let prevUnread = null;
let prevCritIds = new Set();
let stopAttnPoll = null;

function normalizeAttn(rawData) {
  if (Array.isArray(rawData)) {
    const items = rawData.map((x, i) => ({
      id: x.id || `${x.kind || 'item'}:${i}`,
      kind: x.kind || 'info',
      severity: x.severity || 'warning',
      since: x.since || Math.floor(Date.now() / 1000),
      text: x.text || '',
      link: x.link || ['View →', '#/'],
      read: !!x.read,
      snoozed: !!x.snoozed,
    }));
    return { items, unread: items.filter((i) => !i.read).length, critical: items.filter((i) => i.severity === 'critical' && !i.read).length };
  }
  const items = Array.isArray(rawData?.items) ? rawData.items : [];
  return {
    items,
    unread: rawData?.unread ?? items.filter((i) => !i.read).length,
    critical: rawData?.critical ?? items.filter((i) => i.severity === 'critical' && !i.read).length,
  };
}

function triggerBellMotion() {
  if (reducedMotion()) return;
  const btn = document.getElementById('notif-btn');
  const badge = document.getElementById('notif-badge');
  if (btn) {
    btn.classList.remove('ring');
    void btn.offsetWidth;
    btn.classList.add('ring');
    clearTimeout(btn._rt);
    btn._rt = setTimeout(() => btn.classList.remove('ring'), 660);
  }
  if (badge && !badge.hidden) {
    badge.classList.remove('pop');
    void badge.offsetWidth;
    badge.classList.add('pop');
    clearTimeout(badge._pt);
    badge._pt = setTimeout(() => badge.classList.remove('pop'), 380);
  }
}

function applyAttn(next) {
  const data = normalizeAttn(next);
  const unreadCrit = data.items.filter((i) => i.severity === 'critical' && !i.read);
  const nextCritIds = new Set(unreadCrit.map((i) => i.id));
  if (prevUnread !== null) {
    const newCrits = unreadCrit.filter((i) => !prevCritIds.has(i.id));
    if (data.unread > prevUnread || newCrits.length > 0) triggerBellMotion();
    if (newCrits.length > 0) {
      announce(newCrits.length === 1 ? `Critical alert: ${newCrits[0].text}` : `${newCrits.length} new critical alerts. ${newCrits[0].text}`);
    }
  }
  prevUnread = data.unread;
  prevCritIds = nextCritIds;
  attnData = data;
  attnBooted = true;
  paintBell();
  attnSubs.forEach((fn) => { try { fn(attnData); } catch {} });
}

export async function refreshAttention() {
  if (attnInflight) return attnInflight;
  attnInflight = api.quiet(() => api.getAttention())
    .then((d) => { applyAttn(d); return attnData; })
    .finally(() => { attnInflight = null; });
  return attnInflight;
}

export async function sharedAttention() {
  if (attnInflight) return attnInflight;
  if (attnBooted) return attnData;
  return refreshAttention();
}

export function onAttention(fn) {
  attnSubs.add(fn);
  return () => attnSubs.delete(fn);
}

function closeNotifMenus(within = document) {
  within.querySelectorAll('.notif-menu:not([hidden])').forEach((m) => { m.hidden = true; });
  within.querySelectorAll('.notif-more[aria-expanded=true]').forEach((b) => { b.setAttribute('aria-expanded', 'false'); });
}

function closeNotifPop(focusBtn = false) {
  const p = document.getElementById('notif-pop'), b = document.getElementById('notif-btn');
  if (!p || p.hidden) return;
  closeNotifMenus(p);
  p.hidden = true;
  b?.setAttribute('aria-expanded', 'false');
  if (focusBtn) b?.focus();
}

function openNotifPop() {
  const p = document.getElementById('notif-pop'), b = document.getElementById('notif-btn');
  if (!p) return;
  closeMenu();
  p.hidden = false;
  b?.setAttribute('aria-expanded', 'true');
  const first = p.querySelector('.notif-item') || (!document.getElementById('notif-read-all')?.disabled && document.getElementById('notif-read-all')) || p;
  first?.focus();
}

export function openNotifications() {
  const p = document.getElementById('notif-pop');
  if (!p) return;
  if (p.hidden) openNotifPop();
}

document.addEventListener('click', (e) => {
  const nb = e.target.closest?.('#notif-btn');
  if (nb) {
    const p = document.getElementById('notif-pop');
    if (p) (p.hidden ? openNotifPop() : closeNotifPop(true));
    return;
  }
  const pop = document.getElementById('notif-pop');
  if (!pop || pop.hidden) return;
  if (e.target.closest?.('#notif-read-all')) {
    const btn = e.target.closest('#notif-read-all');
    withBusy(btn, async () => applyAttn(await api.markAllAttentionRead()));
    return;
  }
  const nactEl = e.target.closest?.('[data-nact]');
  if (nactEl && pop.contains(nactEl)) {
    const act = nactEl.dataset.nact;
    const id = nactEl.dataset.id;
    if (act === 'menu') {
      const acts = nactEl.closest('.notif-acts');
      const menu = acts?.querySelector('.notif-menu');
      const wasHidden = menu?.hidden;
      closeNotifMenus(pop);
      if (menu && wasHidden) {
        menu.hidden = false;
        nactEl.setAttribute('aria-expanded', 'true');
        if (e.detail === 0) menu.querySelector('[role=menuitem]')?.focus();
      }
      return;
    }
    closeNotifMenus(pop);
    const runAction = async () => {
      try {
        const res = act === 'read'
          ? await api.markAttentionRead(id)
          : await api.snoozeAttention(id, Number(nactEl.dataset.min));
        applyAttn(res);
      } catch (err) {
        if (err.status === 404) await refreshAttention();
        else toast(err.message || 'Could not update notification.', 'err');
      }
    };
    withBusy(nactEl, runAction);
    return;
  }
  if (e.target.closest?.('.notif-go') && pop.contains(e.target)) {
    closeNotifPop(false);
    return;
  }
  if (!e.target.closest?.('.notif-acts')) closeNotifMenus(pop);
  if (!e.target.closest?.('#notif') && !e.target.closest?.('[data-act="open-bell"]')) closeNotifPop(false);
});

document.addEventListener('keydown', (e) => {
  const pop = document.getElementById('notif-pop');
  if (!pop || pop.hidden) return;
  const openMenuEl = pop.querySelector('.notif-menu:not([hidden])');
  if (openMenuEl) {
    const menuItems = [...openMenuEl.querySelectorAll('[role=menuitem]')];
    const idx = menuItems.indexOf(document.activeElement);
    if (e.key === 'Escape') {
      e.preventDefault();
      const more = openMenuEl.closest('.notif-acts')?.querySelector('.notif-more');
      closeNotifMenus(pop);
      more?.focus();
    } else if (e.key === 'ArrowDown' && menuItems.length) {
      e.preventDefault();
      menuItems[(idx + 1) % menuItems.length].focus();
    } else if (e.key === 'ArrowUp' && menuItems.length) {
      e.preventDefault();
      menuItems[(idx - 1 + menuItems.length) % menuItems.length].focus();
    } else if (e.key === 'Home' && menuItems.length) {
      e.preventDefault();
      menuItems[0].focus();
    } else if (e.key === 'End' && menuItems.length) {
      e.preventDefault();
      menuItems[menuItems.length - 1].focus();
    } else if (e.key === 'Tab') {
      closeNotifMenus(pop);
    }
    return;
  }
  if (e.key === 'Escape') { e.preventDefault(); closeNotifPop(true); return; }
  const items = [...pop.querySelectorAll('.notif-item')];
  if (!items.length) return;
  const curItem = document.activeElement?.closest?.('.notif-item');
  const at = curItem ? items.indexOf(curItem) : -1;
  if (e.key === 'ArrowDown') {
    e.preventDefault();
    items[(at + 1) % items.length].focus();
  } else if (e.key === 'ArrowUp') {
    e.preventDefault();
    items[at <= 0 ? items.length - 1 : at - 1].focus();
  } else if (e.key === 'Home') {
    e.preventDefault();
    items[0].focus();
  } else if (e.key === 'End') {
    e.preventDefault();
    items[items.length - 1].focus();
  } else if (e.key === 'Enter' && document.activeElement?.classList.contains('notif-item')) {
    const link = document.activeElement.querySelector('.notif-go');
    if (link) { e.preventDefault(); link.click(); }
  }
});
window.addEventListener('hashchange', () => closeNotifPop(false));

const sunIcon = '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>';
const moonIcon = '<path d="M21 12.8A9 9 0 1111.2 3a7 7 0 009.8 9.8z"/>';
function paintTheme() {
  const b = document.getElementById('theme-btn'); if (!b) return;
  const l = theme() === 'light';
  b.innerHTML = svg(l ? moonIcon : sunIcon, 18); b.setAttribute('aria-label', l ? 'Switch to dark theme' : 'Switch to light theme'); b.title = b.getAttribute('aria-label');
}

// ---------------- helper banner ----------------
// Shown on every page, not only as a toast, while the privileged helper is down or is a different
// build from the panel: pages that read keep working, actions that change the server do not.
export const HELPER_RESTART = 'sudo systemctl restart opsbox-priv';
export function helperBannerHtml(h) {
  if (!h || h.state === 'ok') return '';
  const text = h.state === 'mismatch'
    ? `OpsBox's helper is running a different version (${h.helper || 'unknown'}) than the panel (${h.serve || 'unknown'}). Actions that change the server are paused.`
    : "OpsBox's helper is not running. Actions that change the server are paused.";
  return html`<div class="crit-banner" role="alert" aria-label="Helper status">
    <span class="crit-ico" aria-hidden="true">${raw(svg('<circle cx="12" cy="12" r="9"/><path d="M12 8v5M12 16h.01"/>', 16))}</span>
    <span class="crit-txt">${text} Restart it with <span class="mono">${HELPER_RESTART}</span></span>
    <button type="button" class="btn" data-act="copy-helper-cmd">Copy</button>
  </div>`.toString();
}
let helperState = 'ok||', stopHelperPoll = null;
export async function refreshHelperBanner() {
  const host = document.getElementById('helper-banner'); if (!host) return;
  let h;
  try { h = await api.quiet(() => api.getHelperHealth()); } catch { return; }
  // Repaint only when the answer changed, so a focused Copy button is not rebuilt every poll.
  const key = `${h.state}|${h.serve || ''}|${h.helper || ''}`;
  if (key === helperState) return;
  helperState = key;
  host.innerHTML = helperBannerHtml(h);
}

export function unmountShell() {
  if (stopAttnPoll) { stopAttnPoll(); stopAttnPoll = null; }
  if (stopHelperPoll) { stopHelperPoll(); stopHelperPoll = null; }
  helperState = 'ok||';
  attnData = { items: [], unread: 0, critical: 0 };
  attnInflight = null;
  attnBooted = false;
  prevUnread = null;
  prevCritIds = new Set();
}

export function mountShell(app) {
  app.innerHTML = `
  <header class="topbar"><div class="wrap">
    ${logo({ href: '#/' })}
    <span class="crumb">${api.MODE === 'http' ? esc(hostName) : 'srv-sgn-01<s>/</s>shop-platform<s>/</s><b>production</b>'}</span>
    <span class="grow"></span>
    <button type="button" class="btn pal-btn" id="open-pal">Search or run a command<kbd class="kbd">Ctrl K</kbd></button>
    ${notificationBell({ data: attnData })}
    <button type="button" class="btn icon" id="theme-btn"></button>
    ${accountMenu()}
  </div></header>
  <div class="shell">
    <aside class="side" aria-label="Main"><nav id="nav" class="nav" aria-label="Sections"></nav>
      <div class="side-foot"><button type="button" class="side-toggle" id="side-toggle" aria-label="Collapse sidebar" title="Collapse sidebar">${svg('<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 10l-2 2 2 2"/>')}<span class="lbl">Collapse</span></button></div></aside>
    <div class="content">
      <div id="helper-banner" class="helper-banner"></div>
      <div id="view" style="flex:1;display:flex;flex-direction:column"></div>
      <footer class="statusbar" aria-label="Server status"><div class="wrap" id="status"></div></footer>
    </div>
  </div>`;
  paintTheme(); paintBell(); loadHostName();
  if (stopAttnPoll) stopAttnPoll();
  stopAttnPoll = poll(refreshAttention, 15000, { immediate: true });
  if (stopHelperPoll) stopHelperPoll();
  helperState = 'ok||';
  stopHelperPoll = poll(refreshHelperBanner, 10000, { immediate: true });
  document.getElementById('helper-banner').addEventListener('click', async (e) => {
    if (!e.target.closest('[data-act="copy-helper-cmd"]')) return;
    try { await navigator.clipboard.writeText(HELPER_RESTART); toast('Copied.'); } catch { toast(`Select and copy: ${HELPER_RESTART}`); }
  });
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
  { label: 'Capacity', hint: 'system', run: () => { location.hash = '#/system/capacity'; } },
  { label: 'Add swap', hint: 'system', run: () => { location.hash = '#/system/capacity'; } },
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
