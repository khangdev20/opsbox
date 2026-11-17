// Router + boot. Hash routes: #/ #/apps #/apps/new #/apps/new/configure #/containers ...
import { applyTheme, initialTheme, session, listeners } from './store.js';
import * as api from './api.js';
import { mountShell, setNav, refreshStatus } from './shell.js';
import { toast } from './ui.js';

applyTheme(initialTheme());

const BARE = new Set(['login', 'setup', 'recovery']);
const ROUTES = [
  [/^\/?$/, 'overview'], [/^\/apps\/?$/, 'apps'], [/^\/apps\/new\/?$/, 'newservice'], [/^\/apps\/new\/configure\/?$/, 'configure'],
  [/^\/containers\/?$/, 'containers'], [/^\/system\/?$/, 'system'], [/^\/cleanup\/?$/, 'cleanup'], [/^\/terminal\/?$/, 'terminal'],
  [/^\/files\/?$/, 'files'], [/^\/backups\/?$/, 'backups'], [/^\/backups\/new\/?$/, 'backupdest'], [/^\/alerts\/?$/, 'alerts'], [/^\/apps\/new\/templates\/?$/, 'templates'], [/^\/security\/access\/?$/, 'access'], [/^\/settings\/?$/, 'settings'], [/^\/security\/?$/, 'security'], [/^\/login\/?$/, 'login'], [/^\/setup\/?$/, 'setup'], [/^\/recovery\/?$/, 'recovery'],
];
const app = document.getElementById('app');
let current = null, shellUp = false, gen = 0;

function parse() {
  const raw = location.hash.replace(/^#/, '') || '/'; const [path, qs] = raw.split('?');
  return { path, query: Object.fromEntries(new URLSearchParams(qs || '')) };
}

async function navigate() {
  const my = ++gen; const { path, query } = parse();
  const hit = ROUTES.find(([re]) => re.test(path)); const name = hit ? hit[1] : 'notfound';
  if (current && current.unmount) { try { current.unmount(); } catch {} }
  current = null;
  const bare = BARE.has(name);
  if (!bare && !session.user) { try { sessionStorage.setItem('opsbox.next', location.hash); } catch {} location.hash = '#/login'; return; }
  let root;
  if (bare) { app.innerHTML = '<div id="view" style="flex:1;display:flex;flex-direction:column"></div>'; root = document.getElementById('view'); shellUp = false; }
  else { if (!shellUp || !document.getElementById('nav')) { root = mountShell(app); shellUp = true; } else root = document.getElementById('view'); }
  let mod;
  try { mod = await import(`./pages/${name === 'notfound' ? 'overview' : name}.js`); } catch (e) { root.innerHTML = `<main><h1>Could not load page</h1><pre class="code">${String(e)}</pre></main>`; return; }
  if (my !== gen) return;
  try {
    const page = await mod.view({ query, path });
    if (my !== gen) return;
    if (!bare) setNav(page.nav);
    const host = document.createElement('div'); host.style.cssText = 'flex:1;display:flex;flex-direction:column'; host.innerHTML = page.html.toString();
    root.replaceChildren(host); document.title = `opsbox — ${page.title}`; window.scrollTo(0, 0);
    current = page; if (page.mount) page.mount(host);
  } catch (e) { console.error(e); root.innerHTML = `<main><h1>Something went wrong</h1><p class="muted">${String(e.message || e).replace(/</g, '&lt;')}</p></main>`; }
  if (!bare) refreshStatus();
}
window.addEventListener('hashchange', navigate);

// live metrics tick (drives status bar + pages that subscribe)
setInterval(() => { api.tick(); if (shellUp) refreshStatus(); listeners.forEach((f) => f()); }, 2000);
window.addEventListener('unhandledrejection', (e) => toast(e.reason?.message || 'Request failed', 'err'));
navigate();
