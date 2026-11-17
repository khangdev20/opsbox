import { html } from '../ui.js';
import * as api from '../api.js';
import { session } from '../store.js';

const CMD = {
  AWS: 'EC2 → select the instance → Actions → Monitor and troubleshoot → Get system log',
  'Google Cloud': 'Compute Engine → the instance → Logs → Serial port 1 (console)',
  Azure: 'Virtual machine → Help → Boot diagnostics → Serial log',
  Other: 'Your provider’s console or serial log for this machine',
};
export async function view() {
  return {
    nav: null, title: 'Set up',
    html: html`<div class="split-page">
      <div class="l"><div><a class="logo" href="#/login">opsbox<i>_</i></a></div>
        <div class="rail" aria-label="Setup steps"><div class="on"><span>01</span>Claim this server</div><div><span>02</span>Create admin account</div><div><span>03</span>Scan 2FA code</div></div>
        <p class="muted sm">Nothing here is reachable until you claim it with the token below.</p></div>
      <form class="r" id="f">
        <h2 style="font-size:22px">Claim this server</h2>
        <p class="muted">The installer printed a one-time token to the boot log. Open it from your cloud console:</p>
        <div class="tabs" role="tablist">${Object.keys(CMD).map((k, i) => html`<button type="button" role="tab" aria-selected="${i === 0}" data-p="${k}">${k}</button>`)}</div>
        <p id="where" class="sm">${CMD.AWS}</p>
        <pre class="code">opsbox  setup url   https://203.0.113.10:8443/#/setup
opsbox  setup token 7QK2-9XMD-4TFA   (single use, expires in 1 h)</pre>
        <div class="field"><label for="t">Setup token</label><input class="input lg mono" id="t" placeholder="XXXX-XXXX-XXXX"><span class="hint">Demo token: 7QK2-9XMD-4TFA</span></div>
        <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p>
        <div class="row"><button class="btn primary" type="submit">Claim server</button></div>
      </form></div>`,
    mount(root) {
      root.querySelectorAll('[data-p]').forEach((b) => b.addEventListener('click', () => {
        root.querySelectorAll('[data-p]').forEach((x) => x.setAttribute('aria-selected', x === b));
        root.querySelector('#where').textContent = CMD[b.dataset.p];
      }));
      const f = root.querySelector('#f');
      f.addEventListener('submit', async (e) => {
        e.preventDefault();
        try { await api.auth.claim(f.t.value); session.set('admin'); let next = '#/'; try { next = sessionStorage.getItem('opsbox.next') || '#/'; sessionStorage.removeItem('opsbox.next'); } catch {} location.hash = /^#\/(login|setup)/.test(next) ? '#/' : next; }
        catch (err) { root.querySelector('#err').textContent = err.message; }
      });
    },
  };
}
