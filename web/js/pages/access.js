import { html, st, btn, table, section, header, bindActions, confirmDialog, toast } from '../ui.js';
import * as api from '../api.js';

export async function view() {
  let d = await api.getAccess(), root;
  const body = () => html`
    ${section('SSH access', html`<div class="row spread"><div><b>${d.sshOpen ? 'SSH is open for your IP' : 'SSH daemon is off'}</b><div class="sm muted mono">${d.sshOpen ? 'closes automatically within one hour' : 'port 22 closed in the firewall · password login disabled'}</div></div>${btn('Open for 1 hour', { act: 'ssh', kind: 'primary', disabled: d.sshOpen })}</div>`)}
    ${section('Authorized keys', html`${table('Authorized keys', [['Name'], ['User'], ['Fingerprint'], ['Added'], ['Last used'], ['', 'r']], d.keys.map((k) => [html`<b>${k.name}</b><div class="sm muted">${k.type}${k.note ? ' · ' + k.note : ''}</div>`, html`<span class="mono">${k.user}</span>`, html`<span class="mono sm">${k.fp}</span>`, k.added, k.stale ? st('warn', k.stale) : k.used || 'never', btn('Remove', { act: 'rmkey', data: { id: k.id }, kind: 'link' })]))}
      <div style="max-width:640px;display:flex;flex-direction:column;gap:14px;margin-top:8px">
        <div class="field"><label for="ku">For user</label><select class="select" id="ku">${d.users.map((u) => html`<option>${u.name}</option>`)}</select></div>
        <div class="field"><label for="kp">Public key</label><textarea class="input" id="kp" rows="3" placeholder="ssh-ed25519 AAAA… name" style="height:auto;font-family:var(--mono)"></textarea><span class="hint">Paste the contents of the .pub file. Never paste a private key here.</span></div>
        <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p><div>${btn('Add key', { act: 'addkey', kind: 'primary' })}</div></div>`)}
    ${section('Linux users', html`${table('Linux users', [['User'], ['Groups'], ['Sudo'], ['Last login'], ['Status'], ['', 'r']], d.users.map((u) => [html`<b>${u.name}</b><div class="sm muted">${u.note}</div>`, html`<span class="mono">${u.groups || '—'}</span>`, u.sudo ? 'yes' : 'no', u.last, u.locked ? st('ok', 'locked') : st('ok', 'active'), btn(u.locked ? 'Unlock' : 'Lock', { act: 'lock', data: { n: u.name, l: u.locked ? '0' : '1' }, kind: 'link' })]))}<p class="muted sm">Locking a user keeps their files but stops every login and key. Panel accounts are separate, see Security.</p>`)}
    ${section('Keys opsbox uses to reach other servers', table('Outgoing keys', [['Name'], ['Used for'], ['Storage'], ['', 'r']], d.out.map((o) => [html`<b>${o.name}</b><div class="sm muted">${o.sub}</div>`, 'Terminal tab', st('ok', 'encrypted'), btn('Rotate', { act: 'rot', data: { id: o.id }, kind: 'link' })])))}`;
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  const reload = async () => { d = await api.getAccess(); paint(); };
  return {
    nav: 'access', title: 'Access',
    html: html`<main>${header('Access', 'Who can log in to this server, and with what', html`<a class="btn" href="#/security">← Security</a>`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      bindActions(root, {
        ssh: async () => { if (await confirmDialog({ title: 'Open SSH for one hour?', body: 'Port 22 opens to your current IP only and closes by itself.', confirm: 'Open SSH' })) { await api.openSsh(); toast('Port 22 is open to your IP for one hour.'); await reload(); } },
        addkey: async () => { try { await api.addKey(root.querySelector('#ku').value, root.querySelector('#kp').value); toast('Key added.'); await reload(); } catch (e) { root.querySelector('#err').textContent = e.message; } },
        rmkey: async (el) => { if (await confirmDialog({ title: 'Remove this key?', body: 'Whoever holds it can no longer log in.', confirm: 'Remove', danger: true })) { await api.removeKey(el.dataset.id); await reload(); } },
        lock: async (el) => { const lock = el.dataset.l === '1'; if (lock && !await confirmDialog({ title: `Lock ${el.dataset.n}?`, body: 'All logins and keys for this user stop working.', confirm: 'Lock', danger: true })) return; await api.lockUser(el.dataset.n, lock); await reload(); },
        rot: async (el) => { await api.rotateOutKey(el.dataset.id); toast('New key generated. Add it to the remote server.'); },
      });
    },
  };
}
