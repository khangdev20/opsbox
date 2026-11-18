import { html, bindActions, toast, setBusy, logo } from '../ui.js';
import * as api from '../api.js';
import { session, server, theme, toggleTheme } from '../store.js';

export async function view() {
  const demo = api.MODE === 'mock';
  return {
    nav: null, title: 'Sign in',
    html: html`<div class="split-page">
      <div class="l"><div>${logo({ href: '#/login', size: 'lg' })}</div>
        <div><h1>One panel for<br>your whole server.</h1><p class="muted lede">Deploy from GitHub, watch the machine, open a terminal, edit files. No SSH session needed.</p></div>
        <p class="muted sm mono">opsbox 0.1.0${demo ? ' · demo data' : ''}</p></div>
      <form class="r" id="f" autocomplete="off">
        <h2>Sign in</h2>
        <div class="field"><label for="u">Username</label><input class="input lg" id="u" value="${demo ? 'admin' : ''}" autocomplete="username"></div>
        <div class="field"><label for="p">Password</label><input class="input lg" id="p" type="password" value="${demo ? 'demo-password' : ''}" autocomplete="current-password"></div>
        <div class="field"><label for="c">Authentication code</label><input class="input lg mono" id="c" inputmode="numeric" maxlength="6" placeholder="123456" autocomplete="one-time-code"><span class="hint">6 digits from your authenticator app.${demo ? ' Demo: any 6 digits.' : ' Lost it? An administrator can run  opsbox user reset-2fa  on the server.'}</span></div>
        <p id="err" class="form-err" role="alert"></p>
        <button class="btn primary block" type="submit">Sign in</button>
        <div class="row">${demo || server.setupRequired ? html`<a class="btn link" href="#/setup">First time here?</a>` : ''}<button type="button" class="btn link" id="theme-flip">${theme() === 'light' ? 'Dark' : 'Light'} theme</button></div>
      </form></div>`,
    mount(root) {
      const f = root.querySelector('#f');
      const flip = root.querySelector('#theme-flip'); flip.addEventListener('click', () => { toggleTheme(); flip.textContent = `${theme() === 'light' ? 'Dark' : 'Light'} theme`; });
      f.addEventListener('submit', async (e) => {
        e.preventDefault(); const b = f.querySelector('button[type=submit]'); if (b.hasAttribute('data-busy')) return; setBusy(b, true);
        try {
          const r = await api.auth.login(f.u.value, f.p.value, f.c.value);
          if (r.enrollRequired) { location.hash = '#/enroll'; return; }
          session.set(r.user); server.role = r.role || ''; let next = '#/'; try { next = sessionStorage.getItem('opsbox.next') || '#/'; sessionStorage.removeItem('opsbox.next'); } catch {}
          location.hash = /^#\/(login|setup|enroll)/.test(next) ? '#/' : next;
        } catch (err) { root.querySelector('#err').textContent = err.message; setBusy(b, false); f.c.value = ''; f.c.focus(); }
      });
      root.querySelector(demo ? '#c' : '#u').focus();
    },
  };
}
