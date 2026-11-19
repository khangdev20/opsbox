// Small UI toolkit: safe html templating, formatters, motion + live-data helpers, controls, toasts, dialogs.
// All interpolated values are escaped unless wrapped in raw().

const ESC = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
export const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ESC[c]);

class Raw { constructor(s) { this.s = s; } toString() { return this.s; } }
export const raw = (s) => new Raw(s);

function render(v) {
  if (v instanceof Raw) return v.s;
  if (Array.isArray(v)) return v.map(render).join('');
  if (v === false || v === null || v === undefined) return '';
  return esc(v);
}
// html`<b>${userText}</b>` -> Raw. Nested html`` results are Raw, so they pass through.
export function html(strings, ...vals) {
  let out = strings[0];
  vals.forEach((v, i) => { out += render(v) + strings[i + 1]; });
  return new Raw(out);
}

// ---------- formatters ----------
export const pct = (n) => `${Math.round(n)}%`;
export function bytes(n, d = 1) {
  const u = ['B', 'KB', 'MB', 'GB', 'TB']; let i = 0;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return `${i === 0 ? n : n.toFixed(d)} ${u[i]}`;
}
export const gb = (n, d = 1) => `${n.toFixed(d)} GB`;
export function ago(ts) {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}
export const clock = (ts = Date.now()) => new Date(ts).toTimeString().slice(0, 8);


// ---------- motion helpers ----------
// Durations mirror the --dur-* tokens in app.css (JS tweens cannot read a CSS var cheaply).
export const DUR = { 1: 120, 2: 200, 3: 320 };
const hasDom = () => typeof document !== 'undefined' && typeof document.createElement === 'function';
export const reducedMotion = () => { try { return !!matchMedia('(prefers-reduced-motion: reduce)').matches; } catch { return false; } };
const tabHidden = () => hasDom() && document.hidden === true;
const animOk = () => !reducedMotion() && !tabHidden();

// Named number formats, so markup can carry `data-fmt` and the morph can re-tween it.
export const FORMATS = {
  int: (n) => String(Math.round(n)),
  pct: (n) => `${Math.round(n)}%`,
  pct1: (n) => `${n.toFixed(1)}%`,
  dec1: (n) => n.toFixed(1),
  dec2: (n) => n.toFixed(2),
  mb: (n) => `${Math.round(n)} MB`,
  gb0: (n) => `${Math.round(n)} GB`,
  gb1: (n) => `${n.toFixed(1)} GB`,
  bytes: (n) => bytes(n),
};
// A number that tweens when the markup around it is patched. `fmt` is a FORMATS key.
export function num(value, fmt = 'int') {
  const v = Number(value);
  if (!Number.isFinite(v)) return html`<span class="num">–</span>`;
  return html`<span class="num" data-tween="${v}" data-fmt="${fmt}">${(FORMATS[fmt] || FORMATS.int)(v)}</span>`;
}

const outCubic = (t) => 1 - (1 - t) ** 3;
// Animate the number shown in `el` to `to`. Small changes (< 0.5%), a hidden tab and
// reduced motion jump straight to the final value. The first call never animates.
export function tweenNumber(el, to, { format = FORMATS.int, dur = DUR[3] } = {}) {
  if (!el) return;
  el.classList.add('num');
  if (el._raf) { cancelAnimationFrame(el._raf); el._raf = 0; }
  const from = el._tv;
  el._tv = to;
  const set = (v) => { el.textContent = format(v); };
  if (from === undefined || !Number.isFinite(from) || !animOk()) { set(to); return; }
  const base = Math.max(Math.abs(from), Math.abs(to));
  if (base === 0 || Math.abs(to - from) / base < 0.005) { set(to); return; }
  const t0 = performance.now();
  const step = (now) => {
    const k = Math.min(1, (now - t0) / dur);
    if (k >= 1) { el._raf = 0; set(to); return; }
    set(from + (to - from) * outCubic(k));
    el._raf = requestAnimationFrame(step);
  };
  el._raf = requestAnimationFrame(step);
}

// Meter/usage bar: `el` is the inner <i>. Moves with scaleX; the level class cross-fades colour.
export function setBar(el, ratio, { thresholds = true } = {}) {
  if (!el) return;
  const r = Math.min(1, Math.max(0, Number.isFinite(ratio) ? ratio : 0));
  el.style.transform = `scaleX(${r.toFixed(4)})`;
  el.dataset.bar = String(r);
  const lvl = !thresholds ? '' : r >= 0.9 ? 'lvl-danger' : r >= 0.75 ? 'lvl-warn' : '';
  el.classList.remove('lvl-warn', 'lvl-danger');
  if (lvl) el.classList.add(lvl);
}
export function bar(ratio, label, { thresholds = true } = {}) {
  const r = Math.min(1, Math.max(0, Number.isFinite(ratio) ? ratio : 0));
  const lvl = !thresholds ? '' : r >= 0.9 ? ' lvl-danger' : r >= 0.75 ? ' lvl-warn' : '';
  return html`<div class="bar" role="img" aria-label="${label} ${Math.round(r * 100)} percent"><i class="${lvl.trim()}" data-bar="${r.toFixed(4)}"${raw(thresholds ? '' : ' data-th="off"')} style="transform:scaleX(${r.toFixed(4)})"></i></div>`;
}
// Segmented usage bar, animated with transforms: parts = [[fraction, colour, label?]...].
export function segBar(parts, label) {
  let off = 0;
  const total = parts.reduce((a, p) => a + Math.max(0, p[0]), 0) || 1;
  return html`<div class="seg" role="img" aria-label="${label}">${parts.map(([f, c], i) => {
    const s = Math.max(0, f) / total; const o = off; off += s;
    return html`<i data-key="s${i}" style="--o:${o.toFixed(4)};--s:${Math.max(0, s - 0.004).toFixed(4)};background:${c}"></i>`;
  })}</div>`;
}

// Short background highlight on a value or row that changed (never call on first render).
export function flash(el, kind = '') {
  if (!el || reducedMotion()) return;
  el.classList.remove('flash', 'flash-ok', 'flash-warn', 'flash-err');
  void el.offsetWidth;
  el.classList.add('flash');
  if (kind) el.classList.add(`flash-${kind}`);
  clearTimeout(el._ft);
  el._ft = setTimeout(() => el.classList.remove('flash', 'flash-ok', 'flash-warn', 'flash-err'), 950);
}

