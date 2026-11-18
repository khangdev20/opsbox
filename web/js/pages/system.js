import { html, raw, clock, st, btn, tabs, table, section, header, sw, bindActions, confirmDialog, askForm, toast, patch, skeletonSection, empty, refreshing, announce } from '../ui.js';
import * as api from '../api.js';

const keyed = (cells, key, sig = '') => Object.assign(cells, { meta: { key, sig } });
const PRIO = ['emerg', 'alert', 'crit', 'error', 'warning', 'notice', 'info'];
export async function view() {
  let tab = 0, root, jq = { unit: '', prio: 6, q: '' }, live = false, stopLive = null;
  const panes = [
    async () => { const u = await api.getUpdates(); return html`<div class="row spread mb16"><p>${u.total} updates available · ${u.security} security · running kernel ${u.running} · checked ${u.checked}</p><span class="row">${btn('Install security updates', { act: 'inst', data: { w: 'security' }, kind: 'primary', disabled: !u.security })}${btn('Install all', { act: 'inst', data: { w: 'all' }, disabled: !u.list.length })}${btn('Install selected', { act: 'inst', data: { w: 'selected' }, disabled: !u.list.length })}</span></div>
      ${u.rebootAt ? html`<div class="callout fail"><span class="tag">▲</span><span>A reboot is scheduled for ${clock(u.rebootAt * 1000).slice(0, 5)}. Apps stop for about a minute.</span>${btn('Cancel reboot', { act: 'cancelreboot' })}</div>` : ''}
      ${u.reboot ? html`<div class="callout"><span class="tag">▲</span><span>A new kernel is installed. Reboot to use it.</span>${btn('Reboot now', { act: 'reboot', kind: 'danger' })}</div>` : ''}
      ${u.list.length ? table('Pending updates', [['Select'], ['Package'], ['Installed'], ['Available'], ['Type']], u.list.map((p) => keyed([html`<input type="checkbox" name="package" value="${p.pkg}" aria-label="Select ${p.pkg}">`, html`<span class="mono">${p.pkg}</span>`, html`<span class="mono muted">${p.cur}</span>`, html`<span class="mono">${p.next}</span>`, p.sec ? st('warn', 'security') : 'regular'], `p-${p.pkg}`, p.next))) : empty('Up to date. Nothing to install.')}
      <div class="row mt16"><span>Install security updates automatically</span>${sw('Automatic security updates', u.auto, 'autoup')}</div>`; },
    async () => { const u = await api.getUnits(); return table('Services', [['Unit'], ['State'], ['Boot'], ['Mem', 'r'], ['', 'r']], u.map((x) => keyed([html`<span class="mono">${x.name}</span>`, x.state === 'active' ? st('ok', 'active') : x.state === 'failed' ? st('fail', 'failed') : st('off', 'inactive'), x.boot, x.mem ? `${x.mem} MB` : '–',
      html`<span class="row end">${x.state === 'active' ? html`${btn('Restart', { act: 'unit', data: { n: x.name, a: 'restart' }, kind: 'link' })}${btn('Stop', { act: 'unit', data: { n: x.name, a: 'stop' }, kind: 'link' })}` : btn('Start', { act: 'unit', data: { n: x.name, a: 'start' }, kind: 'link' })}${btn('Logs', { act: 'unitlog', data: { n: x.name }, kind: 'link' })}</span>`], `u-${x.name}`, x.state))); },
    async () => { const j = await api.getJournal(jq); return html`<div class="row mb12"><input class="input grow" id="jq" placeholder="Filter text" value="${jq.q}" aria-label="Filter log text"><select class="select" id="jp" aria-label="Minimum priority">${PRIO.map((p, i) => html`<option value="${i}"${i === jq.prio ? ' selected' : ''}>${p}</option>`)}</select>${jq.unit ? btn(`unit: ${jq.unit} ✕`, { act: 'clearunit' }) : ''}${btn(live ? 'Live ●' : 'Live', { act: 'live', kind: live ? 'primary' : '' })}</div>
      <pre class="code" id="jlog" aria-label="Journal">${j.length ? j.map((l) => `${clock(l.t)} ${l.proc.padEnd(14)} ${l.msg}`).join('\n') : 'No entries.'}</pre>`; },
    async () => { const j = await api.getJobs(); return html`${table('Scheduled jobs', [['Schedule'], ['Job'], ['Last run'], ['', 'r']], j.map((x) => keyed([html`<span class="mono">${x.cron}</span>`, html`${x.name}${x.command ? html`<br><span class="mono muted sm">${x.command}</span>` : ''}`, x.last, html`<span class="row end">${btn('Run now', { act: 'job', data: { id: x.id }, kind: 'link' })}${btn('Remove', { act: 'rmjob', data: { id: x.id }, kind: 'link' })}</span>`], `j-${x.id}`, x.last)))}<div class="mt12">${btn('Add job', { act: 'addjob' })}</div>`; },
    async () => { const f = await api.getFirewall(); return html`${f.sshOpen ? html`<div class="callout fail"><span class="tag">▲</span><span>Emergency SSH is open for your IP. It closes automatically within one hour.</span></div>` : ''}
      ${table('Firewall rules', [['Port'], ['Proto'], ['From'], ['Note'], ['', 'r']], f.rules.map((r) => keyed([html`<span class="mono">${r.port}</span>`, r.proto, html`<span class="mono">${r.src}</span>`, r.note, btn('Remove', { act: 'rmrule', data: { id: r.id }, kind: 'link' })], `f-${r.id}`)))}
      <div class="row mt12">${btn('Add rule', { act: 'addrule' })}${btn('Open emergency SSH for 1 hour', { act: 'ssh' })}</div>`; },
  ];
  const followJournal = () => {
    const log = root.querySelector('#jlog'); if (!log) return;
    let first = true;
    stopLive = api.streamJournal(jq, (entries) => {
      const el = root.querySelector('#jlog'); if (!el) return;
      const lines = entries.map((l) => `${clock(l.t)} ${l.proc.padEnd(14)} ${l.msg}`);
      const keep = first ? [] : el.textContent.split('\n').filter(Boolean); first = false;
      el.textContent = keep.concat(lines).slice(-500).join('\n') || 'No entries.'; el.scrollTop = el.scrollHeight;
    });
  };
  // `fresh`: a different tab, so nothing enters or flashes; refreshes after an action animate in place.
  const paint = async (fresh = false) => {
    if (stopLive) { stopLive(); stopLive = null; }
    patch(root.querySelector('#t'), tabs(['Updates', 'Services', 'Logs', 'Jobs', 'Firewall'], tab).toString());
    const mine = tab;
    const h = (await panes[tab]()).toString();
    if (mine !== tab) return; // the person already moved to another tab
    patch(root.querySelector('#b'), h, fresh ? { animate: false } : undefined);
    if (tab === 2 && live) followJournal();
  };
  const done = (m) => async () => { toast(m); await paint(); };
  return {
    nav: 'system', title: 'System',
    html: html`<main>${header('System', 'Updates, services, logs, jobs and firewall.', btn('Reboot…', { act: 'reboot' }))}<div id="t"></div><div id="b" style="padding-top:16px">${skeletonSection(4)}</div></main>`,
    async mount(r) {
      root = r; await paint(true);
      root.addEventListener('input', async (e) => { if (e.target.id === 'jq') { jq.q = e.target.value; const pos = e.target.selectionStart; await paint(); const i = root.querySelector('#jq'); i.focus(); i.setSelectionRange(pos, pos); } });
      root.addEventListener('change', async (e) => { if (e.target.id === 'jp') { jq.prio = +e.target.value; await paint(); } });
      bindActions(root, {
        tab: async (el) => { tab = +el.dataset.i; await paint(true); },
        inst: async (el) => { const selected = [...root.querySelectorAll('input[name=package]:checked')].map((x) => x.value); if (el.dataset.w === 'selected' && !selected.length) { toast('Select at least one package.', 'err'); return; } const r = await api.installUpdates(el.dataset.w, selected); toast(`Installed ${r.installed} updates.`); await paint(); },
        autoup: async (el) => { await api.setAutoUpdates(el.getAttribute('aria-checked') !== 'true'); await paint(); },
        reboot: async () => { if (await confirmDialog({ title: 'Reboot this instance?', body: 'The machine restarts in one minute (until then you can cancel). All apps stop for about a minute and come back by themselves.', confirm: 'Reboot', danger: true })) { await api.reboot(); toast('Reboot scheduled in one minute. You can cancel it here until then.'); await paint(); } },
        cancelreboot: async () => { await api.cancelReboot(); toast('Reboot cancelled.'); await paint(); },
        unit: async (el) => { if (el.dataset.a === 'stop' && !await confirmDialog({ title: `Stop ${el.dataset.n}?`, body: 'Anything that depends on it may stop working.', confirm: 'Stop', danger: true })) return; await api.unitAction(el.dataset.n, el.dataset.a); await done(`${el.dataset.a} ${el.dataset.n}`)(); },
        unitlog: async (el) => { jq = { unit: el.dataset.n, prio: 6, q: '' }; tab = 2; await paint(true); },
        clearunit: async () => { jq.unit = ''; await paint(); },
        live: async () => { live = !live; await paint(); },
        job: async (el) => { await api.runJob(el.dataset.id); await done('Job finished.')(); },
        rmjob: async (el) => { if (await confirmDialog({ title: 'Remove this job?', body: 'It stops running on its schedule.', confirm: 'Remove', danger: true })) { await api.removeJob(el.dataset.id); await done('Job removed.')(); } },
        addjob: async () => { if (await askForm({ title: 'Add job', fields: [{ name: 'cron', label: 'Schedule', placeholder: '0 4 * * *', hint: 'Five fields: minute hour day-of-month month day-of-week' }, { name: 'name', label: 'Description', placeholder: 'Back up the database' }, { name: 'command', label: 'Command', placeholder: 'pg_dump app > /var/backups/app.sql', hint: 'One line, run as root through sh.' }], submit: 'Add' }, (o) => api.addJob(o))) await done('Job added.')(); },
        rmrule: async (el) => { if (await confirmDialog({ title: 'Remove this rule?', body: 'Traffic on this port will be blocked.', confirm: 'Remove', danger: true })) { await api.removeRule(el.dataset.id); await paint(); } },
        addrule: async () => { if (await askForm({ title: 'Allow a port', fields: [{ name: 'port', label: 'Port', placeholder: '5432' }, { name: 'proto', label: 'Protocol', options: ['tcp', 'udp'], value: 'tcp' }, { name: 'src', label: 'From', value: 'anywhere', hint: 'anywhere or an IP / CIDR' }, { name: 'note', label: 'Note' }], submit: 'Allow' }, (o) => api.addRule(o))) await done('Rule added.')(); },
        ssh: async () => { await api.openSsh(); await done('Port 22 is open to your IP for one hour.')(); },
      });
    },
  };
}
