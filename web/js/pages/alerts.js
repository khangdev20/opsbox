import { html, st, btn, table, section, header, sw, bindActions, askForm, toast, patch, poll, refreshing, announce, empty } from '../ui.js';
import * as api from '../api.js';

const fields = {
  c1: [
    { name: 'host', label: 'SMTP host', placeholder: 'smtp.example.com' },
    { name: 'port', label: 'TLS port (465 or 587)', value: '587' },
    { name: 'username', label: 'SMTP username' },
    { name: 'password', label: 'SMTP password', type: 'password' },
    { name: 'from', label: 'Sender email' }, { name: 'to', label: 'Recipient email' },
  ],
  c2: [{ name: 'url', label: 'Slack incoming webhook URL', type: 'password' }],
  c3: [{ name: 'token', label: 'Telegram bot token', type: 'password' }, { name: 'chat', label: 'Telegram chat ID' }],
  c4: [{ name: 'url', label: 'HTTPS webhook URL', type: 'password' }, { name: 'secret', label: 'Signing secret (at least 16 characters)', type: 'password' }],
  c5: [{ name: 'url', label: 'Webhook URL', type: 'password', hint: 'Server settings → Integrations → Webhooks → Copy URL' }],
};
const kinds = { c1: 'email', c2: 'slack', c3: 'telegram', c4: 'webhook', c5: 'discord' };
const time = (ts) => new Date(ts * 1000).toLocaleString();
const status = (state) => st(state === 'firing' ? 'fail' : state === 'pending' ? 'warn' : 'ok', state);
const keyed = (cells, key, sig = '') => Object.assign(cells, { meta: { key, sig } });