// ---------- keyed DOM patching ----------
// patch(host, htmlString): update `host` in place so scroll, focus, typed input and running
// animations survive. Children with data-key are matched by key; new ones fade in, missing ones
// fade out, changed ones flash. data-tween / data-bar elements animate instead of jumping.
// A child with data-keep is left to whoever owns it (charts). Pass { animate: false } for a
// different logical view (a tab switch); the first patch of a host never animates.
const TRANSIENT = ['enter', 'flash', 'flash-ok', 'flash-warn', 'flash-err', 'busy', 'pending', 'leaving', 'swap'];
const keyOf = (n) => (n.nodeType === 1 ? n.getAttribute('data-key') : null);
const isLeaving = (n) => n.nodeType === 1 && n.classList.contains('leaving');
const FORM = new Set(['INPUT', 'TEXTAREA', 'SELECT']);
// What the server-rendered markup says the value is. A control's own value is only overwritten
// when this changes, so typing and choices survive a refresh (disabled, class etc. do not count).
const vsig = (x) => (x.tagName === 'SELECT' ? [...x.options].map((o) => `${o.value}${o.hasAttribute('selected') ? '*' : ''}`).join('|')
  : x.tagName === 'TEXTAREA' ? x.textContent
    : (x.type === 'checkbox' || x.type === 'radio') ? String(x.hasAttribute('checked')) : (x.getAttribute('value') ?? ''));

function syncAttrs(a, b) {
  const busy = a.hasAttribute('data-busy');
  for (const { name, value } of [...b.attributes]) {
    if (busy && (name === 'style' || name === 'disabled' || name === 'aria-busy')) continue;
    if (name === 'class') {
      const keep = TRANSIENT.filter((c) => a.classList.contains(c));
      const want = [...new Set([...value.split(/\s+/).filter(Boolean), ...keep])].join(' ');
      if (a.getAttribute('class') !== want) a.setAttribute('class', want);
    } else if (a.getAttribute(name) !== value) a.setAttribute(name, value);
  }
  for (const { name } of [...a.attributes]) {
    if (b.hasAttribute(name)) continue;
    if (name === 'data-busy' || (busy && (name === 'style' || name === 'disabled' || name === 'aria-busy'))) continue;
    if (name === 'class') {
      const keep = TRANSIENT.filter((c) => a.classList.contains(c)).join(' ');
      if (keep) a.setAttribute('class', keep); else a.removeAttribute('class');
      continue;
    }
    a.removeAttribute(name);
  }
}

function enter(node) {
  if (reducedMotion() || node.nodeType !== 1 || !node.hasAttribute('data-key')) return;
  node.classList.add('enter');
  setTimeout(() => node.classList.remove('enter'), DUR[3] + 60);
}
function leave(node) {
  if (node.nodeType !== 1 || !node.hasAttribute('data-key')) { node.remove(); return; }
  node.classList.add('leaving');
  node.setAttribute('aria-hidden', 'true');
  node.querySelectorAll('button,a,input,select,textarea').forEach((x) => { x.tabIndex = -1; });
  setTimeout(() => node.remove(), DUR[2] + 40);
}

function morphNode(a, b, ctx) {
  if (a.nodeType !== 1) { if (a.nodeValue !== b.nodeValue) a.nodeValue = b.nodeValue; return; }
  if (b.hasAttribute('data-keep') && a.hasAttribute('data-keep')) return;
  if (a.classList.contains('ind')) return;
  const keyed = a.hasAttribute('data-key');
  const sig = keyed ? (b.getAttribute('data-sig') ?? b.outerHTML) : null;

  if (b.hasAttribute('data-bar')) {
    for (const { name, value } of [...b.attributes]) if (name !== 'style' && name !== 'class') a.setAttribute(name, value);
    setBar(a, parseFloat(b.getAttribute('data-bar')), { thresholds: b.getAttribute('data-th') !== 'off' });
    if (b.classList.contains('lvl-danger') && b.getAttribute('data-th') === 'off') a.classList.add('lvl-danger'); else if (b.getAttribute('data-th') === 'off') a.classList.remove('lvl-danger');
    return;
  }
  if (b.hasAttribute('data-tween') && a._tv === undefined && a.hasAttribute('data-tween')) a._tv = parseFloat(a.getAttribute('data-tween'));
  syncAttrs(a, b);
  if (b.hasAttribute('data-tween')) {
    tweenNumber(a, parseFloat(b.getAttribute('data-tween')), { format: FORMATS[b.getAttribute('data-fmt')] || FORMATS.int });
    return;
  }
  if (FORM.has(a.tagName)) {
    const fsig = vsig(b);
    if (a.tagName === 'SELECT') morphChildren(a, b, ctx);
    if (a._sig !== fsig) {
      if (a !== document.activeElement || a.type === 'checkbox' || a.type === 'radio') {
        if (a.type === 'checkbox' || a.type === 'radio') a.checked = b.hasAttribute('checked');
        else if (a.tagName === 'SELECT') { const i = [...b.options].findIndex((o) => o.hasAttribute('selected')); a.selectedIndex = i >= 0 ? i : 0; }
        else a.value = a.tagName === 'TEXTAREA' ? b.textContent : (b.getAttribute('value') ?? '');
      }
      a._sig = fsig;
    }
    return;
  }
  const before = a.classList.contains('st') ? a.textContent : null;
  morphChildren(a, b, ctx);
  if (before !== null && before !== a.textContent && ctx.animate) { a.classList.remove('swap'); void a.offsetWidth; a.classList.add('swap'); setTimeout(() => a.classList.remove('swap'), DUR[2] + 40); }
  if (keyed) {
    if (ctx.animate && a._rsig !== undefined && a._rsig !== sig) flash(a);
    a._rsig = sig;
  }
}

function morphChildren(a, b, ctx) {
  const olds = [...a.childNodes].filter((n) => !isLeaving(n));
  // data-flip: ranked / fixed-size lists. Leaving rows are removed at once (no fade-in-place
  // height bump). Movers glide with FLIP unless reduced-motion is on or the move is further
  // than the visible list height (then they are just placed).
  const flip = ctx.animate && a.nodeType === 1 && a.hasAttribute('data-flip');
  const first = flip ? new Map(olds.filter((n) => keyOf(n) !== null).map((n) => [n, n.getBoundingClientRect().top])) : null;
  const byKey = new Map();
  olds.forEach((n) => { const k = keyOf(n); if (k !== null) byKey.set(k, n); });
  const used = new Set();
  const plan = [];
  let oi = 0;
  for (const nn of [...b.childNodes]) {
    const k = keyOf(nn);
    let match = null;
    if (k !== null) { match = byKey.get(k) || null; if (match && (used.has(match) || match.nodeName !== nn.nodeName)) match = null; }
    else {
      while (oi < olds.length && (used.has(olds[oi]) || keyOf(olds[oi]) !== null)) oi++;
      const cand = olds[oi];
      if (cand && cand.nodeType === nn.nodeType && cand.nodeName === nn.nodeName) { match = cand; oi++; }
    }
    if (match) { used.add(match); morphNode(match, nn, ctx); plan.push(match); } else plan.push({ fresh: nn });
  }
  olds.forEach((n) => { if (!used.has(n)) { if (ctx.animate && !flip) leave(n); else n.remove(); } });
  const live = () => [...a.childNodes].filter((n) => !isLeaving(n));
  let nodes = live(); let idx = 0;
  for (const p of plan) {
    const node = p.fresh || p;
    if (p.fresh) {
      if (node.nodeType === 1) {
        [node, ...node.querySelectorAll('input,textarea,select')].forEach((x) => { if (FORM.has(x.tagName)) x._sig = vsig(x); });
        [node, ...node.querySelectorAll('[data-key]')].forEach((x) => { if (x.hasAttribute('data-key')) x._rsig = x.getAttribute('data-sig') ?? x.outerHTML; });
      }
      if (ctx.animate) enter(node);
      a.insertBefore(node, nodes[idx] || null);
      nodes = live(); idx = nodes.indexOf(node) + 1;
    } else if (nodes[idx] === node) idx++;
    else { a.insertBefore(node, nodes[idx] || null); nodes = live(); idx = nodes.indexOf(node) + 1; }
  }
  if (flip) {
    const sc = a.closest?.('.tw.tall');
    const visH = sc && sc.clientHeight > 0
      ? Math.min(a.clientHeight || sc.clientHeight, sc.clientHeight)
      : (a.clientHeight || 0);
    for (const [n, top] of first) {
      if (!n.isConnected || typeof n.animate !== 'function') continue;
      if (n._flipAnim) { try { n._flipAnim.cancel(); } catch {} n._flipAnim = null; }
      const dy = top - n.getBoundingClientRect().top;
      if (Math.abs(dy) < 1 || !visH || Math.abs(dy) > visH) continue;
      n._flipAnim = n.animate(
        [{ transform: `translateY(${dy}px)` }, { transform: 'translateY(0)' }],
        { duration: DUR[3], easing: 'cubic-bezier(.2,.8,.2,1)' },
      );
    }
  } else if (a.nodeType === 1 && a.hasAttribute('data-flip')) {
    for (const n of used) if (n._flipAnim) { try { n._flipAnim.cancel(); } catch {} n._flipAnim = null; }
  }
}

