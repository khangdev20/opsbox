import { html, st, section, header, bindActions } from '../ui.js';
import * as api from '../api.js';

export async function view() {
  const repos = await api.getRepos(); let q = '';
  const list = () => repos.filter((r) => r.name.includes(q.toLowerCase())).map((r) => html`<div class="list-row"><div><div class="mono">${r.name}</div><div class="sm muted">${r.vis} · ${r.lang} · pushed ${r.pushed} · detected: ${r.build}</div></div><a class="btn primary" href="#/apps/new/configure?repo=${encodeURIComponent(r.name)}&build=${encodeURIComponent(r.build)}">Deploy</a></div>`);
  return {
    nav: 'apps', title: 'New service',
    html: html`<main class="narrow">${header('New service', 'Pick a repository. opsbox detects how to build it.')}
      <div class="tabs" role="tablist">${['GitHub', 'Public repo URL', 'Docker image', 'Templates'].map((t, i) => html`<button type="button" role="tab" aria-selected="${i === 0}" data-t="${i}">${t}</button>`)}</div>
      <div class="row" style="margin:16px 0"><span class="st ok">■ GitHub connected as acme</span><a class="btn link" href="#/apps">Cancel</a></div>
      <div class="field"><label for="q">Search repositories</label><input class="input lg" id="q" placeholder="acme/…" autocomplete="off"></div>
      <div id="l">${list()}</div></main>`,
    mount(root) {
      root.querySelectorAll('[data-t]').forEach((b) => b.addEventListener('click', () => { if (b.dataset.t === '3') location.hash = '#/apps/new/templates'; })); root.querySelector('#q').addEventListener('input', (e) => { q = e.target.value; const l = list().toString(); root.querySelector('#l').innerHTML = l || '<p class="muted">No repository matches.</p>'; }); },
  };
}
