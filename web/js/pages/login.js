import { html, bindActions, toast } from '../ui.js';
import * as api from '../api.js';
import { session } from '../store.js';

export async function view() {
  return {
    nav: null, title: 'Sign in',
    html: html`<div class="split-page">
      <div class="l"><div><a class="logo" href="#/login">opsbox<i>_</i></a></div>
        <div><h1>One panel for<br>your whole server.</h1><p class="muted" style="max-width:420px;margin-top:16px">Deploy from GitHub, watch the machine, open a terminal, edit files. No SSH session needed.</p></div>
        <p class="muted sm mono">srv-sgn-01 · opsbox 0.1.0 · demo data</p></div>
      <form class="r" id="f" autocomplete="off">
        <h2 style="font-size:22px">Sign in</h2>
        <div class="field"><label for="u">Username</label><input class="input lg" id="u" value="admin"></div>
        <div class="field"><label for="p">Password</label><input class="input lg" id="p" type="password" value="demo-password"></div>
        <div class="field"><label for="c">Authentication code</label><input class="input lg mono" id="c" inputmode="numeric" maxlength="6" placeholder="123456"><span class="hint">6 digits from your authenticator app. Demo: any 6 digits.</span></div>
        <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p>
        <div class="row"><button class="btn primary" type="submit">Sign in</button><a class="btn link" href="#/setup">First time here?</a></div>
      </form></div>`,
    mount(root) {
      const f = root.querySelector('#f');
      f.addEventListener('submit', async (e) => {
        e.preventDefault(); const b = f.querySelector('button'); b.disabled = true;
        try { const r = await api.auth.login(f.u.value, f.p.value, f.c.value); session.set(r.user); let next = '#/'; try { next = sessionStorage.getItem('opsbox.next') || '#/'; sessionStorage.removeItem('opsbox.next'); } catch {} location.hash = /^#\/(login|setup)/.test(next) ? '#/' : next; }
        catch (err) { root.querySelector('#err').textContent = err.message; b.disabled = false; }
      });
      root.querySelector('#c').focus();
    },
  };
}