export function patch(host, htmlString, { animate } = {}) {
  if (!host) return;
  const tpl = document.createElement('template');
  tpl.innerHTML = String(htmlString);
  const ctx = { animate: (animate !== undefined ? animate : !!host._painted) && !reducedMotion() };
  morphChildren(host, tpl.content, ctx);
  host.querySelectorAll('input,textarea,select').forEach((x) => { if (x._sig === undefined) x._sig = vsig(x); });
  host._painted = true;
}

// ---------- live state, polling, announcements ----------
const live = { ok: true, at: 0, subs: new Set() };
export function reportPoll(ok) { live.ok = !!ok; if (ok) live.at = Date.now(); live.subs.forEach((f) => f(live)); }
export const onLive = (f) => { live.subs.add(f); return () => live.subs.delete(f); };
export const liveState = () => live;

// Interval that stops while the tab is hidden and refreshes once when it comes back.
export function poll(fn, ms, { immediate = false } = {}) {
  let timer = null, busy = false, dead = false;
  const run = async () => {
    if (dead || busy || tabHidden()) return;
    busy = true;
    try { await fn(); reportPoll(true); } catch { reportPoll(false); } finally { busy = false; }
  };
  const arm = () => { clearInterval(timer); timer = setInterval(run, ms); };
  const onVis = () => { if (document.hidden) clearInterval(timer); else { run(); arm(); } };
  document.addEventListener('visibilitychange', onVis);
  arm();
  if (immediate) run();
  return () => { dead = true; clearInterval(timer); document.removeEventListener('visibilitychange', onVis); };
}

// Small "updating" dot in the header of the section around `el` while `work` runs. The old data
// stays visible; the dot only appears if the refresh takes more than a moment.
export async function refreshing(el, work) {
  const sec = el && (el.closest?.('.sec') || el.querySelector?.('.sec') || el);
  const t = setTimeout(() => sec?.setAttribute('data-updating', ''), 150);
  try { return await (typeof work === 'function' ? work() : work); } finally { clearTimeout(t); sec?.removeAttribute('data-updating'); }
}

// Polite live region: only for meaningful changes (task finished, alert fired).
export function announce(msg) {
  if (!hasDom()) return;
  let r = document.getElementById('sr-live');
  if (!r) { r = document.createElement('div'); r.id = 'sr-live'; r.className = 'sr'; r.setAttribute('role', 'status'); r.setAttribute('aria-live', 'polite'); r.setAttribute('aria-atomic', 'true'); document.body.append(r); }
  r.textContent = '';
  setTimeout(() => { r.textContent = msg; }, 60);
}

