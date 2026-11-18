// UI kit: every component in every state, dark and light side by side, plus a simulator
// that drives the live-data helpers (tween, flash, list diffing, chart append, tasks).
import { html, raw, st, btn, sw, tabs, table, section, empty, sk, skeletonSection, bar, segBar, num, numberField, dropzone, chk,
  toast, confirmDialog, formDialog, keepOrUndoDialog, bindActions, flash, patch, createChart, trackTask, setSwitch, announce,
  initUi, reportPoll, liveState, logo, iconAct } from './ui.js';
import { accountMenu, navMarkup, notificationBell } from './shell.js';
import { procRow } from './pages/overview.js';
import { memoryMap, featureTable, capacityFooter, fitBadge, featureLabel, FEATURE_LABELS } from './capacity.js';
import { applyTheme, initialTheme, toggleTheme } from './store.js';

const KIT_CAP = {
  profile: { memory_mb: 1024, swap_mb: 2048, cpu_millis: 1000 },
  allocatable: { os_mb: 192, opsbox_mb: 128, docker_mb: 150, reserved_mb: 384, allocatable_mb: 170, database_allocatable_mb: 170, swap_credit_mb: 914, allocatable_cpu_millis: 650, pressure: false },
  reservations: [
    { name: 'web', memory_mb: 128, cpu_millis: 100, database: false },
    { name: 'api', memory_mb: 128, cpu_millis: 100, database: false },
    { name: 'worker', memory_mb: 128, cpu_millis: 100, database: false },
  ],
  features: Object.keys(FEATURE_LABELS).map((feature) => ({
    feature,
    minimum_mb: feature === 'build' ? 1536 : feature === 'database' ? 256 : 128,
    recommended_mb: feature === 'build' ? 2048 : 256,
    disk_mb: feature === 'build' ? 5120 : 0,
    fit: feature === 'build' || feature === 'database'
      ? { status: 'blocked', reasons: [`Needs more memory for ${featureLabel(feature)}.`] }
      : feature === 'template' || feature === 'backup'
        ? { status: 'tight', reasons: ['Fits, but tight.'] }
        : { status: 'ok' },
  })),
  pressure: false,
  swap_offer_mb: 1024,
};
const KIT_CAP_WARN = { ...KIT_CAP, pressure: true, allocatable: { ...KIT_CAP.allocatable, pressure: true, allocatable_mb: 64 } };

applyTheme(initialTheme());
initUi();

const STATES = ['default', 'hover', 'focus', 'active', 'disabled', 'loading'];
const cls = (s) => (s === 'hover' ? ' is-hover' : s === 'focus' ? ' is-focus' : s === 'active' ? ' is-active' : '');
const cell = (label, inner) => html`<div class="kit-cell"><span class="kit-l">${label}</span>${inner}</div>`;
const sub = (title, body, note = '') => html`<section class="sec kit-sec"><div class="sec-h"><h2>${title}</h2></div>${note ? html`<p class="muted sm">${note}</p>` : ''}${body}</section>`;

// ----- live simulation state -----
const sim = { on: false, timer: null, n: 0, seq: 3, cpu: 34, mem: 2.6, disk: 61, net: 3.2, conns: 120,
  mem48: Array(48).fill(55), cpu48: Array(48).fill(30), net48: Array(48).fill(2),
  rows: [{ id: 'web', state: 'running', cpu: 4.2, mem: 180 }, { id: 'api', state: 'running', cpu: 11.5, mem: 320 }, { id: 'worker', state: 'restarting', cpu: 0, mem: 0 }],
  shuffleSeq: 5,
  shuffle: [
    { id: 'r1', name: 'postgres', state: 'ok', v: '12.4%' },
    { id: 'r2', name: 'nginx', state: 'ok', v: '8.1%' },
    { id: 'r3', name: 'node', state: 'warn', v: '6.3%' },
    { id: 'r4', name: 'redis', state: 'ok', v: '2.0%' },
    { id: 'r5', name: 'opsbox', state: 'ok', v: '0.4%' },
  ] };
