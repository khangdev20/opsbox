import { html, st, btn, table, section, header, bindActions, confirmDialog, toast } from '../ui.js';
import * as api from '../api.js';
import { session, server } from '../store.js';

// Real servers report what the SSH daemon is set to do; the demo only has the open/closed state.
const sshTitle = (d) => (!d.ssh ? 'SSH daemon is off' : !d.ssh.installed ? 'No SSH server is installed' : d.ssh.active ? `SSH is running on port ${d.ssh.port}` : 'SSH daemon is off');
const sshDetail = (d) => (!d.ssh ? 'port 22 closed in the firewall · password login disabled' : `${d.ssh.passwordLogin === null ? 'password login unknown' : d.ssh.passwordLogin ? 'password login ON (keys are safer)' : 'password login disabled'} · root login ${d.ssh.rootLogin ?? 'unknown'}`);

export async function view() {
  let d = await api.getAccess(), sec = null, root;
  try { sec = await api.getSecurity(); } catch { /* viewers see access without the panel users list */ }
  const me = session.user || '';
  const isOwner = server.role === 'admin' || api.MODE === 'mock';
  const myDisplay = () => server.displayName || (sec?.users?.find((u) => u.you)?.displayName) || '';
  const body = () => html`
    ${section('Your profile', html`<div style="max-width:640px;display:flex;flex-direction:column;gap:14px">
      <div class="field"><label for="uname">Username</label><input class="input" id="uname" value="${me}" disabled><span class="hint">Sign-in name. It cannot be changed.</span></div>
      <div class="field"><label for="dn">Display name</label><input class="input" id="dn" value="${myDisplay()}" maxlength="64" autocomplete="nickname" placeholder="Your name"><span class="hint">Shown in the account menu and audit log. At most 64 characters. Leave blank to clear.</span></div>
      <div class="field"><span class="hint">Two-factor: ${server.totp?.enabled !== false ? 'on (authenticator app)' : html`<a href="#/enroll">off — enable it</a>`}</span></div>
      <p id="profile-err" class="form-err" role="alert"></p>
      <div>${btn('Save display name', { act: 'savedn', kind: 'primary' })}</div>
    </div>`)}
    ${isOwner && sec ? section('Panel users', html`${table('Panel users', [['User'], ['Display name'], ['Role'], ['2FA'], ['', 'r']], sec.users.map((u) => [
      html`<span class="mono">${u.name}</span>${u.you ? html` <span class="sm muted">(you)</span>` : ''}`,
      u.displayName || '—',
      u.role,
      u.pending ? st('warn', 'invited') : u.tfa ? st('ok', 'on') : st('warn', 'off'),
      u.you ? '' : html`<div class="row" style="gap:8px;justify-content:flex-end">${btn('Edit name', { act: 'editdn', data: { n: u.name, d: u.displayName || '' }, kind: 'link' })}${api.isReal('removeUser') ? btn('Remove', { act: 'rmuser', data: { n: u.name }, kind: 'link' }) : ''}</div>`,
    ]))}<div class="row mt12">${btn('Invite user', { act: 'invite' })}</div>`) : ''}
    ${section('SSH access', html`<div class="row spread"><div><b>${d.sshOpen ? 'SSH is open for your IP' : sshTitle(d)}</b><div class="sm muted mono">${d.sshOpen ? 'closes automatically within one hour' : sshDetail(d)}</div></div>${btn('Open for 1 hour', { act: 'ssh', kind: 'primary', disabled: d.sshOpen })}</div>`)}
    ${section('Authorized keys', html`${table('Authorized keys', [['Name'], ['User'], ['Fingerprint'], ['Added'], ['Last used'], ['', 'r']], d.keys.map((k) => [html`<b>${k.name}</b><div class="sm muted">${k.type}${k.note ? ' · ' + k.note : ''}</div>`, html`<span class="mono">${k.user}</span>`, html`<span class="mono sm">${k.fp}</span>`, k.added ?? '—', k.stale ? st('warn', k.stale) : k.used === null ? '—' : k.used || 'never', btn('Remove', { act: 'rmkey', data: { id: k.id }, kind: 'link' })]))}
      <div style="max-width:640px;display:flex;flex-direction:column;gap:14px;margin-top:8px">
        <div class="field"><label for="ku">For user</label><select class="select" id="ku">${d.users.map((u) => html`<option>${u.name}</option>`)}</select></div>
        <div class="field"><label for="kp">Public key</label><textarea class="input" id="kp" rows="3" placeholder="ssh-ed25519 AAAA… name" style="height:auto;font-family:var(--mono)"></textarea><span class="hint">Paste the contents of the .pub file. Never paste a private key here.</span></div>
        <p id="err" class="form-err" role="alert"></p><div>${btn('Add key', { act: 'addkey', kind: 'primary' })}</div></div>`)}
    ${section('Linux users', html`${table('Linux users', [['User'], ['Groups'], ['Sudo'], ['Last login'], ['Status'], ['', 'r']], d.users.map((u) => [html`<b>${u.name}</b><div class="sm muted">${u.note}</div>`, html`<span class="mono">${u.groups || '—'}</span>`, u.sudo ? 'yes' : 'no', u.last || '—', u.locked ? st('warn', 'locked') : st('ok', 'active'), btn(u.locked ? 'Unlock' : 'Lock', { act: 'lock', data: { n: u.name, l: u.locked ? '0' : '1' }, kind: 'link' })]))}<p class="muted sm">Locking a user keeps their files but stops every login and key. Panel accounts are separate, see Security.</p>`)}
    ${d.out.length ? section('Keys opsbox uses to reach other servers', table('Outgoing keys', [['Name'], ['Used for'], ['Storage'], ['', 'r']], d.out.map((o) => [html`<b>${o.name}</b><div class="sm muted">${o.sub}</div>`, 'Terminal tab', st('ok', 'encrypted'), btn('Rotate', { act: 'rot', data: { id: o.id }, kind: 'link' })]))) : ''}`;
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  const reload = async () => {
    d = await api.getAccess();
    try { sec = await api.getSecurity(); } catch { sec = null; }
    paint();
  };
  return {
    nav: 'access', title: 'Access',
    html: html`<main>${header('Access', 'Your profile, who can log in, and with what', html`<a class="btn" href="#/security">← Security</a>`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      bindActions(root, {
        savedn: async () => {
          const err = root.querySelector('#profile-err');
          try {
            const dn = root.querySelector('#dn').value;
            const r = await api.updateUser(me, { displayName: dn });
            server.displayName = r.displayName || null;
            toast('Display name saved.');
            const { accountMenu } = await import('../shell.js');
            const acct = document.querySelector('.acct');
            if (acct) acct.outerHTML = accountMenu().toString();
            await reload();
          } catch (e) { if (err) err.textContent = e.message; }
        },
        editdn: async (el) => {
          const { askForm } = await import('../ui.js');
          const ok = await askForm({
            title: `Display name for ${el.dataset.n}`,
            fields: [{ name: 'd', label: 'Display name', value: el.dataset.d || '', hint: 'At most 64 characters. Leave blank to clear.' }],
            submit: 'Save',
          }, async (o) => { await api.updateUser(el.dataset.n, { displayName: o.d }); return true; });
          if (ok) { toast('Saved.'); await reload(); }
        },
        invite: async () => {
          const { askForm, confirmDialog: ask } = await import('../ui.js');
          let made;
          const ok = await askForm({
            title: 'Invite user',
            fields: [
              { name: 'n', label: 'Username' },
              { name: 'd', label: 'Display name (optional)', hint: 'Shown in the account menu. They can change it later.' },
              { name: 'r', label: 'Role', options: ['viewer', 'deployer', 'admin'], value: 'viewer', hint: 'viewer: read only · deployer: apps and logs, no terminal · admin: everything' },
            ],
            submit: 'Invite',
          }, async (o) => { made = await api.inviteUser(o.n.trim(), o.r, o.d?.trim() || undefined); return true; });
          if (ok && made) { await ask({ title: `Invitation for ${made.name || 'the new user'}`, body: `Send them this link. It works once and for three days: ${made.link}`, confirm: 'I have copied it' }); toast('Invitation created.'); await reload(); }
        },
        rmuser: async (el) => { if (await confirmDialog({ title: `Remove ${el.dataset.n}?`, body: 'They are signed out at once and can no longer sign in.', confirm: 'Remove', danger: true })) { await api.removeUser(el.dataset.n); toast('Removed.'); await reload(); } },
        ssh: async () => { if (await confirmDialog({ title: 'Open SSH for one hour?', body: 'Port 22 opens to your current IP only and closes by itself.', confirm: 'Open SSH' })) { await api.openSsh(); toast('Port 22 is open to your IP for one hour.'); await reload(); } },
        addkey: async () => { try { await api.addKey(root.querySelector('#ku').value, root.querySelector('#kp').value); toast('Key added.'); await reload(); } catch (e) { root.querySelector('#err').textContent = e.message; } },
        rmkey: async (el) => { if (await confirmDialog({ title: 'Remove this key?', body: 'Whoever holds it can no longer log in.', confirm: 'Remove', danger: true })) { await api.removeKey(el.dataset.id); await reload(); } },
        lock: async (el) => { const lock = el.dataset.l === '1'; if (lock && !await confirmDialog({ title: `Lock ${el.dataset.n}?`, body: 'All logins and keys for this user stop working.', confirm: 'Lock', danger: true })) return; await api.lockUser(el.dataset.n, lock); await reload(); },
        rot: async (el) => { await api.rotateOutKey(el.dataset.id); toast('New key generated. Add it to the remote server.'); },
      });
    },
  };
}