// ---------- components ----------
const GLYPH = { ok: '●', warn: '▲', fail: '✕', off: '○', build: '◐' };
const PULSE = /^(running|building|pending|deploying|starting|restarting|installing|scanning|checking|updating)\b/i;
export function chk(kind, text) { const g = { ok: '✓', warn: '!', fail: '✕' }[kind] || '!'; return html`<span class="chk ${kind}"><i>${g}</i>${text}</span>`; }
// Status chip. "ok" is static; building / pending / running-style states pulse their dot.
export function st(kind, text) {
  const cls = kind === 'build' ? 'warn' : kind;
  const pulse = kind !== 'ok' && kind !== 'fail' && (kind === 'build' || PULSE.test(String(text)));
  return html`<span class="st ${cls}${pulse ? ' pulse' : ''}"><span class="dot" aria-hidden="true">${GLYPH[kind] || '●'}</span>${text}</span>`;
}
export function btn(label, { act, data = {}, kind = '', href, aria, disabled, loading, id } = {}) {
  const attrs = Object.entries(data).map(([k, v]) => ` data-${k}="${esc(v)}"`).join('');
  const dg = kind === 'link' && /^(Stop|Delete|Remove|Revoke|Lock|Disconnect|Uninstall|Reboot|Sign out all)/.test(String(label)) ? ' dg' : '';
  const cls = `btn ${kind}${dg}${loading ? ' busy' : ''}`.trim();
  const al = aria ? ` aria-label="${esc(aria)}"` : '';
  if (href) return html`<a class="${cls}" href="${href}"${raw(al)}>${label}</a>`;
  return html`<button type="button" class="${cls}"${raw(id ? ` id="${esc(id)}"` : '')}${raw(al)}${raw(act ? ` data-act="${esc(act)}"` : '')}${raw(attrs)}${raw(disabled ? ' disabled' : '')}${raw(loading ? ' aria-busy="true" data-busy="1"' : '')}>${label}</button>`;
}
// Compact icon action for table rows (tooltip + aria-label, same data-act wiring as btn()). One row of these
// replaces a wrapping group of text buttons; `danger` tints it for destructive actions.
const ROW_ICONS = {
  logs: '<path d="M4 6h16M4 12h10M4 18h7"/>',
  inspect: '<circle cx="12" cy="12" r="9"/><path d="M12 11v5M12 8h.01"/>',
  restart: '<path d="M20 12a8 8 0 1 1-2.3-5.7"/><path d="M20 4v5h-5"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  start: '<path d="M8 5.5v13l10-6.5z"/>',
  del: '<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16v4z"/><path d="M13.5 6.5l4 4"/>',
  restore: '<path d="M4 12a8 8 0 1 1 2.3 5.7"/><path d="M4 20v-5h5"/>',
  check: '<circle cx="12" cy="12" r="9"/><path d="M8 12.5l3 3 5-6"/>',
  key: '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M17 6l3 3M14 9l2 2"/>',
  star: '<path d="M12 3l2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9L12 3z"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V6a2 2 0 012-2h9"/>',
  globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18"/>',
  cert: '<path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6l8-3z"/><path d="M9 12l2 2 4-4"/>',
  unlink: '<path d="M10 14a4 4 0 005.7 0l3-3a4 4 0 00-5.7-5.7l-1 1M14 10a4 4 0 00-5.7 0l-3 3a4 4 0 005.7 5.7l1-1"/><path d="M4 4l16 16"/>',
  log: '<rect x="5" y="3" width="14" height="18" rx="2"/><path d="M9 8h6M9 12h6M9 16h4"/>',
  rollback: '<path d="M9 14L4 9l5-5"/><path d="M4 9h10a6 6 0 010 12h-3"/>',
};
export function iconAct(act, label, icon, data = {}, { danger = false, disabled = false } = {}) {
  const attrs = Object.entries(data).map(([k, v]) => ` data-${k}="${esc(v)}"`).join('');
  return raw(`<button type="button" class="btn icon row-act${danger ? ' dg' : ''}" data-act="${act}"${attrs}${disabled ? ' disabled' : ''} aria-label="${esc(label)}" title="${esc(label)}"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ROW_ICONS[icon]}</svg></button>`);
}
// Loading state for a button that triggers an API call. Keeps the width, shows an inline
// spinner (after a short delay so instant actions do not flicker) and blocks double submits.
export function setBusy(el, on = true, { pending = false } = {}) {
  if (!el || el.tagName === 'A') return;
  if (on) {
    if (el.hasAttribute('data-busy')) return;
    el.setAttribute('data-busy', '1'); el.setAttribute('aria-busy', 'true');
    if (pending) { el.classList.add('pending'); return; }
    el.style.minWidth = `${el.offsetWidth}px`;
    el._bt = setTimeout(() => el.classList.add('busy'), 120);
  } else {
    clearTimeout(el._bt);
    el.removeAttribute('data-busy'); el.removeAttribute('aria-busy');
    el.classList.remove('busy', 'pending'); el.style.minWidth = '';
  }
}
export async function withBusy(el, fn) {
  if (el && el.hasAttribute('data-busy')) return undefined;
  setBusy(el, true);
  try { return await fn(); } finally { setBusy(el, false); }
}
// Head cell: [text] | [text, 'r'] | [text, { align:'r', label:'Name'|false, check:true }].
// `label` becomes data-label for card mode under 700 px (false / '' skips that cell as a field).
function headMeta(h) {
  const text = h?.[0] ?? '';
  const opt = h?.[1];
  let align = '', label;
  if (opt === 'r') { align = 'r'; label = typeof text === 'string' ? text : ''; }
  else if (opt && typeof opt === 'object') {
    align = opt.align === 'r' ? 'r' : '';
    label = opt.label !== undefined ? opt.label : (typeof text === 'string' ? text : '');
  } else label = typeof text === 'string' ? text : '';
  if (label === false || label == null) label = '';
  return { text, align, label: String(label), check: opt?.check === true };
}
// `meta` on a row: { key, sig, cls, attrs }. key makes the row keyed (patch updates it in place);
// sig is what counts as a change worth flashing (default: the whole row).
// cards (default): under 700 px each row is a card via data-label. sticky: wide tables scroll
// inside .tw with the first column pinned instead (never scroll the page sideways).
export function table(caption, heads, rows, { rowAttrs, flip = false, cards = true, sticky = false } = {}) {
  const meta = heads.map(headMeta);
  const th = meta.map((h, i) => html`<th scope="col" class="${h.align}${h.check ? ' td-check' : ''}${!h.label && !h.check && i === meta.length - 1 ? ' td-acts' : ''}">${h.text}</th>`);
  const tr = rows.map((cells, ri) => {
    const rowMeta = cells.meta || {};
    const attrs = rowAttrs ? rowAttrs(ri) : (rowMeta.attrs || '');
    const key = rowMeta.key !== undefined ? ` data-key="${esc(rowMeta.key)}"` : '';
    const sig = rowMeta.sig !== undefined ? ` data-sig="${esc(rowMeta.sig)}"` : '';
    return html`<tr class="${rowMeta.cls || ''}"${raw(key)}${raw(sig)} ${raw(attrs)}>${cells.map((c, i) => {
      const h = meta[i] || { align: '', label: '' };
      const lbl = h.label ? ` data-label="${esc(h.label)}"` : '';
      const isAct = !h.label && !h.check && i === cells.length - 1;
      return html`<td class="${h.align}${h.check ? ' td-check' : ''}${isAct ? ' td-acts' : ''}"${raw(lbl)}>${c}</td>`;
    })}</tr>`;
  });
  const mode = sticky ? ' scroll sticky-col' : (cards ? ' cards' : '');
  return html`<div class="tw${rows.length > 12 ? ' tall' : ''}${mode}"><table><caption class="sr">${caption}</caption><thead><tr>${th}</tr></thead><tbody${raw(flip ? ' data-flip' : '')}>${tr}</tbody></table></div>`;
}
export function section(title, body, right = '', cls = '') {
  return html`<section class="sec${cls ? ` ${cls}` : ''}"><div class="sec-h"><h2>${title}</h2>${right}</div>${body}</section>`;
}
export function header(title, meta, right = '') {
  return html`<div class="hdr"><div><h1>${title}</h1><div class="meta">${meta}</div></div><div class="row">${right}</div></div>`;
}
// Tabs / segmented control. The `.ind` element slides to the selected item (see initUi).
export function tabs(items, active, act = 'tab') {
  return html`<div class="tabs" role="tablist"><span class="ind" aria-hidden="true"></span>${items.map((t, i) => html`<button type="button" role="tab" aria-selected="${String(i === active)}" data-act="${act}" data-i="${i}">${t}</button>`)}</div>`;
}
export function sw(label, on, act, data = {}) {
  const attrs = Object.entries(data).map(([k, v]) => ` data-${k}="${esc(v)}"`).join('');
  return html`<button type="button" class="switch" role="switch" aria-checked="${on ? 'true' : 'false'}" aria-label="${label}" data-act="${act}"${raw(attrs)}><span class="knob" aria-hidden="true"></span></button>`;
}
export function setSwitch(el, on) { if (el) el.setAttribute('aria-checked', on ? 'true' : 'false'); }
export const legend = (items) => html`<div class="legend">${items.map(([c, t]) => html`<span><i style="background:${c}"></i>${t}</span>`)}</div>`;
// The OpsBox mark and wordmark: "Ops" in the text colour, "Box" in the brand orange (see .brand in app.css).
// `size` is 'lg' on the sign-in screens; `href` makes it a link home.
export function logo({ href = '', size = '' } = {}) {
  const inner = html`<img src="img/logo-mark-128.png" alt="" width="30" height="30"><span class="wm">Ops<b>Box</b></span>`;
  const cls = `brand${size ? ` ${size}` : ''}`;
  return href ? html`<a class="${cls}" href="${href}" aria-label="OpsBox home">${inner}</a>` : html`<span class="${cls}" role="img" aria-label="OpsBox">${inner}</span>`;
}
// Initials for an avatar: "minh.dev" -> "MD", "admin" -> "AD".
export function initials(name) {
  const p = String(name || '').split(/[^a-z0-9]+/i).filter(Boolean);
  return ((p[0] ? p[0][0] : '?') + (p[1] ? p[1][0] : p[0] ? (p[0][1] || '') : '')).toUpperCase();
}
export const code = (text, { raw: isRaw = false } = {}) => html`<pre class="code">${isRaw ? raw(text) : text}</pre>`;