const walk = (v, s, lo, hi) => Math.min(hi, Math.max(lo, v + (Math.random() - 0.5) * s));
const kpi = (l, v, u, r, th = true) => html`<div class="kpi">${html`<span class="l">${l}</span><span class="v">${v}<small>${u}</small></span>`}${bar(r, l, { thresholds: th })}</div>`;
const liveKpis = () => html`<div class="kpis kit-kpis">${kpi('CPU', num(sim.cpu, 'int'), '%', sim.cpu / 100)}${kpi('Memory', num(sim.mem, 'dec1'), ' / 4 GB', sim.mem / 4)}${kpi('Disk /', num(sim.disk, 'int'), ' / 100 GB', sim.disk / 100)}${kpi('Network', num(sim.net, 'dec1'), ' MB/s', Math.min(1, sim.net / 12), false)}</div>`;
const rowState = (r) => (r.state === 'running' ? st('ok', 'running') : r.state === 'restarting' ? st('warn', 'restarting') : r.state === 'building' ? st('build', 'building') : st('off', 'exited'));
const liveRows = () => table('Simulated services', [['Service'], ['State'], ['CPU', 'r'], ['Mem', 'r']], sim.rows.map((r) => Object.assign([html`<span class="mono">${r.id}</span>`, rowState(r), r.state === 'running' ? num(r.cpu, 'pct1') : '–', r.state === 'running' ? num(r.mem, 'mb') : '–'], { meta: { key: r.id, sig: r.state } })));
const stateTick = () => {
  sim.n++;
  sim.cpu = walk(sim.cpu, 22, 4, 96); sim.mem = walk(sim.mem, .5, 1.2, 3.9); sim.disk = Math.min(98, sim.disk + (Math.random() < .2 ? 6 : 0)); if (sim.disk > 90) sim.disk = 61;
  sim.net = sim.n % 14 === 0 ? 11 : walk(sim.net, 1.6, .3, 6); sim.conns = walk(sim.conns, 20, 40, 300);
  const push = (a, v) => { a.push(v); a.shift(); };
  push(sim.cpu48, Math.round(sim.cpu)); push(sim.mem48, Math.round(sim.mem / 4 * 100)); push(sim.net48, +sim.net.toFixed(1));
  sim.rows.forEach((r) => { if (r.state === 'building') r.state = 'running'; if (r.state === 'running') { r.cpu = walk(r.cpu, 5, .1, 60); r.mem = walk(r.mem || 200, 40, 60, 900); } });
  const x = Math.random();
  if (x < .22 && sim.rows.length < 8) sim.rows.splice(Math.floor(Math.random() * (sim.rows.length + 1)), 0, { id: `svc-${++sim.seq}`, state: 'building', cpu: 0, mem: 0 });
  else if (x < .38 && sim.rows.length > 3) sim.rows.splice(Math.floor(Math.random() * sim.rows.length), 1);
  else if (x < .7) { const r = sim.rows[Math.floor(Math.random() * sim.rows.length)]; r.state = r.state === 'running' ? (Math.random() < .5 ? 'restarting' : 'exited') : 'running'; if (r.state === 'running') { r.cpu = 3; r.mem = 150; } }
};
const charts = [];
const shuffleRows = () => sim.shuffle.map((r) => html`<div class="list-row mrow" data-key="${r.id}" data-sig="${r.v}"><span class="mono">${r.name}</span>${st(r.state, r.v)}</div>`).join('');
const paintShuffle = () => {
  document.querySelectorAll('[data-kit-flip]').forEach((h) => patch(h, shuffleRows()));
  document.querySelectorAll('[data-kit-plain]').forEach((h) => patch(h, shuffleRows()));
};
const paintLive = () => {
  document.querySelectorAll('[data-live=kpis]').forEach((h) => patch(h, liveKpis().toString()));
  document.querySelectorAll('[data-live=rows]').forEach((h) => patch(h, liveRows().toString()));
  charts.forEach((c) => c.update(c.kind === 'pct' ? [sim.mem48, sim.cpu48] : [sim.net48]));
};
let taskSeq = 0;
function fakeTask(fail = false) {
  const id = `kit-${++taskSeq}`, determinate = taskSeq % 2 === 1;
  const steps = ['Pulling the image', 'Building', 'Starting the container', 'Checking health'];
  let i = 0;
  const label = fail ? 'Fake failing task' : determinate ? 'Fake deploy (with percent)' : 'Fake backup (no percent)';
  const emit = (status = 'running', extra = {}) => trackTask({ id, label, status, force: true, steps: steps.map((s, k) => ({ label: s, state: status === 'ok' || k < i ? 'done' : k === i ? 'run' : 'wait' })), ...(determinate ? { progress: Math.min(1, i / steps.length) } : {}), ...extra });
  emit();
  const t = setInterval(() => {
    i++;
    if (fail && i === 2) { clearInterval(t); emit('failed', { error: 'The build ran out of memory. Try a smaller build or add swap.' }); return; }
    if (i >= steps.length) { clearInterval(t); emit('ok'); return; }
    emit();
  }, 1100);
}

