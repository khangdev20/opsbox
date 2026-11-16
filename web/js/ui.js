// Small UI toolkit: safe html templating, formatters, toast, confirm dialog.
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

// ---------- components ----------
const GLYPH = { ok: '■', warn: '▲', fail: '✕', off: '□', build: '◐' };
export function st(kind, text) {
  const cls = kind === 'build' ? 'warn' : kind;
  return html`<span class="st ${cls}">${GLYPH[kind] || '■'} ${text}</span>`;
}
export function btn(label, { act, data = {}, kind = '', href, aria, disabled } = {}) {
  const attrs = Object.entries(data).map(([k, v]) => ` data-${k}="${esc(v)}"`).join('');
  const cls = `btn ${kind}`.trim();
  const al = aria ? ` aria-label="${esc(aria)}"` : '';
  if (href) return html`<a class="${cls}" href="${href}"${raw(al)}>${label}</a>`;
  return html`<button type="button" class="${cls}"${raw(al)}${raw(act ? ` data-act="${esc(act)}"` : '')}${raw(attrs)}${raw(disabled ? ' disabled' : '')}>${label}</button>`;
}
export function table(caption, heads, rows, { rowAttrs } = {}) {
  const th = heads.map(([t, a]) => html`<th class="${a === 'r' ? 'r' : ''}">${t}</th>`);
  const tr = rows.map((cells, ri) => {
    const meta = cells.meta || {};
    const attrs = rowAttrs ? rowAttrs(ri) : (meta.attrs || '');
    return html`<tr class="${meta.cls || ''}" ${raw(attrs)}>${cells.map((c, i) => html`<td class="${heads[i] && heads[i][1] === 'r' ? 'r' : ''}">${c}</td>`)}</tr>`;
  });
  return html`<div class="tw"><table><caption class="sr">${caption}</caption><thead><tr>${th}</tr></thead><tbody>${tr}</tbody></table></div>`;
}
export function section(title, body, right = '') {
  return html`<section class="sec"><div class="sec-h"><h2>${title}</h2>${right}</div>${body}</section>`;
}
export function header(title, meta, right = '') {
  return html`<div class="hdr"><div><h1>${title}</h1><div class="meta">${meta}</div></div><div class="row">${right}</div></div>`;
}
export function tabs(items, active, act = 'tab') {
  return html`<div class="tabs" role="tablist">${items.map((t, i) => html`<button type="button" role="tab" aria-selected="${i === active}" data-act="${act}" data-i="${i}">${t}</button>`)}</div>`;
}
export function sw(label, on, act, data = {}) {
  const attrs = Object.entries(data).map(([k, v]) => ` data-${k}="${esc(v)}"`).join('');
  return html`<button type="button" class="switch" role="switch" aria-checked="${!!on}" aria-label="${label}" data-act="${act}"${raw(attrs)}>${on ? 'ON' : 'OFF'}</button>`;
}
export const legend = (items) => html`<div class="legend">${items.map(([c, t]) => html`<span><i style="background:${c}"></i>${t}</span>`)}</div>`;
export const code = (text, { raw: isRaw = false } = {}) => html`<pre class="code">${isRaw ? raw(text) : text}</pre>`;

// ---------- toast ----------
export function toast(msg, kind = '') {
  let box = document.getElementById('toasts');
  if (!box) { box = document.createElement('div'); box.id = 'toasts'; box.setAttribute('role', 'status'); document.body.append(box); }
  const t = document.createElement('div');
  t.className = `toast ${kind}`; t.textContent = msg;
  box.append(t);
  setTimeout(() => t.remove(), kind === 'err' ? 6000 : 3500);
}