const EMPTY_ICON = '<svg viewBox="0 0 24 24" width="28" height="28" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 8l9-5 9 5v8l-9 5-9-5V8z"/><path d="M3 8l9 5 9-5"/></svg>';
// One consistent empty state: icon, one line, at most one action.
export function empty(text, action = '') {
  return html`<div class="empty">${raw(EMPTY_ICON)}<p>${text}</p>${action ? html`<div class="row">${action}</div>` : ''}</div>`;
}
// Placeholder blocks for a first load, sized like the content that replaces them.
export const sk = (h = 16, w = '100%') => html`<span class="sk" style="height:${h}px;width:${w}"></span>`;
export function skeletonSection(rows = 3) {
  return html`<div class="sec sk-sec" aria-hidden="true">${sk(20, '30%')}${[...Array(rows)].map(() => html`<div class="sk-row">${sk(16, '22%')}${sk(16, '38%')}${sk(16, '14%')}</div>`)}</div>`;
}
export function skeletonPage() {
  return html`<main class="sk-page" aria-busy="true" aria-label="Loading"><div class="hdr"><div class="col gap6">${sk(30, '220px')}${sk(14, '320px')}</div></div><div class="kpis">${[1, 2, 3, 4].map(() => html`<div class="kpi">${sk(13, '40%')}${sk(40, '55%')}${sk(6, '100%')}</div>`)}</div>${skeletonSection(4)}</main>`;
}
export function numberField({ id, name = id, value = '', min, max, step = 1, label = '', attrs = '' } = {}) {
  const a = `${min !== undefined ? ` min="${esc(min)}"` : ''}${max !== undefined ? ` max="${esc(max)}"` : ''} step="${esc(step)}"`;
  return html`<div class="stepper"><button type="button" class="step-b" data-step="-1" aria-label="Decrease ${label}" tabindex="-1">−</button><input class="input" type="number" id="${id}" name="${name}" value="${value}"${raw(a)} inputmode="numeric" autocomplete="off"${raw(attrs ? ` ${attrs}` : '')}><button type="button" class="step-b" data-step="1" aria-label="Increase ${label}" tabindex="-1">+</button></div>`;
}
export function dropzone({ id, name = id, accept = '', label = 'Drop a file here or', browse = 'browse', multiple = false } = {}) {
  return html`<label class="drop" for="${id}"><input type="file" class="drop-in" id="${id}" name="${name}"${raw(accept ? ` accept="${esc(accept)}"` : '')}${raw(multiple ? ' multiple' : '')}><svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 16V4M7 9l5-5 5 5M4 16v3a1 1 0 001 1h14a1 1 0 001-1v-3"/></svg><span class="drop-t">${label} <u>${browse}</u></span><span class="drop-f sm muted" data-file>No file chosen</span></label>`;
}

// ---------- floating layer: toasts + task rows ----------
function floatLayer() {
  let f = document.getElementById('float');
  if (!f) {
    f = document.createElement('div'); f.id = 'float';
    f.innerHTML = '<div id="toasts" role="status" aria-live="polite"></div><div id="task-dock" aria-label="Running tasks"></div>';
    document.body.append(f);
  }
  return f;
}
const TOAST_ICON = { ok: '✓', warn: '!', err: '✕' };
// kind: '' | 'ok' | 'warn' | 'err' ('error' is accepted). Stacks, shows a progress line,
// pauses while hovered or focused.
export function toast(msg, kind = '') {
  const k = kind === 'error' ? 'err' : (kind || 'ok');
  floatLayer();
  const box = document.getElementById('toasts');
  const life = k === 'err' ? 7000 : k === 'warn' ? 5500 : 3800;
  const t = document.createElement('div');
  t.className = `toast ${k}`;
  t.style.setProperty('--life', `${life}ms`);
  t.innerHTML = `<span class="t-ico" aria-hidden="true">${TOAST_ICON[k]}</span><span class="t-msg"></span><button type="button" class="t-x" aria-label="Dismiss">×</button><i class="t-bar" aria-hidden="true"></i>`;
  t.querySelector('.t-msg').textContent = msg;
  box.append(t);
  while (box.children.length > 4) box.firstElementChild.remove();
  let left = life, started = Date.now(), timer;
  const close = () => { clearTimeout(timer); t.classList.add('out'); setTimeout(() => t.remove(), DUR[2]); };
  const run = () => { started = Date.now(); timer = setTimeout(close, left); };
  const pause = () => { clearTimeout(timer); left = Math.max(800, left - (Date.now() - started)); t.classList.add('paused'); };
  const resume = () => { t.classList.remove('paused'); run(); };
  t.addEventListener('mouseenter', pause); t.addEventListener('mouseleave', resume);
  t.addEventListener('focusin', pause); t.addEventListener('focusout', resume);
  t.querySelector('.t-x').addEventListener('click', close);
  run();
  return close;
}