const DEMO_NOTIFS = {
  items: [
    { id: 'disk:/', kind: 'disk', severity: 'critical', since: Math.floor(Date.now() / 1000) - 180, text: 'Disk / is at 96%.', link: ['Clean up →', '#/cleanup'], read: false, snoozed: false },
    { id: 'cpu:host', kind: 'cpu', severity: 'critical', since: Math.floor(Date.now() / 1000) - 900, text: 'CPU is at 99%. Review busy processes.', link: ['View alert →', '#/alerts?id=cpu%3Ahost'], read: false, snoozed: true },
    { id: 'backup:opsbox', kind: 'backup', severity: 'warning', since: Math.floor(Date.now() / 1000) - 3600, text: 'Scheduled backup missed for opsbox (daily 02:00)', link: ['View backups →', '#/backups'], read: false, snoozed: false },
    { id: 'update:0.2.0', kind: 'update', severity: 'info', since: Math.floor(Date.now() / 1000) - 7200, text: 'opsbox 0.2.0 is available', link: ['Review in Settings →', '#/settings'], read: true, snoozed: false },
  ],
  unread: 3,
  critical: 2,
};
const pane = (theme, s) => {
  const btnRow = (label, kind) => html`<div class="kit-row"><span class="kit-l">${label}</span>${STATES.map((x) => cell(x, html`<button type="button" class="btn ${kind}${cls(x)}${x === 'loading' ? ' busy' : ''}"${raw(x === 'disabled' ? ' disabled' : '')}${raw(x === 'loading' ? ' aria-busy="true" data-busy="1"' : '')}>${kind === 'link dg' ? 'Delete' : 'Save'}</button>`))}</div>`;
  const field = (label, x, id, extra = '') => cell(label, html`<div class="field"><label for="${id}-${s}">Label</label><input class="input${cls(x)}" id="${id}-${s}" value="${x === 'default' || x === 'invalid' ? '' : 'Value'}" placeholder="Placeholder"${raw(x === 'disabled' ? ' disabled' : x === 'readonly' ? ' readonly' : '')}${raw(x === 'invalid' ? ' aria-invalid="true"' : '')}${raw(extra)}>${x === 'invalid' ? html`<span class="hint bad">Use lowercase letters and dashes.</span>` : html`<span class="hint">Hint text</span>`}</div>`);
  return html`<div class="kit-pane" data-theme="${theme}">
    <h1 class="kit-t">${theme === 'dark' ? 'Dark' : 'Light'} theme</h1>
    ${sub('Buttons', html`${btnRow('Default', '')}${btnRow('Primary', 'primary')}${btnRow('Danger', 'danger')}${btnRow('Link (row action)', 'link')}${btnRow('Link, destructive', 'link dg')}
      <div class="kit-row"><span class="kit-l">Icon only</span>${['default', 'hover', 'focus', 'disabled'].map((x) => cell(x, html`<button type="button" class="btn icon${cls(x)}" aria-label="Settings"${raw(x === 'disabled' ? ' disabled' : '')}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12"/></svg></button>`))}</div>
      <div class="kit-row"><span class="kit-l">Async</span>${cell('click: 1.5 s call', html`<button type="button" class="btn primary" data-act="slow">Save changes</button>`)}${cell('A link button', html`<a class="btn" href="#">Anchor as button</a>`)}</div>`, 'Height 36 px, 44 px touch target. A button that calls the API shows a spinner, keeps its width and cannot be double-clicked.')}
    ${sub('Brand and shell', html`<div class="kit-row"><span class="kit-l">Logo</span>${cell('top bar (30 px)', logo({ href: '#' }))}${cell('sign-in (36 px)', logo({ size: 'lg' }))}${cell('safe mode (26 px)', logo({ size: 'sm' }))}</div>
      <div class="kit-row"><span class="kit-l">Account menu</span>${cell('closed (pill)', html`<div class="kit-menu">${accountMenu({ user: 'minh.dev', role: 'Owner', demo: true })}</div>`)}${cell('open: name, role, 2FA, items, sign out in danger colour', html`<div class="kit-menu open">${accountMenu({ user: 'minh.dev', role: 'Owner', demo: true })}</div>`)}</div>
      <div class="kit-row"><span class="kit-l">Notification bell</span>${cell('0 unread', html`<div class="kit-notif">${notificationBell({ data: { items: [], unread: 0, critical: 0 }, demo: true })}</div>`)}${cell('3 warnings', html`<div class="kit-notif">${notificationBell({ data: { items: [], unread: 3, critical: 0 }, demo: true })}</div>`)}${cell('2 critical (pulse)', html`<div class="kit-notif">${notificationBell({ data: { items: [], unread: 2, critical: 2 }, demo: true })}</div>`)}${cell('open popover (critical on top, snoozed dimmed, unread dot)', html`<div class="kit-notif open">${notificationBell({ data: DEMO_NOTIFS, demo: true, open: true })}</div>`)}${cell('empty ("All clear")', html`<div class="kit-notif open">${notificationBell({ data: { items: [], unread: 0, critical: 0 }, demo: true, open: true })}</div>`)}</div>
      <div class="kit-row"><span class="kit-l">Sidebar groups</span>${cell('expanded', html`<div class="kit-side"><aside class="side" aria-label="Sidebar demo"><nav class="nav">${raw(navMarkup('containers'))}</nav><div class="side-foot"><button type="button" class="side-toggle">${raw('<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 10l-2 2 2 2"/></svg>')}<span class="lbl">Collapse</span></button></div></aside></div>`)}
        ${cell('collapsed: labels become dividers, icon only', html`<div class="kit-side"><div class="shell collapsed"><aside class="side" aria-label="Collapsed sidebar demo"><nav class="nav">${raw(navMarkup('apps'))}</nav><div class="side-foot"><button type="button" class="side-toggle">${raw('<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 10l-2 2 2 2"/></svg>')}<span class="lbl">Collapse</span></button></div></aside></div></div>`)}</div>
      <div class="kit-row"><span class="kit-l">Top process rows</span>${cell('rank, tile, name + pid, CPU and memory bars, "this" badge', html`<div class="kit-w2 procs"><div class="pr pr-h"><span class="pr-rank">#</span><span>Process</span><span class="r">CPU</span><span class="r">Memory</span></div>${[{ name: 'postgres', pid: 1204, cpu: 8.1, mem: 612 }, { name: 'node', pid: 2231, cpu: 6.4, mem: 388 }, { name: 'opsbox', pid: 412, cpu: 0.1, mem: 14, self: true }].map((p, i) => procRow(p, i, 8.1, 8192))}</div>`)}${cell('sort switch and live badge', html`<div class="col gap12"><div class="seg-radio" role="radiogroup" aria-label="Sort by"><button type="button" role="radio" aria-checked="true">CPU</button><button type="button" role="radio" aria-checked="false">Memory</button></div><span><span class="live-badge">● live</span> <span class="live-badge stale">● paused</span></span><a class="more" href="#">All →</a></div>`)}</div>
      <div class="kit-row"><span class="kit-l">Row actions</span>${cell('icon actions (tooltip, danger tint, disabled)', html`<span class="acts">${iconAct('noop', 'Logs', 'logs')}${iconAct('noop', 'Restart', 'restart')}${iconAct('noop', 'Make primary', 'star')}${iconAct('noop', 'Delete', 'del', {}, { danger: true })}${iconAct('noop', 'Disabled', 'check', {}, { disabled: true })}</span>`)}</div>`)}
    ${sub('Text fields', html`<div class="kit-row">${['default', 'hover', 'focus', 'disabled', 'readonly', 'invalid'].map((x) => field(x, x, 'tf'))}</div>
      <div class="kit-row">${cell('number + stepper (memory)', html`<div class="field"><label for="n-${s}">Memory limit (MB)</label>${numberField({ id: `n-${s}`, name: 'n', value: 256, min: 64, max: 4096, step: 64, label: 'memory' })}</div>`)}
        ${cell('search', html`<div class="field"><label for="q-${s}">Search</label><input class="input" id="q-${s}" type="search" value="postgres" placeholder="Search…"></div>`)}
        ${cell('password', html`<div class="field"><label for="p-${s}">Password</label><input class="input" id="p-${s}" type="password" value="hunter2hunter2"></div>`)}
        ${cell('time', html`<div class="field"><label for="t-${s}">From (UTC)</label><input class="input" id="t-${s}" type="time" value="22:00"></div>`)}
        ${cell('date', html`<div class="field"><label for="d-${s}">Date</label><input class="input" id="d-${s}" type="date" value="2026-11-18"></div>`)}</div>
      <div class="kit-row">${cell('textarea', html`<div class="field"><label for="ta-${s}">Environment variables</label><textarea class="input" id="ta-${s}" rows="3" placeholder="KEY=value"></textarea></div>`)}
        ${cell('range', html`<div class="field"><label for="r-${s}">Range</label><input id="r-${s}" type="range" min="0" max="100" value="40"></div>`)}
        ${cell('autofill (Chrome)', html`<form class="field" onsubmit="return false"><label for="u-${s}">Username</label><input class="input" id="u-${s}" name="username" autocomplete="username" placeholder="saved login"></form>`)}</div>`)}
    ${sub('Select, checkbox, radio, switch', html`<div class="kit-row">${['default', 'hover', 'focus', 'disabled', 'invalid'].map((x) => cell(`select ${x}`, html`<div class="field"><label for="s-${x}-${s}">Region</label><select class="select${cls(x)}" id="s-${x}-${s}"${raw(x === 'disabled' ? ' disabled' : '')}${raw(x === 'invalid' ? ' aria-invalid="true"' : '')}><option>Frankfurt</option><option>Singapore</option><option>Virginia</option></select></div>`))}</div>
      <div class="kit-row"><span class="kit-l">Checkbox</span>${[['off', ''], ['on', ' checked'], ['hover', ''], ['focus', ''], ['disabled', ' disabled'], ['disabled on', ' disabled checked']].map(([x, a]) => cell(x, html`<label class="opt"><input type="checkbox" class="${x === 'hover' ? 'is-hover' : x === 'focus' ? 'is-focus' : ''}"${raw(a)}> Option</label>`))}</div>
      <div class="kit-row"><span class="kit-l">Radio</span>${[['off', ''], ['on', ' checked'], ['hover', ''], ['focus', ''], ['disabled', ' disabled']].map(([x, a]) => cell(x, html`<label class="opt"><input type="radio" name="rd-${s}-${x}" class="${x === 'hover' ? 'is-hover' : x === 'focus' ? 'is-focus' : ''}"${raw(a)}> Option</label>`))}</div>
      <div class="kit-row"><span class="kit-l">Switch</span>${cell('off', sw('Off', false, 'noop'))}${cell('on', sw('On', true, 'noop'))}${cell('pending (to on)', html`<button type="button" class="switch pending" role="switch" aria-checked="false" aria-label="Pending"><span class="knob"></span></button>`)}${cell('disabled', html`<button type="button" class="switch" role="switch" aria-checked="true" aria-label="Disabled" disabled><span class="knob"></span></button>`)}${cell('click: works', sw('Works', false, 'swok'))}${cell('click: fails, reverts', sw('Fails', true, 'swfail'))}</div>`)}
    ${sub('Tabs and segmented control', html`<div class="kit-row">${cell('tabs: the pill slides', tabs(['Overview', 'Domains', 'Settings'], 0, 'kittab'))}${cell('segmented', html`<div class="segctl" role="tablist"><span class="ind" aria-hidden="true"></span>${['Hour', 'Day', 'Week'].map((t, i) => html`<button type="button" role="tab" aria-selected="${String(i === 1)}" data-act="kittab" data-i="${i}">${t}</button>`)}</div>`)}</div>`)}
    ${sub('Status chips, check rows', html`<div class="kit-row">${cell('ok (still)', st('ok', 'running 3 d'))}${cell('warn', st('warn', 'degraded'))}${cell('fail', st('fail', 'failed'))}${cell('off', st('off', 'exited'))}${cell('building (pulse)', st('build', 'building'))}${cell('pending (pulse)', st('warn', 'pending'))}${cell('running task (pulse)', st('warn', 'running'))}${cell('check ok', chk('ok', 'All good'))}${cell('check warn', chk('warn', 'Needs a look'))}${cell('fit Fits', fitBadge({ status: 'ok' }))}${cell('fit Tight', fitBadge({ status: 'tight' }))}${cell('fit Blocked', fitBadge({ status: 'blocked' }))}</div>`)}
    ${sub('Capacity', html`<div class="kit-row">${cell('memory map', html`<div class="kit-w2">${memoryMap(KIT_CAP)}</div>`)}</div>
      <div class="kit-row">${cell('feature table (Fits / Tight / Blocked)', html`<div class="kit-w2">${featureTable(KIT_CAP)}</div>`)}</div>
      <div class="kit-row">${cell('Overview Memory footer · normal', html`<div class="kpi kit-w">${capacityFooter(KIT_CAP).html}</div>`)}${cell('Overview Memory footer · warning', html`<div class="kpi kit-w warn-foot">${capacityFooter(KIT_CAP_WARN).html}</div>`)}</div>`, 'Shared labels and badges from capacity.js; Overview keeps a one-line footer with a deep link to #/system/capacity.')}
    ${sub('Bars, skeletons', html`<div class="kit-row">${cell('ok 40%', html`<div class="kit-w">${bar(.4, 'Disk')}</div>`)}${cell('warn 80%', html`<div class="kit-w">${bar(.8, 'Disk')}</div>`)}${cell('danger 95%', html`<div class="kit-w">${bar(.95, 'Disk')}</div>`)}${cell('indeterminate', html`<div class="kit-w"><div class="bar indet" aria-hidden="true"><i></i></div></div>`)}${cell('segments', html`<div class="kit-w">${segBar([[30, 'var(--muted)'], [20, 'var(--accent)'], [12, 'var(--warn)'], [38, 'var(--line)']], 'Usage')}</div>`)}</div>
      <div class="kit-row">${cell('skeleton line / block', html`<div class="kit-w col gap6">${sk(16, '60%')}${sk(40, '100%')}</div>`)}${cell('skeleton section', html`<div class="kit-w">${skeletonSection(2)}</div>`)}</div>`)}
    ${sub('Feedback: toasts, dialogs, tooltip', html`<div class="kit-row">${btn('Toast ok', { act: 'toast-ok' })}${btn('Toast warn', { act: 'toast-warn' })}${btn('Toast error', { act: 'toast-err' })}${btn('Confirm dialog', { act: 'dlg-confirm' })}${btn('Danger confirm', { act: 'dlg-danger' })}${btn('Typed confirm', { act: 'dlg-typed' })}${btn('Form dialog', { act: 'dlg-form' })}${btn('Keep or undo', { act: 'dlg-keep' })}
      <span class="tip" data-tip="Appears after a short delay, never on touch"><button type="button" class="btn">Hover me</button></span></div>`, 'Toasts and dialogs use the page theme (button below). Toasts stack, pause on hover and show a progress line.')}
    ${sub('Drop zone, scrollbar', html`<div class="kit-row">${cell('default', html`<div class="kit-w">${dropzone({ id: `dz-${s}`, label: 'Drop a backup here or' })}</div>`)}${cell('drag over', html`<div class="kit-w"><label class="drop over"><svg viewBox="0 0 24 24" width="22" height="22" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="M12 16V4M7 9l5-5 5 5M4 16v3a1 1 0 001 1h14a1 1 0 001-1v-3"/></svg><span class="drop-t">Release to upload</span></label></div>`)}${cell('file chosen', html`<div class="kit-w"><label class="drop has"><span class="drop-t">Drop a file here or <u>browse</u></span><span class="drop-f sm">backup-2026-11-18.tar.zst (1.4 GB)</span></label></div>`)}${cell('thin themed scrollbar', html`<div class="kit-scroll">${[...Array(14)].map((_, i) => html`<p class="mono sm">log line ${i + 1} · the quick brown fox jumps over the lazy dog</p>`)}</div>`)}</div>`)}
    ${sub('Tables, empty state', html`${cell('sticky header on long tables, numbers right aligned', html`<div class="kit-w2">${table('Long table', [['Name'], ['Size', 'r'], ['State']], [...Array(18)].map((_, i) => [html`<span class="mono">file-${i + 1}.log</span>`, `${(i * 37.3 + 12).toFixed(1)} MB`, st(i % 5 ? 'ok' : 'warn', i % 5 ? 'ok' : 'large')]))}</div>`)}${cell('empty state', html`<div class="kit-w2">${empty('Nothing here yet.', btn('Do the thing', { kind: 'primary', act: 'noop' }))}</div>`)}`)}
    ${sub('Live data (press "Simulate live data")', html`<div data-live="kpis">${liveKpis()}</div>
      <div class="kit-row"><span class="kit-l">Chart: append, stable axis</span></div>
      <div class="chart kit-chart" data-chart="pct" data-keep></div><div class="chart kit-chart" data-chart="net" data-keep></div>
      <div data-live="rows">${liveRows()}</div>
      <div class="kit-row"><span class="kit-l">Stable live lists</span>${btn('Shuffle', { act: 'shuffle', kind: 'primary' })}</div>
      <div class="kit-flip-grid"><div class="kit-flip-box"><h3>With data-flip (constant height, rows glide)</h3><div class="flip-list" data-flip data-kit-flip></div></div><div class="kit-flip-box"><h3>Without data-flip (leaving rows fade out)</h3><div class="flip-list" data-kit-plain></div></div></div>
      <div class="kit-row"><span class="kit-l">Flash</span>${btn('Flash value', { act: 'flash-ok' })}${btn('Flash warning', { act: 'flash-warn' })}${btn('Flash error', { act: 'flash-err' })}<span class="kit-flash mono">12.4 MB</span></div>`)}
  </div>`;
};

