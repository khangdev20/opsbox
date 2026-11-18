import { html, raw, st, btn, header, section, bindActions, confirmDialog, formDialog, download, toast } from '../ui.js';
import * as api from '../api.js';

function renderEndpoint(p, vals) {
  if (!p) return '';
  if (p.sftp) return vals.host ? `sftp://${vals.user || 'user'}@${vals.host}` : '';
  const tpl = p.endpointTemplate || '';
  if (tpl === '{endpoint}') return vals.endpoint || '';
  const filled = tpl
    .replace('{account}', (vals.account || '').trim())
    .replace('{region}', (vals.region || '').trim())
    .replace('{host}', (vals.host || '').trim())
    .replace('{user}', (vals.user || '').trim() || 'user');
  return /\{[a-z]+\}/.test(filled) ? '' : filled;
}

export async function view() {
  const { providers, hostname } = await api.getProviders();
  let pid = (providers[0] && providers[0].id) || 'r2', vals = {}, test = null, root;
  const P = () => providers.find((x) => x.id === pid);
  const tile = (p) => html`<button type="button" class="ptile ${p.id === pid ? 'on' : ''}" data-act="pick" data-id="${p.id}" aria-pressed="${String(p.id === pid)}"><span class="plogo" data-logo-slot="${p.id}" title="Provider logo">${p.mono}</span><span><b>${p.name}</b><span class="sm muted">${p.sub}</span></span></button>`;
  const field = (f) => {
    const k = f.key, label = f.label, hint = f.hint, secret = !!f.secret, optional = !!f.optional;
    return html`<div class="field"><label for="f_${k}">${label}${optional ? raw(' <span class="muted">(optional)</span>') : ''}</label>
      <div class="row" style="gap:8px;flex-wrap:nowrap"><input class="input grow mono" id="f_${k}" data-k="${k}" type="${secret ? 'password' : 'text'}" value="${vals[k] || ''}" autocomplete="off" spellcheck="false">${secret ? btn('Show', { act: 'show', kind: 'link' }) : ''}</div>
      ${hint ? html`<span class="hint">${hint}</span>` : ''}${secret ? html`<span class="hint">Stored encrypted on this server. Never shown again after saving.</span>` : ''}</div>`;
  };
  const body = () => {
    const p = P(), ep = renderEndpoint(p, vals);
    return html`
    <div class="steps-row"><span class="done"><i>✓</i>Choose provider</span><span class="${test?.ok ? 'done' : 'on'}"><i>${test?.ok ? '✓' : '2'}</i>Credentials</span><span class="${test?.ok ? 'on' : ''}"><i>3</i>Test and save</span></div>
    ${section('Provider', html`<div class="pgrid">${providers.map(tile)}</div><p class="sm muted">Any S3-compatible storage works. Pick yours and opsbox fills in the endpoint and shows where to find the keys.</p>`)}
    <div class="cols"><div class="w3">
      ${section(`${p.name} ${p.sftp ? 'connection' : 'credentials'}`, html`${p.fields.map(field)}
        ${ep ? html`<div class="field"><label>Endpoint</label><input class="input mono" value="${ep}" readonly style="background:var(--fill);color:var(--muted)"><span class="hint">Filled in for you.</span></div>` : ''}
        <div class="field"><label for="f_name">Name in opsbox</label><input class="input" id="f_name" value="${vals.__name || `Offsite (${p.name})`}" data-k="__name"></div>`)}
      ${section('Test connection', test ? html`<div style="display:flex;flex-direction:column;gap:10px">${test.steps.map(([k, t]) => html`<span class="row" style="gap:10px;flex-wrap:nowrap">${st(k, k === 'ok' ? 'ok' : k === 'warn' ? 'check' : 'failed')}<span>${t}</span></span>`)}</div>` : html`<p class="muted sm">Run a test before saving. opsbox connects, finds the bucket, and writes and reads back a small file.</p>`, btn(test ? 'Run test again' : 'Run test', { act: 'test', kind: test ? 'link' : 'primary' }))}
    </div><div>
      ${section('Where to find these keys', html`<ol class="howto">${p.steps.map((x) => html`<li>${x}</li>`)}</ol>${p.tip ? html`<div class="note ok"><b>Good choice:</b> ${p.tip}</div>` : ''}`)}
      ${section('Encryption', html`<p class="muted sm">Everything is encrypted on this server before upload. ${p.name} only ever sees encrypted data.</p><div>${btn('Download recovery key', { act: 'key' })}</div><p class="sm" style="color:var(--warn);margin:0">▲ Keep the recovery key off this server. Without it, these backups cannot be opened.</p>`)}
    </div></div>
    <div class="row end"><a class="btn" href="#/backups">Cancel</a>${btn('Save destination', { act: 'save', kind: 'primary', disabled: !test?.ok })}</div>`;
  };
  const read = () => root.querySelectorAll('[data-k]').forEach((i) => { vals[i.dataset.k] = i.value; });
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  return {
    nav: 'backups', title: 'Add backup destination',
    html: html`<main>${header('Add backup destination', `Where should opsbox send backups for ${hostname || 'this server'}?`, html`<a class="btn" href="#/backups">Cancel</a>`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      root.addEventListener('input', (e) => { if (e.target.dataset.k) { vals[e.target.dataset.k] = e.target.value; if (test) { test = null; const pos = e.target.selectionStart, id = e.target.id; paint(); const n = root.querySelector('#' + id); if (n) { n.focus(); n.setSelectionRange(pos, pos); } } else if (['account', 'region', 'endpoint', 'host', 'user'].includes(e.target.dataset.k)) { const id = e.target.id, pos = e.target.selectionStart; paint(); const n = root.querySelector('#' + id); if (n) { n.focus(); n.setSelectionRange(pos, pos); } } } });
      bindActions(root, {
        pick: (el) => { read(); pid = el.dataset.id; test = null; vals = { __name: '' }; paint(); },
        show: (el) => { const i = el.parentElement.querySelector('input'); i.type = i.type === 'password' ? 'text' : 'password'; el.textContent = i.type === 'password' ? 'Show' : 'Hide'; },
        test: async () => { read(); test = await api.testDestination(pid, vals); paint(); },
        key: async () => {
          const o = await formDialog({
            title: 'Download recovery key',
            body: 'Re-enter your account password to export the restic encryption recovery password.',
            fields: [{ name: 'password', label: 'Account password', type: 'password' }],
            submit: 'Download',
          });
          if (!o) return;
          const res = await api.downloadRecoveryKey(o.password);
          const text = `# opsbox backup recovery key (${res.hostname || hostname || 'server'})\n# ${res.instructions || ''}\nRESTIC_PASSWORD=${res.password}\n`;
          download(`opsbox-${(res.hostname || hostname || 'server').replace(/[^a-z0-9.-]/gi, '-')}-recovery-key.txt`, text);
          toast('Recovery key downloaded.');
        },
        save: async () => {
          read();
          const saved = await api.saveDestination(pid, vals, vals.__name);
          if (saved && saved.publicKey) {
            await confirmDialog({
              title: 'Add SSH public key on your SFTP server',
              body: html`<p class="sm">Add this Ed25519 key to <span class="mono">~/.ssh/authorized_keys</span> on your backup server, then click <b>Test</b> on this destination in Backups.</p><pre class="mono sm" style="white-space:pre-wrap;word-break:break-all;background:var(--fill);padding:10px;border-radius:6px">${saved.publicKey}</pre>`,
              confirm: 'Done',
            });
          } else {
            toast('Destination saved. Next backups go there.');
          }
          location.hash = '#/backups';
        },
      });
    },
  };
}