// Long tasks: api.js emits opsbox:task while it waits for one; each becomes a row in the dock.
const tasks = new Map();
const TASK_NAMES = { 'updates.install': 'Installing updates', 'updates.auto': 'Changing automatic updates', 'cleanup.run': 'Cleaning up', 'docker.install': 'Installing Docker', 'system.swap': 'Changing swap', 'system.disk_grow': 'Growing the disk', 'backups.run': 'Running a backup', 'backups.restore': 'Restoring a backup', 'backups.restore_test': 'Testing a restore', 'backups.check': 'Checking backups', 'backups.prune': 'Pruning old backups', panel_cert: 'Issuing the panel certificate' };
const taskLabel = (t) => t.label || TASK_NAMES[t.kind] || String(t.kind || 'Working').replace(/[._-]+/g, ' ').replace(/^./, (c) => c.toUpperCase());
function taskSteps(t) {
  if (Array.isArray(t.steps) && t.steps.length) return t.steps.map((s) => ({ label: s.label || String(s), state: s.state || 'done' }));
  const log = (t.log || []).filter(Boolean).slice(-4);
  return log.map((l, i) => ({ label: l, state: t.status === 'running' && i === log.length - 1 ? 'run' : 'done' }));
}
function taskRow(t) {
  const p = t.progress == null ? null : Math.min(1, Math.max(0, t.progress > 1 ? t.progress / 100 : t.progress));
  const state = t.status === 'ok' ? 'Done' : t.status === 'failed' ? 'Failed' : p != null ? `${Math.round(p * 100)}%` : 'Working…';
  const steps = taskSteps(t);
  return html`<div class="task ${t.status}" data-key="${t.id}" data-sig="${t.status}" role="group" aria-label="${taskLabel(t)}">
    <div class="task-h"><b>${taskLabel(t)}</b><span class="sm muted">${state}</span></div>
    ${t.status === 'running' && p == null ? html`<div class="bar indet" aria-hidden="true"><i></i></div>` : html`<div class="bar" role="progressbar" aria-label="${taskLabel(t)}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${Math.round((t.status === 'ok' ? 1 : p ?? 0) * 100)}"><i data-bar="${t.status === 'ok' ? 1 : p ?? 0}" data-th="off" class="${t.status === 'failed' ? 'lvl-danger' : ''}" style="transform:scaleX(${t.status === 'ok' ? 1 : p ?? 0})"></i></div>`}
    ${steps.length ? html`<ol class="task-steps">${steps.map((s, i) => html`<li data-key="s${i}" class="${s.state}"><i class="sdot" aria-hidden="true"></i>${s.label}</li>`)}</ol>` : ''}
    ${t.status === 'failed' && t.error ? html`<p class="sm err-t">${t.error}</p>` : ''}</div>`;
}
function paintTasks() { floatLayer(); patch(document.getElementById('task-dock'), [...tasks.values()].map(taskRow).join('')); }
// Public: also used by the UI kit to drive a fake task. `t` = { id, kind|label, status, progress?, steps?|log?, error? }.
export function trackTask(t) {
  const prev = tasks.get(t.id);
  const row = { ...prev, ...t, started: prev?.started || Date.now() };
  tasks.set(t.id, row);
  paintTasks();
  if (t.status === 'ok' || t.status === 'failed') {
    const long = Date.now() - row.started > 2500;
    if (t.status === 'failed') { toast(`${taskLabel(row)} failed. ${row.error || ''}`.trim(), 'err'); announce(`${taskLabel(row)} failed.`); }
    else { if (long || row.force) toast(`${taskLabel(row)} finished.`, 'ok'); announce(`${taskLabel(row)} finished.`); }
    setTimeout(() => { tasks.delete(t.id); paintTasks(); }, t.status === 'failed' ? 9000 : 2200);
  }
}

// ---------- dialogs ----------
const FOCUSABLE = 'a[href],button:not([disabled]),input:not([disabled]):not([type=hidden]),select:not([disabled]),textarea:not([disabled]),[tabindex]:not([tabindex="-1"])';
// Opens a modal <dialog>: focus trapped inside, Escape closes, focus goes back to the trigger.
export function openModal(d, focus) {
  const trigger = document.activeElement;
  document.body.append(d);
  d.addEventListener('keydown', (e) => {
    if (e.key !== 'Tab') return;
    const f = [...d.querySelectorAll(FOCUSABLE)].filter((x) => x.offsetParent !== null || x === document.activeElement);
    if (!f.length) return;
    const first = f[0], last = f[f.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === d)) { last.focus(); e.preventDefault(); }
    else if (!e.shiftKey && document.activeElement === last) { first.focus(); e.preventDefault(); }
  });
  d.addEventListener('close', () => { if (trigger && trigger.isConnected && typeof trigger.focus === 'function') trigger.focus({ preventScroll: true }); }, { once: true });
  d.showModal();
  if (focus) focus.focus();
}

export function confirmDialog({ title, body, confirm = 'Confirm', danger = false, typed = null }) {
  return new Promise((resolve) => {
    const d = document.createElement('dialog');
    d.className = 'dlg compact';
    d.setAttribute('aria-labelledby', 'dlg-t');
    d.innerHTML = `<form method="dialog"><div class="d-b"><h2 id="dlg-t">${esc(title)}</h2><p class="muted">${esc(body)}</p>${typed ? `<div class="field"><label for="cf">Type <b class="mono">${esc(typed)}</b> to confirm</label><input id="cf" class="input" autocomplete="off"></div>` : ''}</div><div class="d-f"><button type="button" class="btn" data-no>Cancel</button><button type="submit" class="btn ${danger ? 'danger' : 'primary'}" value="yes" ${typed ? 'disabled' : ''}>${esc(confirm)}</button></div></form>`;
    d.querySelector('[data-no]').addEventListener('click', () => d.close('no'));
    if (typed) {
      const inp = d.querySelector('#cf'), ok = d.querySelector('button[value=yes]');
      inp.addEventListener('input', () => { ok.disabled = inp.value !== typed; });
    }
    d.addEventListener('close', () => { const yes = d.returnValue === 'yes'; d.remove(); resolve(yes); });
    openModal(d, typed ? d.querySelector('#cf') : null);
  });
}

// ---------- svg stepped line ----------
export function steps(vals, w = 640, h = 180) {
  const n = vals.length; if (n < 2) return '';
  const dx = w / (n - 1);
  const y = (v) => (h - Math.min(100, Math.max(0, v)) / 100 * h).toFixed(1);
  let d = `M0,${y(vals[0])}`;
  for (let i = 1; i < n; i++) d += ` H${(i * dx).toFixed(1)} V${y(vals[i])}`;
  return d;
}

