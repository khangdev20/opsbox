import { html, st, btn, bindActions, toast } from '../ui.js';
import * as api from '../api.js';

export async function view() {
  let d, root;
  const paint = async () => {
    d = await api.getRecovery();
    root.querySelector('#b').innerHTML = html`<div class="row" style="margin-bottom:16px"><span class="safe-tag">SAFE MODE</span><span class="muted">Served by the watchdog on :8444, separate from the main panel.</span></div>
      <h1>${d.failed ? 'opsbox is not running.' : 'opsbox is healthy.'}</h1>
      <div style="margin:20px 0">${d.checks.map(([n, k, s, note]) => html`<div class="list-row"><span>${st(k, n)}</span><span class="mono sm">${s} <span class="muted">· ${note}</span></span></div>`)}</div>
      <pre class="code" aria-label="Recent log">${d.log.join('\n')}</pre>
      <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p>
      <div class="row" style="margin-top:16px">${btn('Free disk space', { act: 'free', kind: d.failed ? 'primary' : '' })}${btn('Restart opsbox', { act: 'restart' })}${btn('Roll back to 0.1.3', { act: 'rollback' })}${btn('Open SSH for 1 hour', { act: 'ssh' })}${btn('Reboot', { act: 'reboot', kind: 'danger' })}</div>
      <div class="row" style="margin-top:24px">${d.failed ? '' : btn('Simulate failure', { act: 'sim', kind: 'link' })}<a class="btn link" href="#/">Back to panel</a></div>`.toString();
  };
  const act = (a) => async () => { try { const r = await api.recoveryAction(a); toast(r.msg); await paint(); } catch (e) { root.querySelector('#err').textContent = e.message; } };
  return {
    nav: null, title: 'Safe mode',
    html: html`<main class="narrow" id="b" style="padding-top:48px"></main>`,
    async mount(r) {
      root = r; await paint();
      bindActions(root, { free: act('free'), restart: act('restart'), rollback: act('rollback'), ssh: act('ssh'), reboot: act('reboot'), sim: async () => { await api.simulateFailure(); await paint(); } });
    },
  };
}
