import { html, clock, st, btn, tabs, table, section, header, sw, bindActions, confirmDialog, askForm, toast, patch, skeletonSection, empty, refreshing } from '../ui.js';
import * as api from '../api.js';
import { capacityPage, addSwap, editMemory, bindMemoryMap } from '../capacity.js';

const TAB_IDS = ['updates', 'services', 'capacity', 'logs', 'jobs', 'firewall'];
const TAB_LABELS = ['Updates', 'Services', 'Capacity', 'Logs', 'Jobs', 'Firewall'];
const tabFromPath = (path) => {
  const m = String(path || '').match(/^\/system(?:\/([^/?#]+))?/);
  const id = (m && m[1] || '').toLowerCase();
  const i = TAB_IDS.indexOf(id);
  return i >= 0 ? i : 0;
};
const setTabHash = (i) => {
  const next = `#/system/${TAB_IDS[i]}`;
  if (location.hash !== next) history.replaceState(null, '', next);
};

const keyed = (cells, key, sig = '') => Object.assign(cells, { meta: { key, sig } });
const PRIO = ['emerg', 'alert', 'crit', 'error', 'warning', 'notice', 'info'];

export async function view({ path } = {}) {
  let tab = tabFromPath(path), root, jq = { unit: '', prio: 6, q: '' }, live = false, stopLive = null;
  let capSort = { by: 'name', dir: 1 }, lastCap = null, usageByName = {};
  const selectedPackages = new Set();

  const loadCapacity = async () => {
    const [cap, containers] = await Promise.all([
      api.getCapacity(),
      api.listContainers().catch(() => []),
    ]);
    lastCap = cap;
    usageByName = {};
    for (const c of containers || []) {
      if (c?.name != null && c.mem != null) usageByName[c.name] = c.mem;
    }
    return cap;
  };

  const panes = [
    async () => {
      const u = await api.getUpdates();
      const available = new Set(u.list.map((p) => p.pkg));
      for (const name of selectedPackages) if (!available.has(name)) selectedPackages.delete(name);
      return html`<div class="updates-summary"><div><h2>Package updates <span class="updates-count">${u.total}</span></h2><p>${u.security} security updates · Last checked ${u.checked}</p></div><div class="row">${btn('Install security updates', { act: 'inst', data: { w: 'security' }, kind: 'primary', disabled: !u.security })}${btn('Install all', { act: 'inst', data: { w: 'all' }, disabled: !u.list.length })}</div></div>
      ${u.rebootAt ? html`<div class="callout fail"><span class="tag">▲</span><span>A reboot is scheduled for ${clock(u.rebootAt * 1000).slice(0, 5)}. Apps stop for about a minute.</span>${btn('Cancel reboot', { act: 'cancelreboot' })}</div>` : ''}
      ${u.reboot ? html`<div class="callout"><span class="tag">▲</span><span>A new kernel is installed. Reboot to use it.</span>${btn('Reboot now', { act: 'reboot', kind: 'danger' })}</div>` : ''}
      <section class="updates-panel" aria-label="Pending updates">
        ${u.list.length ? html`<div class="updates-table-head"><div><h3>Available packages</h3><p id="package-selection" role="status" aria-live="polite"></p></div>${btn('Install selected', { act: 'inst', data: { w: 'selected' }, id: 'install-selected', disabled: !selectedPackages.size })}</div>
        ${table('Pending updates', [[html`<input type="checkbox" id="packages-all" aria-label="Select all packages">`, { label: false, check: true }], ['Package'], ['Installed version'], ['Available version'], ['Type']], u.list.map((p) => keyed([html`<input type="checkbox" name="package" value="${p.pkg}" aria-label="Select ${p.pkg}"${selectedPackages.has(p.pkg) ? ' checked' : ''}>`, html`<span class="mono">${p.pkg}</span>`, html`<span class="mono">${p.cur}</span>`, html`<span class="mono">${p.next}</span>`, p.sec ? st('warn', 'security') : st('off', 'regular')], `p-${p.pkg}`, p.next)), { sticky: true, cards: false })}` : empty('Everything is up to date.', btn('Check again', { act: 'retry' }))}
        <div class="updates-footer"><span>Running kernel <span class="mono">${u.running}</span></span>${u.list.length ? html`<span>${u.list.length} packages listed</span>` : ''}</div>
      </section>
      <div class="updates-auto"><div><b>Automatic security updates</b><p>Install security patches automatically to keep this server protected.</p></div>${sw('Automatic security updates', u.auto, 'autoup')}</div>`; },
    async () => { const u = await api.getUnits(); return !u.length ? empty('No services found.', btn('Refresh services', { act: 'retry' })) : table('Services', [['Unit'], ['State'], ['Boot'], ['Mem', 'r'], ['', 'r']], u.map((x) => keyed([html`<span class="mono">${x.name}</span>`, x.state === 'active' ? st('ok', 'active') : x.state === 'failed' ? st('fail', 'failed') : st('off', 'inactive'), x.boot, x.mem ? `${x.mem} MB` : '–',
      html`<span class="row end">${x.state === 'active' ? html`${btn('Restart', { act: 'unit', data: { n: x.name, a: 'restart' }, kind: 'link' })}${btn('Stop', { act: 'unit', data: { n: x.name, a: 'stop' }, kind: 'link' })}` : btn('Start', { act: 'unit', data: { n: x.name, a: 'start' }, kind: 'link' })}${btn('Logs', { act: 'unitlog', data: { n: x.name }, kind: 'link' })}</span>`], `u-${x.name}`, x.state)), { flip: true }); },
    async () => {
      const c = await loadCapacity();
      return capacityPage(c, { usageByName, sortBy: capSort.by, sortDir: capSort.dir });
    },
    async () => { const j = await api.getJournal(jq); return html`<div class="row mb12"><input class="input grow" id="jq" placeholder="Filter text" value="${jq.q}" aria-label="Filter log text"><select class="select" id="jp" aria-label="Minimum priority">${PRIO.map((p, i) => html`<option value="${i}"${i === jq.prio ? ' selected' : ''}>${p}</option>`)}</select>${jq.unit ? btn(`unit: ${jq.unit} ✕`, { act: 'clearunit' }) : ''}${btn(live ? 'Live ●' : 'Live', { act: 'live', kind: live ? 'primary' : '' })}</div>
      ${j.length || live ? html`<pre class="code" id="jlog" aria-label="Journal">${j.length ? j.map((l) => `${clock(l.t)} ${l.proc.padEnd(14)} ${l.msg}`).join('\n') : 'No entries.'}</pre>` : empty('No log entries match these filters.', btn('Clear filters', { act: 'clearfilters' }))}`; },
    async () => { const j = await api.getJobs(); return !j.length ? empty('No scheduled jobs yet. Add a job to run a command on a schedule.', btn('Add job', { act: 'addjob' })) : html`${table('Scheduled jobs', [['Schedule'], ['Job'], ['Last run'], ['', 'r']], j.map((x) => keyed([html`<span class="mono">${x.cron}</span>`, html`${x.name}${x.command ? html`<br><span class="mono muted sm">${x.command}</span>` : ''}`, x.last, html`<span class="row end">${btn('Run now', { act: 'job', data: { id: x.id }, kind: 'link' })}${btn('Remove', { act: 'rmjob', data: { id: x.id }, kind: 'link' })}</span>`], `j-${x.id}`, x.last)))}<div class="mt12">${btn('Add job', { act: 'addjob' })}</div>`; },
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
  // Rows fading out after an install no longer belong to the pending selection.
  const packageInputs = () => [...root.querySelectorAll('.updates-panel tbody tr:not(.leaving) input[name=package]')];
  const syncPackageSelection = () => {
    const boxes = packageInputs();
    selectedPackages.clear();
    boxes.forEach((box) => { if (box.checked) selectedPackages.add(box.value); });
    const n = selectedPackages.size;
    const all = root.querySelector('#packages-all');
    if (all) { all.checked = boxes.length > 0 && n === boxes.length; all.indeterminate = n > 0 && n < boxes.length; }
    const count = root.querySelector('#package-selection');
    if (count) count.textContent = `${n} of ${boxes.length} selected`;
    const install = root.querySelector('#install-selected');
    if (install) { install.disabled = n === 0; install.textContent = n ? `Install selected (${n})` : 'Install selected'; }
  };
  // `fresh`: a different tab, so nothing enters or flashes; refreshes after an action animate in place.
  const paint = async (fresh = false) => {
    if (stopLive) { stopLive(); stopLive = null; }
    setTabHash(tab);
    patch(root.querySelector('#t'), tabs(TAB_LABELS, tab).toString());
    const mine = tab;
    let h;
    try { h = (await panes[tab]()).toString(); }
    catch (e) {
      if (mine !== tab) return;
      patch(root.querySelector('#b'), html`<div role="alert">${empty(e.message || String(e), btn('Retry', { act: 'retry' }))}</div>`.toString(), { animate: false });
      return;
    }
    if (mine !== tab) return; // the person already moved to another tab
    patch(root.querySelector('#b'), h, fresh ? { animate: false } : undefined);
    if (tab === 0) syncPackageSelection();
    if (tab === 2) bindMemoryMap(root.querySelector('#b'));
    if (tab === 3 && live) followJournal();
  };
  const done = (m) => async () => { toast(m); await paint(); };
  const refreshCap = async () => {
    if (tab !== 2) return;
    const body = root.querySelector('#b');
    await refreshing(body, async () => {
      const c = await loadCapacity();
      patch(body, capacityPage(c, { usageByName, sortBy: capSort.by, sortDir: capSort.dir }).toString());
      bindMemoryMap(body);
    });
  };
  return {
    nav: 'system', title: 'System',
    html: html`<main>${header('System', 'Updates, services, capacity, logs, jobs and firewall.', btn('Reboot…', { act: 'reboot' }))}<div id="t"></div><div id="b" style="padding-top:16px">${skeletonSection(4)}</div></main>`,
    async mount(r) {
      root = r; await paint(true);
      root.addEventListener('input', async (e) => { if (e.target.id === 'jq') { jq.q = e.target.value; const pos = e.target.selectionStart; await paint(); const i = root.querySelector('#jq'); i.focus(); i.setSelectionRange(pos, pos); } });
      root.addEventListener('change', async (e) => {
        if (e.target.id === 'packages-all') {
          packageInputs().forEach((box) => { box.checked = e.target.checked; });
          syncPackageSelection();
        } else if (e.target.name === 'package') syncPackageSelection();
        else if (e.target.id === 'jp') { jq.prio = +e.target.value; await paint(); }
      });
      bindActions(root, {
        retry: () => paint(true),
        tab: async (el) => { tab = +el.dataset.i; await paint(true); },
        inst: async (el) => { const selected = packageInputs().filter((x) => x.checked).map((x) => x.value); if (el.dataset.w === 'selected' && !selected.length) { toast('Select at least one package.', 'err'); return; } const r = await api.installUpdates(el.dataset.w, selected); toast(`Installed ${r.installed} updates.`); await paint(); },
        autoup: async (el) => { await api.setAutoUpdates(el.getAttribute('aria-checked') !== 'true'); await paint(); },
        reboot: async () => { if (await confirmDialog({ title: 'Reboot this instance?', body: 'The machine restarts in one minute (until then you can cancel). All apps stop for about a minute and come back by themselves.', confirm: 'Reboot', danger: true })) { await api.reboot(); toast('Reboot scheduled in one minute. You can cancel it here until then.'); await paint(); } },
        cancelreboot: async () => { await api.cancelReboot(); toast('Reboot cancelled.'); await paint(); },
        unit: async (el) => { if (el.dataset.a === 'stop' && !await confirmDialog({ title: `Stop ${el.dataset.n}?`, body: 'Anything that depends on it may stop working.', confirm: 'Stop', danger: true })) return; await api.unitAction(el.dataset.n, el.dataset.a); await done(`${el.dataset.a} ${el.dataset.n}`)(); },
        unitlog: async (el) => { jq = { unit: el.dataset.n, prio: 6, q: '' }; tab = 3; await paint(true); },
        clearunit: async () => { jq.unit = ''; await paint(); },
        clearfilters: async () => { jq = { unit: '', prio: 6, q: '' }; await paint(); },
        live: async () => { live = !live; await paint(); },
        job: async (el) => { await api.runJob(el.dataset.id); await done('Job finished.')(); },
        rmjob: async (el) => { if (await confirmDialog({ title: 'Remove this job?', body: 'It stops running on its schedule.', confirm: 'Remove', danger: true })) { await api.removeJob(el.dataset.id); await done('Job removed.')(); } },
        addjob: async () => { if (await askForm({ title: 'Add job', fields: [{ name: 'cron', label: 'Schedule', placeholder: '0 4 * * *', hint: 'Five fields: minute hour day-of-month month day-of-week' }, { name: 'name', label: 'Description', placeholder: 'Back up the database' }, { name: 'command', label: 'Command', placeholder: 'pg_dump app > /var/backups/app.sql', hint: 'One line, run as root through sh.' }], submit: 'Add' }, (o) => api.addJob(o))) await done('Job added.')(); },
        rmrule: async (el) => { if (await confirmDialog({ title: 'Remove this rule?', body: 'Traffic on this port will be blocked.', confirm: 'Remove', danger: true })) { await api.removeRule(el.dataset.id); await paint(); } },
        addrule: async () => { if (await askForm({ title: 'Allow a port', fields: [{ name: 'port', label: 'Port', placeholder: '5432' }, { name: 'proto', label: 'Protocol', options: ['tcp', 'udp'], value: 'tcp' }, { name: 'src', label: 'From', value: 'anywhere', hint: 'anywhere or an IP / CIDR' }, { name: 'note', label: 'Note' }], submit: 'Allow' }, (o) => api.addRule(o))) await done('Rule added.')(); },
        ssh: async () => { await api.openSsh(); await done('Port 22 is open to your IP for one hour.')(); },
        'capacity-swap': async (el) => { await addSwap(Number(el.dataset.mb)); await refreshCap(); },
        'capacity-swap-offer': async () => {
          const mb = lastCap?.swap_offer_mb;
          if (!mb) { toast('This server already has swap, or adding one is not offered.', 'warn'); return; }
          await addSwap(mb); await refreshCap();
        },
        'cap-scroll-res': () => { root.querySelector('#cap-reservations')?.scrollIntoView({ behavior: 'smooth', block: 'start' }); },
        'cap-sort': async (el) => {
          const by = el.dataset.by;
          if (capSort.by === by) capSort.dir *= -1; else { capSort.by = by; capSort.dir = 1; }
          await refreshCap();
        },
        'cap-edit': async (el) => {
          await editMemory({ id: el.dataset.name, name: el.dataset.name, kind: el.dataset.db === '1' ? 'db' : 'app', memoryLimitMb: Number(el.dataset.mb) || 128 });
          await refreshCap();
        },
      });
    },
  };
}
