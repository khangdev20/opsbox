import { html, st, chk, btn, table, section, header, bindActions, confirmDialog, askForm, toast, clock, download } from '../ui.js';
import * as api from '../api.js';

const emptyFilters = () => ({ actor: '', action: '', result: '', from: '', to: '' });

/** `datetime-local` value → RFC 3339 UTC for the API. */
function toApiTime(v) {
  if (!v) return undefined;
  if (/Z$|[+-]\d{2}:\d{2}$/.test(v)) return v;
  const d = new Date(v);
  return Number.isNaN(d.getTime()) ? v : d.toISOString();
}

export async function view() {
  let d, audit = { entries: [], nextCursor: null }, filters = emptyFilters(), root;
  const auditFilters = () => ({
    actor: filters.actor.trim() || undefined,
    action: filters.action.trim() || undefined,
    result: filters.result || undefined,
    from: toApiTime(filters.from.trim()),
    to: toApiTime(filters.to.trim()),
  });
  const readFilters = () => {
    if (!root) return;
    filters = {
      actor: root.querySelector('#af-actor')?.value || '',
      action: root.querySelector('#af-action')?.value || '',
      result: root.querySelector('#af-result')?.value || '',
      from: root.querySelector('#af-from')?.value || '',
      to: root.querySelector('#af-to')?.value || '',
    };
  };
  const auditSection = () => {
    const rows = (audit.entries || []).map((a) => [
      clock(a.t),
      html`<span class="mono">${a.user}</span>`,
      html`<span class="mono">${api.formatAuditIp(a.ip)}</span>`,
      a.warn ? st('warn', a.action) : a.action,
    ]);
    const filtersUi = html`<div class="row gap12" style="align-items:flex-end">
      <div class="field grow" style="min-width:120px;margin:0"><label for="af-actor">Actor</label><input class="input" id="af-actor" value="${filters.actor}" placeholder="username" autocomplete="off"></div>
      <div class="field grow" style="min-width:120px;margin:0"><label for="af-action">Action</label><input class="input mono" id="af-action" value="${filters.action}" placeholder="auth." autocomplete="off"></div>
      <div class="field" style="min-width:110px;margin:0"><label for="af-result">Result</label><select class="input" id="af-result"><option value=""${filters.result === '' ? ' selected' : ''}>Any</option><option value="ok"${filters.result === 'ok' ? ' selected' : ''}>ok</option><option value="denied"${filters.result === 'denied' ? ' selected' : ''}>denied</option><option value="failed"${filters.result === 'failed' ? ' selected' : ''}>failed</option></select></div>
      <div class="field" style="min-width:150px;margin:0"><label for="af-from">From</label><input class="input mono" id="af-from" type="datetime-local" value="${filters.from}"></div>
      <div class="field" style="min-width:150px;margin:0"><label for="af-to">To</label><input class="input mono" id="af-to" type="datetime-local" value="${filters.to}"></div>
      <div class="row" style="margin:0">${btn('Apply', { act: 'filter' })}${btn('Export CSV', { act: 'export' })}</div>
    </div>
    <p class="sm muted mt6">Loopback addresses show as local; the reverse proxy hop shows as panel.</p>`;
    return section('Audit log', html`${filtersUi}${table('Audit log', [['Time'], ['User'], ['IP'], ['Action']], rows, { sticky: true, cards: false })}`);
  };
  const body = () => html`<div class="cols"><div class="w2">${section(`Checks · ${d.passing} of ${d.total} passing`, d.checks.map((c) => html`<div class="list-row"><span>${chk(c.ok ? 'ok' : 'warn', c.t)}</span>${c.action && c.id !== 'sshd' ? btn(c.action, { act: 'enable', data: { id: c.id } }) : c.id === 'sshd' ? btn(d.checks.find((x) => x.id === 'sshd').warn ? 'Open now' : c.action, { act: 'ssh' }) : c.link && !c.ok ? btn('Open', { href: c.link }) : ''}</div>`))}
      ${section('Allowed IPs', html`<div class="row"><input class="input grow mono" id="ips" value="${d.allowIps}" aria-label="Allowed IP addresses"><button type="button" class="btn" data-act="saveips">Save</button></div><p class="sm muted mt6">Comma-separated IPs or CIDR ranges.</p>`)}</div>
    <div>${section('Users', html`${table('Users', [['User'], ['Display name'], ['Role'], ['2FA'], ['', 'r']], d.users.map((u) => [html`<span class="mono">${u.name}</span>${u.you ? html` <span class="sm muted">(you)</span>` : ''}`, u.displayName || '—', u.role, u.pending ? st('warn', 'invited') : u.tfa ? st('ok', 'on') : st('warn', 'off'), u.you ? '' : (api.isReal('removeUser') ? btn('Remove', { act: 'rmuser', data: { n: u.name }, kind: 'link' }) : '')]))}<div class="row mt12">${btn('Invite user', { act: 'invite' })}${btn('Sign out all sessions', { act: 'signout' })}</div>`)}
      ${auditSection()}</div></div>`;
  const loadAudit = async () => {
    try {
      const res = await api.getAudit(auditFilters());
      audit = Array.isArray(res) ? { entries: res, nextCursor: null } : res;
    } catch {
      audit = { entries: d?.audit || [], nextCursor: null };
    }
  };
  const paint = async () => {
    readFilters();
    d = await api.getSecurity();
    await loadAudit();
    root.querySelector('#b').innerHTML = body().toString();
  };
  d = await api.getSecurity();
  await loadAudit();
  return {
    nav: 'security', title: 'Security',
    html: html`<main>${header('Security', `${d.passing} of ${d.total} checks passing`, html`<a class="btn" href="#/security/access">Users and SSH keys →</a>`)}<div id="b">${body()}</div></main>`,
    mount(r) {
      root = r;
      bindActions(root, {
        enable: async (el) => { await api.toggleCheck(el.dataset.id); toast('Enabled.'); await paint(); },
        ssh: async () => { if (await confirmDialog({ title: 'Open SSH for one hour?', body: 'Port 22 opens to your current IP only and closes by itself.', confirm: 'Open SSH' })) { await api.openSsh(); toast('Port 22 is open to your IP for one hour.'); await paint(); } },
        saveips: async () => { await api.setAllowIps(root.querySelector('#ips').value); toast('Saved.'); await paint(); },
        invite: async () => {
          let made;
          const ok = await askForm({ title: 'Invite user', fields: [{ name: 'n', label: 'Username' }, { name: 'd', label: 'Display name (optional)', hint: 'Shown in the account menu. They can change it later.' }, { name: 'r', label: 'Role', options: ['viewer', 'deployer', 'admin'], value: 'viewer', hint: 'viewer: read only · deployer: apps and logs, no terminal · admin: everything' }], submit: 'Invite' }, async (o) => { made = await api.inviteUser(o.n.trim(), o.r, o.d?.trim() || undefined); return true; });
          if (ok && made) { await confirmDialog({ title: `Invitation for ${made.name || 'the new user'}`, body: `Send them this link. It works once and for three days: ${made.link}`, confirm: 'I have copied it' }); toast('Invitation created.'); await paint(); }
        },
        rmuser: async (el) => { if (await confirmDialog({ title: `Remove ${el.dataset.n}?`, body: 'They are signed out at once and can no longer sign in.', confirm: 'Remove', danger: true })) { await api.removeUser(el.dataset.n); toast('Removed.'); await paint(); } },
        signout: async () => { if (await confirmDialog({ title: 'Sign out everyone?', body: 'All sessions end, including yours.', confirm: 'Sign out all', danger: true })) { await api.signOutAll(); const { session } = await import('../store.js'); session.clear(); location.hash = '#/login'; } },
        filter: async () => {
          readFilters();
          await loadAudit();
          root.querySelector('#b').innerHTML = body().toString();
        },
        export: async () => {
          readFilters();
          try {
            const file = await api.exportAuditCsv(auditFilters());
            download(file.filename, file.text, 'text/csv');
            toast('Audit log exported.');
          } catch (e) {
            toast(e.message || 'Export failed.', 'err');
          }
        },
      });
    },
  };
}
