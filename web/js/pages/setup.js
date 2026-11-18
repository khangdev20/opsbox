import { html, raw, setBusy, logo } from '../ui.js';
import * as api from '../api.js';
import { session, server } from '../store.js';
import { qrSvg, groupSecret } from '../qr.js';

const CMD = {
  'Cloud console': 'Your provider’s console or serial log for this machine (AWS: Actions → Monitor and troubleshoot → Get system log)',
  'Over SSH': 'Run  sudo opsbox setup-token  on the server. It prints a new token and the setup URL.',
  'Journal': 'Run  sudo journalctl -u opsbox | grep -A3 "first-run setup"  on the server.',
};
const STEPS = ['Claim this server', 'Create admin account', 'Scan 2FA code'];
const rail = (at) => html`<div class="rail" aria-label="Setup steps">${STEPS.map((t, i) => html`<div class="${i === at ? 'on' : ''}"><span>0${i + 1}</span>${t}</div>`)}</div>`;
const left = (at) => html`<div class="l"><div>${logo({ href: '#/login', size: 'lg' })}</div><div class="col gap6"><span class="muted">First-time setup</span>${rail(at)}</div><p class="muted sm">Nothing here is reachable until you claim it with the token below.</p></div>`;
const err = html`<p id="err" class="form-err" role="alert"></p>`;

const stepClaim = () => html`<form class="r" id="f">
  <h2>Claim this server</h2>
  <p class="muted">The installer printed a one-time token. Find it here:</p>
  <div class="tabs" role="tablist">${Object.keys(CMD).map((k, i) => html`<button type="button" role="tab" aria-selected="${String(i === 0)}" data-p="${k}">${k}</button>`)}</div>
  <p id="where" class="sm">${Object.values(CMD)[0]}</p>
  <div class="field"><label for="t">Setup token</label><input class="input lg mono" id="t" placeholder="XXXX-XXXX-XXXX" autocomplete="off" autocapitalize="characters"><span class="hint">Single use, valid for one hour.</span></div>
  ${err}<div class="row"><button class="btn primary" type="submit">Claim server</button></div></form>`;

const stepAdmin = () => html`<form class="r" id="f">
  <h2>Create the admin account</h2>
  <div class="field"><label for="u">Username</label><input class="input lg" id="u" autocomplete="username" placeholder="admin"><span class="hint">Lower-case letters, digits, dots and dashes.</span></div>
  <div class="field"><label for="p">Password</label><input class="input lg" id="p" type="password" autocomplete="new-password"><span class="hint">At least 12 characters. A few random words work well.</span></div>
  <div class="field"><label for="p2">Repeat the password</label><input class="input lg" id="p2" type="password" autocomplete="new-password"></div>
  ${err}<div class="row"><button class="btn primary" type="submit">Continue</button></div></form>`;

const stepScan = (e, qr) => html`<form class="r" id="f" autocomplete="off">
  <h2>Scan this code</h2>
  <p class="muted">Open an authenticator app (1Password, Aegis, Google Authenticator, …) and scan it. The code is drawn in this page; nothing is sent anywhere.</p>
  <div id="qr" class="qr">${raw(qr)}</div>
  <p class="sm muted">Or enter the key by hand: <b class="mono" id="key">${groupSecret(e.secret)}</b></p>
  <div class="field"><label for="c">First code from the app</label><input class="input lg mono" id="c" inputmode="numeric" maxlength="6" placeholder="123456" autocomplete="one-time-code"></div>
  ${err}<div class="row"><button class="btn primary" type="submit">Finish setup</button></div></form>`;

export async function view() {
  return {
    nav: null, title: 'Set up',
    html: html`<div class="split-page setup" id="setup">${left(0)}${stepClaim()}</div>`,
    mount(root) {
      const page = root.querySelector('#setup');
      const show = (at, form) => { page.querySelector('.l').outerHTML = left(at).toString(); page.querySelector('form').outerHTML = form.toString(); wire(at); };
      const submit = (handler) => {
        const f = page.querySelector('#f');
        f.addEventListener('submit', async (e) => {
          e.preventDefault(); const b = f.querySelector('button[type=submit]'); if (b.hasAttribute('data-busy')) return; setBusy(b, true);
          try { await handler(f); } catch (ex) { page.querySelector('#err').textContent = ex.message; setBusy(b, false); }
        });
        f.querySelector('input')?.focus();
      };
      function wire(at) {
        if (at === 0) {
          page.querySelectorAll('[data-p]').forEach((b) => b.addEventListener('click', () => {
            page.querySelectorAll('[data-p]').forEach((x) => x.setAttribute('aria-selected', x === b));
            page.querySelector('#where').textContent = CMD[b.dataset.p];
          }));
          submit(async (f) => { await api.auth.claim(f.t.value); show(1, stepAdmin()); });
        } else if (at === 1) {
          submit(async (f) => {
            if (f.p.value !== f.p2.value) throw new Error('The two passwords do not match.');
            const e = await api.auth.setupAdmin(f.u.value, f.p.value);
            show(2, stepScan(e, await qrSvg(e.otpauthUri)));
          });
        } else {
          submit(async (f) => {
            const r = await api.auth.setupConfirm(f.c.value);
            session.set(r.user); server.role = r.role || ''; server.setupRequired = false; location.hash = '#/';
          });
        }
      }
      wire(0);
    },
  };
}
