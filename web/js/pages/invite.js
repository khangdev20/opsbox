// Someone invited to the panel opens their one-time link: they choose a password, then set
// up two-factor sign-in on the next page.
import { html, esc, setBusy } from '../ui.js';
import * as api from '../api.js';

const ROLE = { admin: 'an administrator', deployer: 'a deployer (apps and logs, no terminal)', viewer: 'a viewer (read only)' };

export async function view({ query }) {
  const token = query.token || '';
  let info = null, problem = '';
  try { if (!token) throw new Error('This invitation link is incomplete. Open the whole link you were sent.'); info = await api.auth.inviteInfo(token); }
  catch (e) { problem = e.message; }
  const left = html`<div class="l"><div><a class="logo" href="#/login">opsbox<i>_</i></a></div>
    <div><h1>You are invited</h1><p class="muted" style="max-width:420px;margin-top:16px">${info ? html`Choose a password for <b>${info.name}</b>. You will join as ${ROLE[info.role] || info.role}. Next you set up a code app for two-factor sign-in.` : 'This link cannot be used.'}</p></div><p></p></div>`;
  return {
    nav: null, title: 'Accept invitation',
    html: info ? html`<div class="split-page">${left}
      <form class="r" id="f" autocomplete="off">
        <h2>Choose your password</h2>
        <div class="field"><label for="p">Password</label><input class="input lg" id="p" type="password" autocomplete="new-password"><span class="hint">At least 12 characters. A few random words work well.</span></div>
        <div class="field"><label for="p2">Repeat the password</label><input class="input lg" id="p2" type="password" autocomplete="new-password"></div>
        <p id="err" class="form-err" role="alert"></p>
        <div class="row"><button class="btn primary" type="submit">Continue</button></div></form></div>`
      : html`<div class="split-page">${left}<div class="r"><h2>This invitation does not work</h2><p role="alert" style="color:var(--danger)">${problem}</p><div class="row"><a class="btn" href="#/login">Go to sign in</a></div></div></div>`,
    mount(root) {
      const f = root.querySelector('#f'); if (!f) return;
      f.addEventListener('submit', async (ev) => {
        ev.preventDefault(); const b = f.querySelector('button'); if (b.hasAttribute('data-busy')) return; setBusy(b, true);
        try {
          if (f.p.value !== f.p2.value) throw new Error('The two passwords do not match.');
          await api.auth.claimInvite(token, f.p.value); location.hash = '#/enroll?new=1';
        } catch (err) { root.querySelector('#err').textContent = err.message; setBusy(b, false); }
      });
      root.querySelector('#p').focus();
    },
  };
}