export async function view({ query = {} } = {}) {
  let d = await api.getAlerts(), root, stopPoll;
  const known = new Map((d.incidents || []).map((i) => [i.id, i.state]));
  const body = () => {
    const incidents = (d.incidents || []).filter((i) => !query.id || i.id === query.id);
    return html`
    ${query.id ? html`<p class="muted">Showing the alert selected in Overview. <a href="#/alerts">Show all alerts</a></p>` : ''}
    ${incidents.length ? incidents.map((i) => html`<div class="callout" data-key="i-${i.id}" data-sig="${i.state}|${i.silenceUntil}">${status(i.state)}<span><b>${i.name}</b><br>${i.msg}<br><span class="sm muted">Since ${time(i.since)}${i.silenceUntil > Date.now() / 1000 ? ` · silenced until ${time(i.silenceUntil)}` : ''}</span></span><span class="row"><a class="btn primary" href="${i.link}">Review</a>${i.state !== 'resolved' ? btn(i.silenceUntil > Date.now() / 1000 ? 'End silence' : 'Silence 1 h', { act: 'silence', data: { id: i.id, seconds: i.silenceUntil > Date.now() / 1000 ? 0 : 3600 } }) : ''}</span></div>`) : html`<p class="muted">${query.id ? 'This alert is no longer active or has expired.' : 'No active alerts.'}</p>`}
    <div class="cols"><div class="w3">
      ${section('Channels', table('Notification channels', [['Channel'], ['For'], ['Status'], ['', { align: 'r', label: false }]], d.channels.map((c) => keyed([html`${c.name}${c.id === 'c5' && c.on ? html`<div class="sm muted">${c.to}</div>` : ''}`, c.for || 'all alerts', html`${c.error ? st('fail', c.error) : c.on ? st(c.verified ? 'ok' : 'warn', c.verified ? 'delivery working' : 'ready to test') : st('off', 'off')}`, html`<div class="row">${c.on ? btn('Send test', { act: 'test', data: { id: c.id }, kind: 'link' }) : ''}${btn(c.on ? 'Edit' : 'Connect', { act: 'connect', data: { id: c.id }, kind: 'link' })}${c.on ? btn('Disconnect', { act: 'disconnect', data: { id: c.id }, kind: 'link' }) : ''}</div>`], `c-${c.id}`, `${c.on}|${c.verified}|${c.error || ''}`))))}
      ${section('Rules', table('Alert rules', [['Rule'], ['When'], ['Level'], ['Notify'], ['On']], d.rules.map((r) => keyed([r.name, html`<span class="sm">${r.cond}</span>`, r.level === 'critical' ? st('fail', 'critical') : st('warn', 'warning'), r.notify, sw(`Enable ${r.name}`, r.on, 'rule', { id: r.id })], `r-${r.id}`, `${r.on}`))))}
      ${section('Recent alerts', d.history.length ? table('Recent alerts', [['Time'], ['Alert'], ['State']], d.history.map((h) => keyed([html`<span class="mono">${h.ts ? time(h.ts) : h.t}</span>`, html`<b>${h.name}</b><div class="sm muted">${h.msg}</div>`, status(h.state)], `h-${h.ts || h.t}-${h.name}`, h.state))) : empty('No alerts yet. When something needs you, it shows up here.'))}
    </div><div>
      ${section('Quiet hours (UTC)', html`<p class="muted sm">Warnings wait until quiet hours end. Critical alerts always send. Set equal times to turn quiet hours off.</p>
        <div class="field"><label for="qf">From (UTC)</label><input class="input" type="time" id="qf" value="${d.quiet.from}"></div>
        <div class="field"><label for="qt">To (UTC)</label><input class="input" type="time" id="qt" value="${d.quiet.to}"></div>
        <div class="row">${btn('Save', { act: 'quiet' })}</div>`)}
      ${section('What to expect', html`<p class="muted sm">Alerts are checked every 15 seconds. Notifications say what happened and where to fix it. Active problems repeat every four hours, and send again when they get worse or recover. A channel that fails will retry.</p>`)}
    </div></div>`;
  };
  const paint = () => patch(root.querySelector('#b'), body().toString());
  const reload = async () => {
    d = await api.getAlerts();
    if (!root?.isConnected) return;
    // Say out loud only what matters: an alert that started firing, or one that cleared.
    for (const i of d.incidents || []) {
      const was = known.get(i.id);
      if (was !== i.state) announce(i.state === 'firing' ? `Alert firing: ${i.name}. ${i.msg}` : i.state === 'resolved' ? `Alert resolved: ${i.name}.` : '');
      known.set(i.id, i.state);
    }
    paint();
  };
  return {
    nav: 'alerts', title: 'Alerts',
    html: html`<main>${header('Alerts', 'Know when something needs you, without watching the panel', btn('Send a test alert', { act: 'test', data: { id: '' } }))}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      stopPoll = poll(() => refreshing(root.querySelector('#b'), reload), 15000);
      bindActions(root, {
        silence: async (el) => { await api.silenceAlert(el.dataset.id, Number(el.dataset.seconds)); toast(Number(el.dataset.seconds) ? 'Silenced for 1 hour.' : 'Silence ended.'); await reload(); },
        test: async (el) => { try { await api.testAlert(el.dataset.id); toast('Test alert sent.'); } finally { await reload(); } },
        rule: async (el) => { await api.setRule(el.dataset.id, el.getAttribute('aria-checked') !== 'true'); await reload(); },
        quiet: async () => { await api.setQuiet({ from: root.querySelector('#qf').value, to: root.querySelector('#qt').value }); toast('Saved.'); },
        disconnect: async (el) => { await api.disconnectChannel(el.dataset.id); toast('Channel disconnected.'); await reload(); },
        connect: async (el) => {
          const c = d.channels.find((x) => x.id === el.dataset.id);
          if (await askForm({ title: `${c.on ? 'Edit' : 'Connect'} ${c.name}`, fields: fields[c.id], submit: 'Save channel' }, (o) => api.connectChannel(c.id, { ...o, kind: kinds[c.id], ...(c.id === 'c1' ? { port: Number(o.port) } : {}) }))) { toast('Channel saved. Send a test to check delivery.'); await reload(); }
        },
      });
    },
    unmount() { if (stopPoll) stopPoll(); root = null; },
  };
}