// Live line chart: new points slide in from the right (WAAPI on a transform, one step per
// update) and the y axis keeps its scale until a value no longer fits (20% headroom) or the
// data fills under 40% of it, so lines do not bounce. series: [{ color, width }].
export function createChart(host, { series, label, cap = 100, floor = 20, fmt = (v) => `${Math.round(v)}%`, w = 640, h = 180, every = 1000 }) {
  host.innerHTML = `<div class="y" aria-hidden="true"><span></span><span></span><span></span></div><svg viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" role="img" aria-label="${esc(label)}" style="width:100%;height:${h}px"><g class="pg">${series.map((s) => `<path fill="none" stroke="${s.color}" stroke-width="${s.width || 2}"${s.dash ? ` stroke-dasharray="${s.dash}"` : ''} vector-effect="non-scaling-stroke"/>`).join('')}</g></svg>`;
  const ys = host.querySelectorAll('.y span'), g = host.querySelector('.pg'), paths = [...g.querySelectorAll('path')];
  let base = 0, top = floor, cur = null, anim = null;
  const nice = (v) => { const p = 10 ** Math.floor(Math.log10(v)); const m = v / p; return (m <= 1 ? 1 : m <= 2 ? 2 : m <= 2.5 ? 2.5 : m <= 5 ? 5 : 10) * p; };
  const draw = (arrs, dx) => {
    const Y = (v) => (h - Math.min(top, Math.max(0, v)) / top * h).toFixed(1);
    arrs.forEach((vals, i) => { let d = `M0,${Y(vals[0])}`; for (let j = 1; j < vals.length; j++) d += ` H${(j * dx).toFixed(1)} V${Y(vals[j])}`; paths[i].setAttribute('d', d); });
  };
  const shiftOf = (prev, next) => {
    if (!prev || prev.length !== next.length) return -1;
    const n = next.length;
    for (let k = 0; k <= Math.min(4, n - 2); k++) { let ok = true; for (let i = 0; i < n - k; i++) if (prev[i + k] !== next[i]) { ok = false; break; } if (ok) return k; }
    return -1;
  };
  return {
    get top() { return top; },
    update(data) {
      const n = data[0].length; if (n < 2) return;
      const max = Math.max(1, ...data.flat());
      const oldTop = top;
      if (!base || max > top || max < top * 0.4) { base = max; top = Math.min(cap, Math.max(floor, nice(base * 1.2))); }
      if (top !== oldTop || !ys[0].textContent) { ys[0].textContent = fmt(top); ys[1].textContent = fmt(top / 2); ys[2].textContent = fmt(0); }
      const dx = w / (n - 1);
      if (anim) { const a = anim; anim = null; a.onfinish = null; a.cancel(); }
      const k = cur && top === oldTop && animOk() ? shiftOf(cur[0], data[0]) : -1;
      if (k > 0) {
        const ext = data.map((vals, i) => cur[i].slice(0, k).concat(vals));
        draw(ext, dx);
        const a = g.animate([{ transform: 'translateX(0px)' }, { transform: `translateX(${-k * dx}px)` }], { duration: every * k, easing: 'linear', fill: 'forwards' });
        anim = a;
        a.onfinish = () => { if (anim !== a) return; draw(data, dx); a.cancel(); anim = null; };
      } else if (k !== 0) draw(data, dx);
      cur = data.map((v) => v.slice());
    },
  };
}

// ---------- event delegation + async button state ----------
export function bindActions(root, handlers) {
  root.addEventListener('click', async (e) => {
    const el = e.target.closest('[data-act]'); if (!el || !root.contains(el)) return;
    const fn = handlers[el.dataset.act]; if (!fn) return;
    if (el.tagName === 'A') e.preventDefault();
    if (el.disabled || el.hasAttribute('data-busy') || el.getAttribute('aria-disabled') === 'true') return;
    const isBtn = el.tagName === 'BUTTON';
    if (isBtn) setBusy(el, true, { pending: el.getAttribute('role') === 'switch' });
    try { await fn(el, e); } catch (err) { toast(err.message || String(err), 'err'); }
    finally { if (isBtn) setBusy(el, false); }
  });
}

// Shows a failure on the form's `role="alert"` line and moves focus there. Nothing else on the
// form is touched: what the person typed stays exactly as it was.
export function showFormError(el, message) {
  if (!el) return;
  el.textContent = message;
  if (!el.hasAttribute('tabindex')) el.setAttribute('tabindex', '-1');
  el.focus?.();
}

// ---------- form dialog: fields = [{name,label,value,placeholder,hint,type,options,error}] ----------
// type: text (default) | password | number (stepper, min/max/step) | textarea | checkbox.
// With `run`, the dialog stays open while it works (submit shows a spinner) and shows the
// error next to the field; without it the values are returned when the person submits.
const fieldHtml = (x) => {
  const id = `f_${esc(x.name)}`;
  const err = x.error ? `<span class="hint bad" id="${id}_e">${esc(x.error)}</span>` : '';
  const hint = x.hint ? `<span class="hint">${esc(x.hint)}</span>` : '';
  const inv = x.error ? ` aria-invalid="true" aria-describedby="${id}_e"` : '';
  const cls = `field${x.error ? ' invalid' : ''}`;
  if (x.type === 'checkbox') return `<div class="${cls}"><label class="opt" for="${id}"><input type="checkbox" id="${id}" name="${esc(x.name)}"${x.value ? ' checked' : ''}> <span>${esc(x.label)}</span></label>${hint}${err}</div>`;
  let ctl;
  if (x.options) ctl = `<select class="select" id="${id}" name="${esc(x.name)}"${inv}>${x.options.map((o) => `<option${o === x.value ? ' selected' : ''}>${esc(o)}</option>`).join('')}</select>`;
  else if (x.type === 'textarea') ctl = `<textarea class="input" id="${id}" name="${esc(x.name)}" rows="${x.rows || 4}" placeholder="${esc(x.placeholder || '')}"${inv}>${esc(x.value || '')}</textarea>`;
  else if (x.type === 'number') ctl = numberField({ id, name: x.name, value: x.value ?? '', min: x.min, max: x.max, step: x.step || 1, label: x.label, attrs: x.error ? 'aria-invalid="true"' : '' }).toString();
  else ctl = `<input class="input" id="${id}" name="${esc(x.name)}" type="${x.type || 'text'}" value="${esc(x.value || '')}" placeholder="${esc(x.placeholder || '')}" autocomplete="${x.type === 'password' ? 'new-password' : 'off'}" data-1p-ignore data-lpignore="true"${inv}>`;
  return `<div class="${cls}"><label for="${id}">${esc(x.label)}</label>${ctl}${hint}${err}</div>`;
};
export function formDialog({ title, body = '', fields, submit = 'Save', danger = false, capacity = null, run = null }) {
  return new Promise((resolve) => {
    const d = document.createElement('dialog');
    d.className = 'dlg sheet';
    d.setAttribute('aria-labelledby', 'dlg-t');
    d.innerHTML = `<form method="dialog" novalidate><div class="d-handle" aria-hidden="true"></div><div class="d-b"><h2 id="dlg-t">${esc(title)}</h2>${body ? `<p class="muted">${esc(body)}</p>` : ''}${fields.map(fieldHtml).join('')}<p class="form-err" role="alert" tabindex="-1"></p></div><div class="d-f"><button type="button" class="btn" data-no>Cancel</button><button type="submit" class="btn ${danger ? 'danger' : 'primary'}" value="yes">${esc(submit)}</button></div></form>`;
    const form = d.querySelector('form'), ok = d.querySelector('button[type=submit]'), errLine = d.querySelector('.form-err');
    d.querySelector('[data-no]').addEventListener('click', () => d.close('no'));
    let result = null, capacityControl, capacityModule;
    if (capacity) ok.disabled = true;
    if (capacity) import('./capacity.js').then((m) => {
      if (!d.isConnected) return;
      capacityModule = m; const holder = document.createElement('div'); holder.innerHTML = m.capacityFields(capacity.memory).toString(); d.querySelector('.d-b').insertBefore(holder, errLine);
      if (capacity.fixed) d.querySelector('[data-capacity-memory]').readOnly = true;
      capacityControl = m.bindCapacity(d, () => capacity.request(d), 'button[type="submit"]');
    });
    const collect = () => {
      const out = {};
      d.querySelectorAll('input,select,textarea').forEach((i) => { if (i.name) out[i.name] = i.type === 'checkbox' ? i.checked : i.value; });
      if (capacityModule) out.capacity = capacityModule.capacityPayload(d);
      return out;
    };
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      if (ok.disabled || ok.hasAttribute('data-busy')) return;
      const out = collect();
      if (!run) { result = out; d.close('yes'); return; }
      errLine.textContent = '';
      d.querySelectorAll('.field.invalid').forEach((f) => f.classList.remove('invalid'));
      setBusy(ok, true);
      try { result = await run(out); d.close('yes'); }
      catch (err) {
        const msg = err.message || String(err);
        const f = err.field && d.querySelector(`[name="${CSS.escape(err.field)}"]`);
        if (f) { const box = f.closest('.field'); box.classList.add('invalid'); f.setAttribute('aria-invalid', 'true'); let h = box.querySelector('.hint.bad'); if (!h) { h = document.createElement('span'); h.className = 'hint bad'; box.append(h); } h.textContent = msg; f.focus(); }
        else showFormError(errLine, msg);
      } finally { setBusy(ok, false); }
    });
    d.addEventListener('close', () => { capacityControl?.dispose(); d.remove(); resolve(d.returnValue === 'yes' ? result : null); });
    openModal(d, d.querySelector('input:not([type=checkbox]),select,textarea'));
  });
}
// Keeps the dialog open until `run` succeeds: validation errors show next to the field.
export function askForm(opts, run) { return formDialog({ ...opts, run }); }
export function download(name, text, type = 'text/plain') {
  const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([text], { type })); a.download = name; document.body.append(a); a.click(); a.remove();
}

