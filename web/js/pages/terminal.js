import { html, esc, btn, bindActions, confirmDialog, toast } from '../ui.js';
import * as api from '../api.js';
import { server } from '../store.js';

export async function view() { return api.MODE === 'http' ? realView() : mockView(); }

// xterm.js (MIT, self-hosted under /vendor/xterm) is loaded when this page is first opened.
let xtermP;
function loadXterm() {
  if (window.Terminal && window.FitAddon) return Promise.resolve();
  if (xtermP) return xtermP;
  const base = new URL('../../vendor/xterm/', import.meta.url).href;
  const link = document.createElement('link'); link.rel = 'stylesheet'; link.href = base + 'xterm.css'; document.head.append(link);
  const script = (src) => new Promise((resolve, reject) => {
    const el = document.createElement('script'); el.src = base + src; el.onload = resolve;
    el.onerror = () => reject(new Error('Could not load the terminal.')); document.head.append(el);
  });
  xtermP = script('xterm.js').then(() => script('addon-fit.js')).catch((e) => { xtermP = null; throw e; });
  return xtermP;
}

// The real terminal: a login shell on this server. Other servers (SSH from the panel) need
// the desktop app, so those tabs are present but disabled.
async function realView() {
  const isAdmin = server.role === 'admin';
  let users = null, loadError = '', hostName = 'this server';
  try { hostName = (await api.getHost()).hostname || hostName; } catch { /* keep the generic name */ }
  if (isAdmin) { try { users = await api.getTerminalUsers(); } catch (e) { loadError = e.message; } }
  let root, term = null, fit = null, conn = null, ro = null, state = 'idle';
  const status = (text, kind = '') => { const el = root?.querySelector('#stat'); if (el) { el.textContent = text; el.className = `sm ${kind}`.trim(); } };
  const controls = () => {
    const live = state === 'connecting' || state === 'open';
    root?.querySelector('[data-act=connect]')?.toggleAttribute('hidden', live);
    root?.querySelector('[data-act=disconnect]')?.toggleAttribute('hidden', !live);
    const pick = root?.querySelector('#u'); if (pick) pick.disabled = live;
  };
  const theme = () => { const cs = getComputedStyle(document.documentElement); return { background: cs.getPropertyValue('--code').trim() || '#111113', foreground: cs.getPropertyValue('--codetext').trim() || '#DADADF', cursor: cs.getPropertyValue('--accent').trim() || '#FF6A3D' }; };
  const size = () => { try { fit.fit(); } catch { /* not laid out yet */ } return { cols: term.cols || 80, rows: term.rows || 24 }; };

  const start = async (user, confirmRoot) => {
    state = 'connecting'; controls(); status('Connecting…');
    try { await loadXterm(); } catch (e) { state = 'idle'; controls(); status(e.message, 'st fail'); throw e; }
    let dims;
    try {
      if (!term) {
        term = new window.Terminal({ cursorBlink: true, fontFamily: 'Geist Mono, ui-monospace, monospace', fontSize: 14, scrollback: 5000, theme: theme(), allowProposedApi: false });
        fit = new window.FitAddon.FitAddon(); term.loadAddon(fit);
        term.open(root.querySelector('#xt'));
        term.onData((d) => conn?.send(d));
        term.onBinary((d) => conn?.sendBytes(Uint8Array.from(d, (c) => c.charCodeAt(0))));
        term.onResize(({ cols, rows }) => conn?.resize(cols, rows));
        // Keyboard users must be able to leave the terminal: Tab belongs to the shell.
        term.attachCustomKeyEventHandler((e) => { if (e.type === 'keydown' && e.key === 'Escape' && e.shiftKey) { root.querySelector('#u')?.focus(); return false; } return true; });
        if (typeof ResizeObserver !== 'undefined') { let raf; ro = new ResizeObserver(() => { cancelAnimationFrame(raf); raf = requestAnimationFrame(() => { try { fit.fit(); } catch { /* hidden */ } }); }); ro.observe(root.querySelector('#xt')); }
      } else term.reset();
      dims = size();
    } catch (e) {
      state = 'idle'; controls(); status('The terminal could not be drawn in this browser.', 'st fail'); throw e;
    }
    conn = api.openTerminal({ user, cols: dims.cols, rows: dims.rows, confirmRoot }, {
      ready: (m) => { state = 'open'; controls(); status(`Connected as ${m.user}`, 'st ok'); term.focus(); },
      data: (bytes) => term.write(bytes),
      exit: (code) => { term.write(`\r\n\x1b[2m[session ended${code === null || code === undefined ? '' : ` with exit code ${code}`}]\x1b[0m\r\n`); },
      error: (msg) => status(msg, 'st fail'),
      close: ({ ready, ended, why }) => {
        conn = null; state = 'idle'; controls();
        if (!ready) { status(why || 'The terminal could not be opened.', 'st fail'); toast(why || 'The terminal could not be opened.', 'err'); }
        else status(ended ? 'Session ended. Start a new one to continue.' : 'Disconnected. Start a new session to continue.', 'muted');
      },
    });
  };
  const connectAs = async (name) => {
    const u = users.users.find((x) => x.name === name); if (!u) return;
    if (u.root && !(await confirmDialog({ title: 'Open a terminal as root?', body: 'Root can change or delete anything on this server. Everything you do here is tied to your account in the audit log.', confirm: 'Open as root', danger: true }))) return;
    await start(u.name, u.root);
  };

  const tabs = html`<div id="hosts" class="hosts" role="tablist" aria-label="Servers">
      <button type="button" class="host on" role="tab" aria-selected="true"><b>${hostName}</b><span class="sm muted">this server</span></button>
      <button type="button" class="host" role="tab" aria-selected="false" disabled title="Connecting to other servers over SSH needs the desktop app"><b>Other servers</b><span class="sm muted">needs the desktop app</span></button></div>`;
  let body;
  if (!isAdmin) body = html`<div class="callout"><span class="tag">▲</span><span>Only administrators can open a terminal.</span></div>`;
  else if (loadError) body = html`<div class="callout fail"><span class="tag">▲</span><span>${loadError}</span></div>`;
  else body = html`<div class="row" style="margin-bottom:12px;gap:12px;flex-wrap:wrap;align-items:center">
        <label for="u" class="sr">Open as user</label>
        <select class="select" id="u">${users.users.map((u) => html`<option value="${u.name}"${u.name === (users.default || '') ? ' selected' : ''}>${u.name}${u.root ? ' (root)' : ''}</option>`)}</select>
        ${btn('New session', { act: 'connect', kind: 'primary' })}${btn('Disconnect', { act: 'disconnect' })}
        <span id="stat" class="sm muted" role="status" aria-live="polite"></span></div>
      ${users.default ? '' : html`<p class="muted sm">This server has no ordinary user account to open a terminal as. Choose one, or root.</p>`}
      <div class="term"><div id="xt" class="xterm-host" aria-label="Terminal"></div>
        <div class="term-foot"><span>Ctrl+D or <span class="mono">exit</span> ends the session</span><span>Shift+Esc leaves the terminal</span><span>Idle sessions close on their own</span></div></div>`;
  return {
    nav: 'terminal', title: 'Terminal',
    html: html`<main><div class="hdr"><div><h1>Terminal</h1><div class="meta">A login shell on ${hostName}, as the Linux user you choose. Starts and ends are recorded in the audit log; what you type is not.</div></div></div>
      <div class="split"><div style="flex:0 0 240px">${tabs}</div><div style="flex:1 1 480px;min-width:0">${body}</div></div></main>`,
    mount(r) {
      root = r;
      if (!users) return;
      bindActions(root, {
        connect: () => connectAs(root.querySelector('#u').value),
        disconnect: () => { conn?.close(); },
      });
      controls();
      if (users.default) connectAs(users.default).catch((e) => toast(e.message, 'err'));
    },
    unmount() { try { conn?.close(); ro?.disconnect(); term?.dispose(); } catch { /* leaving anyway */ } conn = null; term = null; },
  };
}

async function mockView() {
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