// ---------- confirm dialog (native <dialog>) ----------
export function confirmDialog({ title, body, confirm = 'Confirm', danger = false, typed = null }) {
  return new Promise((resolve) => {
    const d = document.createElement('dialog');
    d.innerHTML = `<form method="dialog"><div class="d-b"><h2>${esc(title)}</h2><p class="muted">${esc(body)}</p>${typed ? `<div class="field"><label for="cf">Type <b class="mono">${esc(typed)}</b> to confirm</label><input id="cf" class="input" autocomplete="off"></div>` : ''}</div><div class="d-f"><button class="btn" value="no">Cancel</button><button class="btn ${danger ? 'danger' : 'primary'}" value="yes" ${typed ? 'disabled' : ''}>${esc(confirm)}</button></div></form>`;
    document.body.append(d);
    if (typed) {
      const inp = d.querySelector('#cf'), ok = d.querySelector('button[value=yes]');
      inp.addEventListener('input', () => { ok.disabled = inp.value !== typed; });
    }
    d.addEventListener('close', () => { const yes = d.returnValue === 'yes'; d.remove(); resolve(yes); });
    d.showModal();
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

// ---------- event delegation + async button state ----------
export function bindActions(root, handlers) {
  root.addEventListener('click', async (e) => {
    const el = e.target.closest('[data-act]'); if (!el || !root.contains(el)) return;
    const fn = handlers[el.dataset.act]; if (!fn) return;
    if (el.tagName === 'A') e.preventDefault();
    if (el.disabled) return;
    const prev = el.disabled; if (el.tagName === 'BUTTON') el.disabled = true;
    try { await fn(el, e); } catch (err) { toast(err.message || String(err), 'err'); }
    finally { if (el.isConnected && el.tagName === 'BUTTON') el.disabled = prev; }
  });
}

// ---------- form dialog: fields = [{name,label,value,placeholder,hint,type,options}] ----------
export function formDialog({ title, body = '', fields, submit = 'Save', danger = false }) {
  return new Promise((resolve) => {
    const d = document.createElement('dialog');
    const f = fields.map((x) => `<div class="field"><label for="f_${esc(x.name)}">${esc(x.label)}</label>${x.options
      ? `<select class="select" id="f_${esc(x.name)}" name="${esc(x.name)}">${x.options.map((o) => `<option${o === x.value ? ' selected' : ''}>${esc(o)}</option>`).join('')}</select>`
      : `<input class="input" id="f_${esc(x.name)}" name="${esc(x.name)}" type="${x.type || 'text'}" value="${esc(x.value || '')}" placeholder="${esc(x.placeholder || '')}" autocomplete="off">`}${x.hint ? `<span class="hint">${esc(x.hint)}</span>` : ''}</div>`).join('');
    d.innerHTML = `<form method="dialog"><div class="d-b"><h2>${esc(title)}</h2>${body ? `<p class="muted">${esc(body)}</p>` : ''}${f}<p class="err" role="alert" style="color:var(--danger);font-size:13px;min-height:0"></p></div><div class="d-f"><button class="btn" value="no" formnovalidate>Cancel</button><button class="btn ${danger ? 'danger' : 'primary'}" value="yes">${esc(submit)}</button></div></form>`;
    document.body.append(d);
    d.addEventListener('close', () => {
      const ok = d.returnValue === 'yes'; const out = {};
      if (ok) d.querySelectorAll('input,select').forEach((i) => { out[i.name] = i.value; });
      d.remove(); resolve(ok ? out : null);
    });
    d.showModal(); const first = d.querySelector('input,select'); if (first) first.focus();
  });
}
// Re-asks until validate() passes (validate throws Error with a message).
export async function askForm(opts, run) {
  let vals = opts.fields;
  for (;;) {
    const out = await formDialog({ ...opts, fields: vals });
    if (!out) return null;
    try { return await run(out); } catch (e) { toast(e.message || String(e), 'err'); vals = opts.fields.map((f) => ({ ...f, value: out[f.name] })); }
  }
}
export function download(name, text, type = 'text/plain') {
  const a = document.createElement('a'); a.href = URL.createObjectURL(new Blob([text], { type })); a.download = name; document.body.append(a); a.click(); a.remove();
}
