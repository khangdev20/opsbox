import { html, ago, st, btn, table, section, header, bindActions, confirmDialog, askForm, toast, clock } from '../ui.js';
import * as api from '../api.js';

export async function view() {
  let d, root;
  const body = () => html`<div class="cols"><div class="w2">${section(`Checks · ${d.passing} of ${d.total} passing`, d.checks.map((c) => html`<div class="list-row"><span>${c.ok ? st('ok', c.t) : st('warn', c.t)}</span>${c.action && c.id !== 'sshd' ? btn(c.action, { act: 'enable', data: { id: c.id } }) : c.id === 'sshd' ? btn(d.checks.find((x) => x.id === 'sshd').warn ? 'Open now' : c.action, { act: 'ssh' }) : ''}</div>`))}
      ${section('Allowed IPs', html`<div class="row"><input class="input grow mono" id="ips" value="${d.allowIps}" aria-label="Allowed IP addresses"><button type="button" class="btn" data-act="saveips">Save</button></div><p class="sm muted" style="margin-top:6px">Comma-separated IPs or CIDR ranges.</p>`)}</div>
    <div>${section('Users', html`${table('Users', [['User'], ['Role'], ['2FA']], d.users.map((u) => [html`<span class="mono">${u.name}</span>`, u.role, u.tfa ? st('ok', 'on') : st('warn', 'off')]))}<div class="row" style="margin-top:12px">${btn('Invite user', { act: 'invite' })}${btn('Sign out all sessions', { act: 'signout' })}</div>`)}
      ${section('Audit log', table('Audit log', [['Time'], ['User'], ['Action']], d.audit.map((a) => { const r = [clock(a.t), html`<span class="mono">${a.user}</span>`, a.warn ? st('warn', a.action) : a.action]; return r; })))}</div></div>`;
  const paint = async () => { d = await api.getSecurity(); root.querySelector('#b').innerHTML = body().toString(); };
  d = await api.getSecurity();
  return {
    nav: 'security', title: 'Security',
    html: html`<main>${header('Security', `${d.passing} of ${d.total} checks passing`, html`<a class="btn" href="#/security/access">Users and SSH keys →</a>`)}<div id="b">${body()}</div></main>`,
    mount(r) {
      root = r;
      bindActions(root, {
        enable: async (el) => { await api.toggleCheck(el.dataset.id); toast('Enabled.'); await paint(); },
        ssh: async () => { if (await confirmDialog({ title: 'Open SSH for one hour?', body: 'Port 22 opens to your current IP only and closes by itself.', confirm: 'Open SSH' })) { await api.openSsh(); toast('Port 22 is open to your IP for one hour.'); await paint(); } },
        saveips: async () => { await api.setAllowIps(root.querySelector('#ips').value); toast('Saved.'); await paint(); },
        invite: async () => { if (await askForm({ title: 'Invite user', fields: [{ name: 'n', label: 'Username' }], submit: 'Invite' }, (o) => api.inviteUser(o.n.trim()))) { toast('Invitation created.'); await paint(); } },
        signout: async () => { if (await confirmDialog({ title: 'Sign out everyone?', body: 'All sessions end, including yours.', confirm: 'Sign out all', danger: true })) { await api.signOutAll(); const { session } = await import('../store.js'); session.clear(); location.hash = '#/login'; } },
      });
    },
  };
}
