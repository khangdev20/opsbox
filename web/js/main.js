// Router + boot. Hash routes: #/ #/apps #/apps/new #/apps/new/configure #/containers ...
import { applyTheme, initialTheme, session, server, listeners } from './store.js';
import * as api from './api.js';
import { mountShell, setNav, refreshStatus } from './shell.js';
import { toast, confirmDialog, formDialog, keepOrUndoDialog, initUi, reportPoll, liveState, skeletonPage } from './ui.js';

applyTheme(initialTheme());
initUi();

// Questions the API layer needs a person to answer (see api.setUiHooks).
api.setUiHooks({
  confirm: (message, title) => confirmDialog({ title: title || 'This could lock you out', body: message, confirm: 'Continue', danger: true }),
  prompt: async ({ title, body, label, placeholder }) => { const r = await formDialog({ title, body, fields: [{ name: 'v', label, placeholder }], submit: 'Continue' }); return r && r.v ? r.v.trim() : null; },
  revert: (at, keep) => keepOrUndoDialog({ title: 'Keep this firewall change?', body: 'Check that you can still reach this machine (open a second SSH session or reload this page). If you do nothing, the change is undone.', until: at, keep }),
});

const BARE = new Set(['login', 'setup', 'enroll', 'invite', 'recovery']);
const ROUTES = [
  [/^\/?$/, 'overview'], [/^\/apps\/?$/, 'apps'], [/^\/apps\/new\/?$/, 'newservice'], [/^\/apps\/new\/configure\/?$/, 'configure'],
  [/^\/containers\/?$/, 'containers'], [/^\/system\/?$/, 'system'], [/^\/cleanup\/?$/, 'cleanup'], [/^\/terminal\/?$/, 'terminal'],
  [/^\/files\/?$/, 'files'], [/^\/backups\/?$/, 'backups'], [/^\/backups\/new\/?$/, 'backupdest'], [/^\/alerts\/?$/, 'alerts'], [/^\/apps\/new\/templates\/?$/, 'templates'], [/^\/databases\/[^/]+\/?$/, 'database'], [/^\/security\/access\/?$/, 'access'], [/^\/settings\/?$/, 'settings'], [/^\/security\/?$/, 'security'], [/^\/login\/?$/, 'login'], [/^\/setup\/?$/, 'setup'], [/^\/enroll\/?$/, 'enroll'], [/^\/invite\/?$/, 'invite'], [/^\/recovery\/?$/, 'recovery'],
];
const app = document.getElementById('app');
let current = null, shellUp = false, gen = 0;

