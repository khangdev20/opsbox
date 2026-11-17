import { html, raw, st, btn, header, section, bindActions, toast } from '../ui.js';
import * as api from '../api.js';

export async function view() {
  let pid = 'r2', vals = {}, test = null, root;
  const P = () => api.PROVIDERS.find((x) => x.id === pid);
  const tile = (p) => html`<button type="button" class="ptile ${p.id === pid ? 'on' : ''}" data-act="pick" data-id="${p.id}" aria-pressed="${p.id === pid}"><span class="plogo" data-logo-slot="${p.id}" title="Provider logo">${p.mono}</span><span><b>${p.name}</b><span class="sm muted">${p.sub}</span></span></button>`;
  const field = ([k, label, hint]) => {
    const secret = k === 'secret';
    return html`<div class="field"><label for="f_${k}">${label}${k === 'prefix' || (k === 'region' && pid === 'custom') ? raw(' <span class="muted">(optional)</span>') : ''}</label>
      <div class="row" style="gap:8px;flex-wrap:nowrap"><input class="input grow mono" id="f_${k}" data-k="${k}" type="${secret ? 'password' : 'text'}" value="${vals[k] || ''}" autocomplete="off" spellcheck="false">${secret ? btn('Show', { act: 'show', kind: 'link' }) : ''}</div>
      ${hint ? html`<span class="hint">${hint}</span>` : ''}${secret ? html`<span class="hint">Stored encrypted on this server. Never shown again after saving.</span>` : ''}</div>`;
  };
  const body = () => {
    const p = P(), ep = p.endpoint(vals);
    return html`
    <div class="steps-row"><span class="${test?.ok ? 'done' : 'done'}"><i>✓</i>Choose provider</span><span class="${test?.ok ? 'done' : 'on'}"><i>${test?.ok ? '✓' : '2'}</i>Credentials</span><span class="${test?.ok ? 'on' : ''}"><i>3</i>Test and save</span></div>
    ${section('Provider', html`<div class="pgrid">${api.PROVIDERS.map(tile)}</div><p class="sm muted">Any S3-compatible storage works. Pick yours and opsbox fills in the endpoint and shows where to find the keys.</p>`)}
    <div class="cols"><div class="w3">
      ${section(`${p.name} ${p.sftp ? 'connection' : 'credentials'}`, html`${p.fields.map(field)}
        ${ep ? html`<div class="field"><label>Endpoint</label><input class="input mono" value="${ep}" readonly style="background:var(--fill);color:var(--muted)"><span class="hint">Filled in for you.</span></div>` : ''}
        <div class="field"><label for="f_name">Name in opsbox</label><input class="input" id="f_name" value="${vals.__name || `Offsite (${p.name})`}" data-k="__name"></div>`)}
      ${section('Test connection', test ? html`<div style="display:flex;flex-direction:column;gap:10px">${test.steps.map(([k, t]) => html`<span class="row" style="gap:10px;flex-wrap:nowrap">${st(k, k === 'ok' ? 'ok' : k === 'warn' ? 'check' : 'failed')}<span>${t}</span></span>`)}</div>` : html`<p class="muted sm">Run a test before saving. opsbox connects, finds the bucket, and writes and reads back a small file.</p>`, btn(test ? 'Run test again' : 'Run test', { act: 'test', kind: test ? 'link' : 'primary' }))}
    </div><div>
      ${section('Where to find these keys', html`<ol class="howto">${p.steps.map((x) => html`<li>${x}</li>`)}</ol>${p.tip ? html`<div class="note ok"><b>Good choice:</b> ${p.tip}</div>` : ''}`)}
      ${section('Encryption', html`<p class="muted sm">Everything is encrypted on this server before upload. ${p.name} only ever sees encrypted data.</p><div>${btn('Download recovery key', { act: 'key' })}</div><p class="sm" style="color:var(--warn);margin:0">▲ Keep the recovery key off this server. Without it, these backups cannot be opened.</p>`)}
    </div></div>
    <div class="row" style="justify-content:flex-end"><a class="btn" href="#/backups">Cancel</a>${btn('Save destination', { act: 'save', kind: 'primary', disabled: !test?.ok })}</div>`;
  };
  const read = () => root.querySelectorAll('[data-k]').forEach((i) => { vals[i.dataset.k] = i.value; });
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  return {
    nav: 'backups', title: 'Add backup destination',
    html: html`<main>${header('Add backup destination', 'Where should opsbox send backups for srv-sgn-01?', html`<a class="btn" href="#/backups">Cancel</a>`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      root.addEventListener('input', (e) => { if (e.target.dataset.k) { vals[e.target.dataset.k] = e.target.value; if (test) { test = null; const pos = e.target.selectionStart, id = e.target.id; paint(); const n = root.querySelector('#' + id); n.focus(); n.setSelectionRange(pos, pos); } else if (['account', 'region', 'endpoint', 'host', 'user'].includes(e.target.dataset.k)) { const id = e.target.id, pos = e.target.selectionStart; paint(); const n = root.querySelector('#' + id); n.focus(); n.setSelectionRange(pos, pos); } } });
      bindActions(root, {
        pick: (el) => { read(); pid = el.dataset.id; test = null; vals = { __name: '' }; paint(); },
        show: (el) => { const i = el.parentElement.querySelector('input'); i.type = i.type === 'password' ? 'text' : 'password'; el.textContent = i.type === 'password' ? 'Show' : 'Hide'; },
        test: async () => { read(); test = await api.testDestination(pid, vals); paint(); },
        key: () => toast('The recovery key downloads in the real build.'),
        save: async () => { read(); await api.saveDestination(pid, vals, vals.__name); toast('Destination saved. Next backups go there.'); location.hash = '#/backups'; },
      });
    },
  };
}