// ---------- keep-or-undo dialog for a provisional change ----------
// `until` is when the server undoes the change (unix seconds). `keep()` tells it to keep it.
export function keepOrUndoDialog({ title = 'Keep this change?', body, until, keep }) {
  const d = document.createElement('dialog');
  d.className = 'dlg compact';
  d.setAttribute('aria-labelledby', 'dlg-t');
  d.innerHTML = `<form method="dialog"><div class="d-b"><h2 id="dlg-t">${esc(title)}</h2><p class="muted">${esc(body)}</p><p class="mono" id="kc" role="timer" style="font-size:20px"></p></div><div class="d-f"><button class="btn primary" value="keep">Yes, keep it</button></div></form>`;
  const kc = d.querySelector('#kc');
  const tick = () => {
    const left = Math.max(0, Math.round(until - Date.now() / 1000));
    kc.textContent = left > 0 ? `Undoing in ${left} s` : 'Time is up: the change was undone.';
    if (left === 0) { clearInterval(timer); d.querySelector('button').textContent = 'Close'; d.querySelector('button').value = 'close'; }
  };
  const timer = setInterval(tick, 500); tick();
  d.addEventListener('close', async () => {
    clearInterval(timer); const kept = d.returnValue === 'keep'; d.remove();
    if (kept) { try { await keep(); toast('Change kept.'); } catch (e) { toast(e.message || 'Could not keep the change.', 'err'); } }
  });
  openModal(d, d.querySelector('button'));
}

// ---------- one-time document wiring (call once from main.js and the kit page) ----------
let started = false;
export function initUi() {
  if (started || !hasDom()) return;
  started = true;
  const root = document.documentElement;
  const vis = () => { root.dataset.hidden = document.hidden ? '1' : ''; };
  document.addEventListener('visibilitychange', vis); vis();
  // Number steppers.
  document.addEventListener('click', (e) => {
    const b = e.target.closest('.step-b'); if (!b) return;
    const inp = b.parentElement.querySelector('input'); if (!inp || inp.readOnly || inp.disabled) return;
    const dir = Number(b.dataset.step);
    const stepv = Number(inp.step) || 1, min = inp.min === '' ? -Infinity : Number(inp.min), max = inp.max === '' ? Infinity : Number(inp.max);
    const cur = Number(inp.value) || 0;
    inp.value = String(Math.min(max, Math.max(min, Math.round((cur + dir * stepv) / stepv) * stepv)));
    inp.dispatchEvent(new Event('input', { bubbles: true })); inp.dispatchEvent(new Event('change', { bubbles: true }));
  });
  // File drop zones.
  const zone = (e) => e.target.closest?.('.drop');
  document.addEventListener('dragover', (e) => { const z = zone(e); if (z) { e.preventDefault(); z.classList.add('over'); } });
  document.addEventListener('dragleave', (e) => { const z = zone(e); if (z && !z.contains(e.relatedTarget)) z.classList.remove('over'); });
  document.addEventListener('drop', (e) => {
    const z = zone(e); if (!z) return; e.preventDefault(); z.classList.remove('over');
    const inp = z.querySelector('input[type=file]'); if (inp && e.dataTransfer?.files?.length) { inp.files = e.dataTransfer.files; inp.dispatchEvent(new Event('change', { bubbles: true })); }
  });
  document.addEventListener('change', (e) => {
    const inp = e.target; if (inp.type !== 'file') return;
    const z = inp.closest('.drop'); const t = z && z.querySelector('[data-file]');
    if (t) t.textContent = inp.files.length ? [...inp.files].map((f) => `${f.name} (${bytes(f.size)})`).join(', ') : 'No file chosen';
    if (z) z.classList.toggle('has', inp.files.length > 0);
  });
  // Sliding indicator on tabs / segmented controls.
  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver((es) => es.forEach((en) => placeInd(en.target))) : null;
  const watched = new WeakSet();
  const scan = (scope) => scope.querySelectorAll?.('.tabs,.segctl').forEach((t) => { if (ro && !watched.has(t)) { watched.add(t); ro.observe(t); } placeInd(t); });
  new MutationObserver((ms) => {
    const seen = new Set();
    for (const m of ms) {
      const t = m.target.nodeType === 1 ? m.target.closest?.('.tabs,.segctl') : null;
      if (t) seen.add(t);
      m.addedNodes.forEach((n) => { if (n.nodeType === 1) { if (n.matches?.('.tabs,.segctl')) seen.add(n); n.querySelectorAll?.('.tabs,.segctl').forEach((x) => seen.add(x)); } });
    }
    seen.forEach((t) => { if (ro && !watched.has(t)) { watched.add(t); ro.observe(t); } placeInd(t); });
  }).observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ['aria-selected'] });
  scan(document);
  // Long tasks reported by the API layer.
  addEventListener('opsbox:task', (e) => trackTask(e.detail));
}
function placeInd(t) {
  const ind = t.querySelector(':scope > .ind'); if (!ind) return;
  const cur = t.querySelector('[aria-selected="true"]');
  if (!cur || !cur.offsetWidth) { ind.style.opacity = '0'; return; }
  ind.style.opacity = '1';
  ind.style.width = `${cur.offsetWidth}px`; ind.style.height = `${cur.offsetHeight}px`;
  ind.style.transform = `translate(${cur.offsetLeft}px,${cur.offsetTop}px)`;
  if (!ind.classList.contains('ready')) requestAnimationFrame(() => requestAnimationFrame(() => ind.classList.add('ready')));
}