function parse() {
  const raw = location.hash.replace(/^#/, '') || '/'; const [path, qs] = raw.split('?');
  return { path, query: Object.fromEntries(new URLSearchParams(qs || '')) };
}

async function navigate() {
  const my = ++gen; const { path, query } = parse(); api.resetUsage();
  const hit = ROUTES.find(([re]) => re.test(path)); const name = hit ? hit[1] : 'notfound';
  if (current && current.unmount) { try { current.unmount(); } catch {} }
  current = null;
  const bare = BARE.has(name);
  if (bare) endStream();
  if (!bare && !session.user) { try { sessionStorage.setItem('opsbox.next', location.hash); } catch {} location.hash = server.setupRequired ? '#/setup' : '#/login'; return; }
  // Real server: setup exists only until the first admin is enrolled, and a signed-in
  // person has no use for the sign-in page.
  if (api.MODE === 'http') {
    if (name === 'setup' && !server.setupRequired) { location.hash = session.user ? '#/' : '#/login'; return; }
    if (name === 'login' && session.user) { location.hash = '#/'; return; }
    if (name === 'login' && server.setupRequired) { location.hash = '#/setup'; return; }
  }
  let root, sk = 0;
  if (bare) { app.innerHTML = '<div id="view" style="flex:1;display:flex;flex-direction:column"></div>'; root = document.getElementById('view'); shellUp = false; }
  else { if (!shellUp || !document.getElementById('nav')) { root = mountShell(app); shellUp = true; } else root = document.getElementById('view'); }
  // A first load never leaves the page blank: after a moment, show blocks shaped like the page.
  if (!bare) sk = setTimeout(() => { if (my !== gen) return; const h = document.createElement('div'); h.style.cssText = 'flex:1;display:flex;flex-direction:column'; h.innerHTML = skeletonPage().toString(); root.replaceChildren(h); }, 140);
  let mod;
  try { mod = await import(`./pages/${name === 'notfound' ? 'overview' : name}.js`); } catch (e) { clearTimeout(sk); root.innerHTML = `<main><h1>Could not load page</h1><pre class="code">${String(e)}</pre></main>`; return; }
  if (my !== gen) { clearTimeout(sk); return; }
  try {
    const page = await mod.view({ query, path });
    clearTimeout(sk);
    if (my !== gen) return;
    if (!bare) setNav(page.nav);
    const host = document.createElement('div'); host.style.cssText = 'flex:1;display:flex;flex-direction:column'; host.innerHTML = page.html.toString(); if (!bare) host.classList.add('page-in');
    root.replaceChildren(host); document.title = `opsbox — ${page.title}`; window.scrollTo(0, 0);
    current = page; if (page.mount) page.mount(host);
    markSampleData(host, name);
  } catch (e) { clearTimeout(sk); console.error(e); root.innerHTML = `<main><h1>Something went wrong</h1><p class="muted">${String(e.message || e).replace(/</g, '&lt;')}</p></main>`; }
  if (!bare) { ensureStream(); refreshStatus(); }
}
window.addEventListener('hashchange', navigate);

// In http mode, say so on every page that still shows mock data. Which functions a
// page used is recorded by api.js, so the badge goes away on its own once the last
// mock call on that page is wired to the real API.
function markSampleData(host, page) {
  if (api.MODE !== 'http') return;
  const { sample, real } = api.usageFor(page);
  if (!sample.length) return;
  const badge = document.createElement('span');
  badge.className = 'tag-sm sample'; badge.dataset.sample = sample.join(',');
  badge.textContent = real.length ? 'partly sample data' : 'sample data';
  badge.title = `Not connected to this server yet: ${sample.join(', ')}`;
  const row = host.querySelector('.hdr .row');
  if (row) row.prepend(badge); else host.prepend(badge);
}

// Live metrics drive the status bar and the pages that subscribe. Mock mode ticks every
// 2 s; http mode is pushed by the server over a WebSocket, only while signed in. Nothing
// ticks while the tab is hidden; coming back refreshes once straight away.
const refreshLive = () => { if (document.hidden) return; if (shellUp) refreshStatus(); api.quiet(() => listeners.forEach((f) => { try { Promise.resolve(f()).catch(() => {}); } catch { /* a page's poll failed; the status bar shows it */ } })); };
let stopStream = null;
const ensureStream = () => { if (api.MODE === 'http' && !stopStream) stopStream = api.startMetricsStream(refreshLive); };
const endStream = () => { if (stopStream) { stopStream(); stopStream = null; } };
if (api.MODE !== 'http') setInterval(() => { if (document.hidden) return; api.tick(); refreshLive(); }, 2000);
document.addEventListener('visibilitychange', () => { if (!document.hidden) refreshLive(); });
// "reconnecting": the metrics stream dropped, or no frame has arrived for a while.
addEventListener('opsbox:stream', (e) => { if (e.detail && e.detail.up === false) reportPoll(false); });
setInterval(() => { if (!document.hidden && stopStream && Date.now() - liveState().at > 6000) reportPoll(false); }, 3000);
window.addEventListener('unhandledrejection', (e) => toast(e.reason?.message || 'Request failed', 'err'));

// The server decides who is signed in. Ask it once at start-up (http mode) so the first
// screen is right: sign-in, first-run setup, or the app.
async function boot() {
  if (api.MODE !== 'http' && new URLSearchParams(location.search).has('kit')) { location.replace('kit.html'); return; }
  if (api.MODE === 'http') {
    try {
      const s = await api.auth.session();
      server.setupRequired = !!s.setupRequired; server.role = s.role || '';
      if (s.authenticated) session.set(s.user); else session.clear();
      if (s.enrollRequired && !/^#\/enroll/.test(location.hash)) location.hash = '#/enroll';
      // `?kit` opens the UI kit for an Owner (the server refuses anyone else).
      if (s.authenticated && s.role === 'admin' && new URLSearchParams(location.search).has('kit')) { location.replace('/kit.html'); return; }
    } catch {
      session.clear();
      try {
        const st = await api.auth.setupStatus();
        server.setupRequired = !!st.setupRequired;
      } catch { /* signed-in or offline */ }
    }
  }
  navigate();
}
// A 401 from any call means the session ended: back to the sign-in page.
window.addEventListener('opsbox:unauthorized', () => {
  if (api.MODE !== 'http' || !session.user) return;
  session.clear(); toast('Your session ended. Sign in again.', 'err'); location.hash = '#/login';
});
boot();
