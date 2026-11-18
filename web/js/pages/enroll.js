// Two-factor enrolment for an account whose second factor was reset (see
// `opsbox user reset-2fa`): the password was right, but a code app must be set up first.
import { html, raw, esc, setBusy } from '../ui.js';
import * as api from '../api.js';
import { session, server } from '../store.js';
import { qrSvg, groupSecret } from '../qr.js';

export async function view({ query = {} } = {}) {
  const fresh = !!query.new;
  const e = await api.auth.enroll();
  const qr = await qrSvg(e.otpauthUri);
  return {
    nav: null, title: 'Set up two-factor',
    html: html`<div class="split-page">
      <div class="l"><div><a class="logo" href="#/login">opsbox<i>_</i></a></div>
        <div><h1>Set up two-factor<br>authentication</h1><p class="muted" style="max-width:420px;margin-top:16px">${fresh ? 'One last step. Scan the code with an authenticator app, then enter the first 6 digits it shows.' : 'Your authenticator was reset by an administrator. Scan the new code, then enter the first 6 digits it shows.'}</p></div><p></p></div>
      <form class="r" id="f" autocomplete="off">
        <h2 style="font-size:22px">Scan this code</h2>
        <div id="qr" style="width:200px;border-radius:8px;overflow:hidden">${raw(qr)}</div>
        <p class="sm muted">Or enter the key by hand: <b class="mono" id="key">${groupSecret(e.secret)}</b></p>
        <div class="field"><label for="c">Authentication code</label><input class="input lg mono" id="c" inputmode="numeric" maxlength="6" placeholder="123456" autocomplete="one-time-code"></div>
        <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p>
        <div class="row"><button class="btn primary" type="submit">Turn on and sign in</button></div>
      </form></div>`,
    mount(root) {
      const f = root.querySelector('#f');
      f.addEventListener('submit', async (ev) => {
        ev.preventDefault(); const b = f.querySelector('button'); if (b.hasAttribute('data-busy')) return; setBusy(b, true);
        try { const r = await api.auth.enrollConfirm(f.c.value); session.set(r.user); server.role = r.role || ''; location.hash = '#/'; }
        catch (err) { root.querySelector('#err').textContent = err.message; setBusy(b, false); }
      });
      root.querySelector('#c').focus();
    },
  };
}
