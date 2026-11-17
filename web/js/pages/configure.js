import { html, header, section, toast } from '../ui.js';
import * as api from '../api.js';

export async function view({ query }) {
  const repo = query.repo || 'acme/shop'; const build = query.build || 'Dockerfile';
  const branch = query.branch || 'main';
  const def = repo.replace(/\.git$/, '').split('/').filter(Boolean).pop() || 'app';
  const branches = [...new Set([branch, 'main', 'master', 'develop'])];
  const builds = ['Dockerfile', 'Nixpacks', 'static site', 'Docker Compose'];
  return {
    nav: 'apps', title: 'Configure',
    html: html`<main class="narrow">${header(`Configure ${repo}`, 'Review the detected settings, then deploy.')}
      <form id="f">
        <div class="field"><label for="name">Service name</label><input class="input" id="name" name="name" value="${def}"></div>
        <div class="field"><label for="branch">Branch</label><select class="select" id="branch" name="branch">${branches.map((b) => html`<option${b === branch ? ' selected' : ''}>${b}</option>`)}</select></div>
        <div class="field"><label for="build">Build</label><select class="select" id="build" name="build">${builds.map((b) => html`<option${build.startsWith(b) ? ' selected' : ''}>${b}</option>`)}</select><span class="hint">Detected: ${build}</span></div>
        <div class="field"><label for="port">Port</label><input class="input" id="port" name="port" value="3000" inputmode="numeric"></div>
        <div class="field"><label for="domain">Subdomain</label><div class="row"><input class="input grow" id="domain" name="domain" value="${def.replace('shop-', '')}"><span class="muted mono">.example.com</span></div><span class="hint">HTTPS certificate is issued automatically.</span></div>
        <div class="field"><label for="vars">Environment variables</label><textarea class="input" id="vars" name="vars" rows="4" placeholder="KEY=value (one per line)" style="height:auto;font-family:var(--mono)"></textarea></div>
        <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p>
        <div class="row"><button class="btn primary" type="submit">Deploy</button><a class="btn link" href="#/apps/new">Back</a></div>
      </form></main>`,
    mount(root) {
      const f = root.querySelector('#f');
      f.addEventListener('submit', async (e) => {
        e.preventDefault(); const b = f.querySelector('button'); b.disabled = true;
        const vars = f.vars.value.split('\n').map((l) => l.trim()).filter(Boolean).map((l) => { const i = l.indexOf('='); return i < 0 ? [l, ''] : [l.slice(0, i), l.slice(i + 1)]; });
        try { await api.createService({ repo, name: f.name.value, branch: f.branch.value, build: f.build.value, port: f.port.value, domain: f.domain.value, vars }); toast('Deploy started.'); location.hash = `#/apps?s=${encodeURIComponent(f.name.value.trim())}`; }
        catch (err) { root.querySelector('#err').textContent = err.message; b.disabled = false; }
      });
    },
  };
}
