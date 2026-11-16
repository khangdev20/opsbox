import { html, st, btn, table, section, header, sw, bindActions, askForm, toast } from '../ui.js';
import * as api from '../api.js';

export async function view() {
  let d = await api.getAlerts(), root;
  const body = () => html`
    ${d.firing ? html`<div class="callout"><span class="tag">▲</span><span><b>Disk / is at 82%</b> since 03:12; the threshold is 75%. 14.6 GB can be reclaimed.</span><span class="row"><a class="btn primary" href="#/cleanup">Clean up</a>${btn('Silence 1 h', { act: 'silence' })}</span></div>` : ''}
    <div class="cols"><div class="w3">
      ${section('Channels', table('Notification channels', [['Channel'], ['Sends to'], ['For'], ['Status'], ['', 'r']], d.channels.map((c) => [c.name, html`<span class="mono">${c.to || '—'}</span>`, c.for || '—', c.on ? st('ok', 'verified') : st('off', 'off'), c.on ? btn('Send test', { act: 'test', data: { id: c.id }, kind: 'link' }) : btn('Connect', { act: 'connect', data: { id: c.id }, kind: 'link' })])))}
      ${section('Rules', table('Alert rules', [['Rule'], ['When'], ['Level'], ['Notify'], ['On']], d.rules.map((r) => [r.name, html`<span class="mono sm">${r.cond}</span>`, r.level === 'critical' ? st('fail', 'critical') : st('warn', 'warning'), r.notify, sw(`Enable ${r.name}`, r.on, 'rule', { id: r.id })])))}
      ${section('Recent alerts', table('Recent alerts', [['Time'], ['Alert'], ['State']], d.history.map((h) => [html`<span class="mono">${h.t}</span>`, html`<b>${h.name}</b><div class="sm muted">${h.msg}</div>`, h.state === 'firing' ? st('warn', 'firing') : st('ok', h.state)])))}
    </div><div>
      ${section('Quiet hours', html`<p class="muted sm">Warnings wait until morning. Critical alerts always send.</p>
        <div class="field"><label for="qf">From</label><select class="select" id="qf">${['22:00', '23:00', '00:00'].map((x) => html`<option${x === d.quiet.from ? ' selected' : ''}>${x}</option>`)}</select></div>
        <div class="field"><label for="qt">To</label><select class="select" id="qt">${['06:00', '07:00', '08:00'].map((x) => html`<option${x === d.quiet.to ? ' selected' : ''}>${x}</option>`)}</select></div>
        <div class="row">${btn('Save', { act: 'quiet' })}${btn('Send a test alert', { act: 'test', data: { id: '' } })}</div>`)}
      ${section('What to expect', html`<p class="muted sm">Each alert says what happened and links to where to fix it. The same problem is sent once, then again only when it clears or gets worse.</p>`)}
    </div></div>`;
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  const reload = async () => { d = await api.getAlerts(); paint(); };
  return {
    nav: 'alerts', title: 'Alerts',
    html: html`<main>${header('Alerts', 'Know when something needs you, without watching the panel', btn('Send a test alert', { act: 'test', data: { id: '' } }))}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      bindActions(root, {
        silence: async () => { await api.silenceAlert(); toast('Silenced for 1 hour.'); await reload(); },
        test: async (el) => { await api.testAlert(el.dataset.id); toast('Test alert sent.'); },
        rule: async (el) => { await api.setRule(el.dataset.id, el.getAttribute('aria-checked') !== 'true'); await reload(); },
        quiet: async () => { await api.setQuiet({ from: root.querySelector('#qf').value, to: root.querySelector('#qt').value }); toast('Saved.'); },
        connect: async (el) => {
          const c = d.channels.find((x) => x.id === el.dataset.id);
          const label = c.id === 'c4' ? 'Webhook URL' : c.id === 'c3' ? 'Telegram chat id' : 'Address';
          if (await askForm({ title: `Connect ${c.name}`, fields: [{ name: 'to', label, placeholder: c.id === 'c4' ? 'https://…' : '' }], submit: 'Connect' }, (o) => api.connectChannel(c.id, o.to.trim()))) { toast(`${c.name} connected.`); await reload(); }
        },
      });
    },
  };
}