function mount() {
  const root = document.getElementById('kit');
  root.innerHTML = `<header class="kit-top"><div><strong>opsbox UI kit</strong><span class="muted sm"> · every component in every state · not linked in the panel</span></div><div class="row"><button type="button" class="btn primary" id="sim">Simulate live data</button><button type="button" class="btn" data-act="fake">Run fake task</button><button type="button" class="btn" data-act="fakefail">Fake failing task</button><button type="button" class="btn" data-act="theme">Page theme (toasts, dialogs)</button><button type="button" class="btn" data-act="live">Toggle “reconnecting”</button></div></header>
    <div class="kit-grid">${pane('dark', 'd')}${pane('light', 'l')}</div>`;
  root.querySelectorAll('[data-chart]').forEach((h) => {
    const pct = h.dataset.chart === 'pct';
    const c = pct ? createChart(h, { label: 'CPU and memory', series: [{ color: 'var(--muted)', width: 1.5, dash: '5 4' }, { color: 'var(--text)', width: 2 }], cap: 100, floor: 20 })
      : createChart(h, { label: 'Network MB/s', series: [{ color: 'var(--ok)', width: 2 }], cap: Infinity, floor: 4, fmt: (v) => `${+v.toFixed(1)}` });
    c.kind = h.dataset.chart; charts.push(c);
  });
  paintLive();
  paintShuffle();
  const toggleSim = () => {
    sim.on = !sim.on; const b = document.getElementById('sim');
    b.textContent = sim.on ? 'Stop simulation' : 'Simulate live data'; b.classList.toggle('primary', !sim.on);
    clearInterval(sim.timer);
    if (sim.on) { fakeTask(); sim.timer = setInterval(() => { stateTick(); paintLive(); if (sim.n % 24 === 0) fakeTask(); }, 1000); }
  };
  document.getElementById('sim').addEventListener('click', toggleSim);
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const flashIt = (kind) => root.querySelectorAll('.kit-flash').forEach((e) => flash(e, kind));
  bindActions(root, {
    noop: () => {},
    slow: async () => { await wait(1500); toast('Saved.'); },
    swok: async (el) => { await wait(900); setSwitch(el, el.getAttribute('aria-checked') !== 'true'); },
    swfail: async () => { await wait(900); throw new Error('Could not change this setting. It was left as it was.'); },
    kittab: (el) => { el.parentElement.querySelectorAll('[role=tab]').forEach((b) => b.setAttribute('aria-selected', String(b === el))); },
    'toast-ok': () => toast('Saved. Nothing else to do.', 'ok'), 'toast-warn': () => toast('The disk is almost full. Clean up before the next backup.', 'warn'), 'toast-err': () => toast('Could not reach the server. Check that it is running.', 'err'),
    'dlg-confirm': async () => { toast((await confirmDialog({ title: 'Restart web?', body: 'The container stops briefly and starts again.', confirm: 'Restart' })) ? 'Confirmed.' : 'Cancelled.'); },
    'dlg-danger': async () => { await confirmDialog({ title: 'Delete web?', body: 'This cannot be undone.', confirm: 'Delete', danger: true }); },
    'dlg-typed': async () => { await confirmDialog({ title: 'Delete the database?', body: 'All its data is lost.', confirm: 'Delete', danger: true, typed: 'shop-db' }); },
    'dlg-form': async () => {
      const r = await formDialog({ title: 'Add a job', body: 'Runs on a schedule as root.', fields: [{ name: 'cron', label: 'Schedule', placeholder: '0 4 * * *', hint: 'Five fields' }, { name: 'keep', label: 'Keep daily snapshots', type: 'number', value: '14', min: 1, max: 365 }, { name: 'where', label: 'Where', options: ['Local disk', 'S3'], value: 'S3' }, { name: 'notes', label: 'Notes', type: 'textarea' }, { name: 'on', label: 'Enabled', type: 'checkbox', value: true }], submit: 'Add', run: async (o) => { await wait(900); if (!/^\S+( \S+){4}$/.test(o.cron.trim())) { const e = new Error('Use five fields, like 0 4 * * *.'); e.field = 'cron'; throw e; } return o; } });
      if (r) toast('Job added.');
    },
    'dlg-keep': () => keepOrUndoDialog({ body: 'Check that you can still reach this machine. If you do nothing, the change is undone.', until: Date.now() / 1000 + 20, keep: async () => {} }),
    'flash-ok': () => flashIt('ok'), 'flash-warn': () => flashIt('warn'), 'flash-err': () => flashIt('err'),
    fake: () => fakeTask(false), fakefail: () => fakeTask(true),
    theme: () => toggleTheme(),
    shuffle: () => {
      const dropIdx = 1;
      const kept = sim.shuffle.filter((_, i) => i !== dropIdx).reverse();
      kept.splice(2, 0, { id: `r${++sim.shuffleSeq}`, name: `worker-${sim.shuffleSeq}`, state: 'ok', v: `${(Math.random() * 9 + 1).toFixed(1)}%` });
      sim.shuffle = kept;
      paintShuffle();
    },
    live: () => { reportPoll(!liveState().ok); announce(liveState().ok ? 'Live' : 'Reconnecting'); },
  });
}
mount();
