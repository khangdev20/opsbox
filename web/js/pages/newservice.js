import { html, st, btn, section, header, bindActions, confirmDialog, toast } from '../ui.js';
import * as api from '../api.js';
import { server } from '../store.js';

export async function view({ query = {} } = {}) {
  let tab = query.tab === 'deploy-key' ? 1 : query.tab === 'public' ? 2 : query.tab === 'image' ? 3 : 0;
  let q = '';
  let gh = await api.getGithubStatus();
  let repoRes = gh.connected && gh.installed ? await api.getRepos() : { connected: gh.connected, installed: gh.installed, account: gh.account, installUrl: gh.installUrl, manageUrl: gh.manageUrl, repos: [] };
  let keys = await api.listDeployKeys();
  let pubUrl = '', pubInfo = null, pubErr = '', dkRepo = '', dkErr = '', ghErr = '', busy = false;
  let root;

  const repoRows = () => {
    const list = Array.isArray(repoRes) ? repoRes : (repoRes.repos || []);
    const term = q.trim().toLowerCase();
    return list.filter((r) => !term || r.name.toLowerCase().includes(term) || (r.lang || '').toLowerCase().includes(term) || (r.build || '').toLowerCase().includes(term));
  };

  const renderRepoList = () => {
    const rows = repoRows();
    if (!rows.length) return html`<p class="muted">${q.trim() ? 'No repository matches.' : 'No repositories are accessible to this GitHub App installation yet.'}</p>`;
    return rows.map((r) => html`<div class="list-row">
      <div>
        <div class="mono">${r.name}</div>
        <div class="sm muted">${r.vis} · ${r.lang || '–'} · pushed ${r.pushed} · detected: ${r.build}</div>
      </div>
      <a class="btn primary" href="#/apps/new/configure?repo=${encodeURIComponent(r.name)}&build=${encodeURIComponent(r.build)}&branch=${encodeURIComponent(r.defaultBranch || 'main')}&source=github_app">Deploy</a>
    </div>`);
  };

  const renderGithubTab = () => {
    if (!gh.connected) {
      return section('Connect GitHub or use a fallback', html`<div class="col gap16">
        <div class="row spread">
          <div>
            <div class="row" style="gap:8px">${st('off', 'GitHub not connected')}</div>
            <p class="sm muted mt6">Connect a private GitHub App in Settings to browse repositories and receive automatic push webhooks.</p>
          </div>
          ${server.role === 'admin' || api.MODE === 'mock'
            ? html`<a class="btn primary" href="#/settings">Connect GitHub in Settings</a>`
            : html`<span class="sm muted">Ask an administrator to connect GitHub in Settings.</span>`}
        </div>
        <div class="row">
          ${btn('Use a deploy key (single private repo)', { act: 'goTab', data: { i: '1' } })}
          ${btn('Deploy from a public repo URL', { act: 'goTab', data: { i: '2' } })}
        </div>
      </div>`);
    }
    if (!gh.installed) {
      return section('Choose repositories on GitHub', html`<div class="col gap16">
        <div class="row spread">
          <div>
            <div class="row" style="gap:8px">${st('warn', 'Awaiting repository access')}<b>${gh.appSlug}</b></div>
            <p class="sm muted mt6">Your GitHub App is created, but not installed on any account or repository yet.</p>
          </div>
          <div class="row">
            ${gh.installUrl ? html`<a class="btn primary" href="${gh.installUrl}" target="_blank" rel="noopener">Choose repositories on GitHub ↗</a>` : ''}
            ${btn(busy ? 'Refreshing…' : 'Refresh', { act: 'refreshRepos', disabled: busy })}
          </div>
        </div>
      </div>`);
    }
    return html`<div class="col gap16">
      <div class="row spread" style="margin:4px 0">
        <div class="row" style="gap:10px">
          ${st('ok', `GitHub connected as ${gh.account || gh.owner}`)}
          ${gh.manageUrl ? html`<a class="btn link" href="${gh.manageUrl}" target="_blank" rel="noopener">Manage access ↗</a>` : ''}
          ${btn(busy ? 'Refreshing…' : 'Refresh', { act: 'refreshRepos', kind: 'link', disabled: busy })}
        </div>
        <a class="btn link" href="#/apps">Cancel</a>
      </div>
      ${ghErr ? html`<p class="form-err" role="alert">${ghErr}</p>` : ''}
      <div class="field"><label for="q">Search repositories</label><input class="input lg" id="q" placeholder="owner/repo, language, Dockerfile…" value="${q}" autocomplete="off"></div>
      <div id="l">${renderRepoList()}</div>
    </div>`;
  };

  const renderDeployKeyTab = () => html`<div class="col gap24">
    ${section('Deploy key for a single private repo', html`<div class="col gap16">
      <p class="sm muted">Generate a dedicated Ed25519 SSH deploy key for one private repository. The private key stays encrypted with the server master key and never leaves this host.</p>
      <div class="row">
        <input class="input grow" id="dk-repo" placeholder="owner/private-repo or git@github.com:owner/private-repo.git" value="${dkRepo}"${busy ? ' disabled' : ''}>
        ${btn(busy ? 'Generating…' : 'Generate deploy key', { act: 'genKey', kind: 'primary', disabled: busy })}
      </div>
      ${dkErr ? html`<p class="form-err" role="alert">${dkErr}</p>` : ''}
    </div>`)}
    ${keys.length ? keys.map((k) => section(k.repo, html`<div class="col gap12">
      <div class="row spread">
        <span class="sm muted mono">${k.sshUrl} · ${k.fingerprint}</span>
        <div class="row">
          ${btn('Copy public key', { act: 'copyKey', data: { key: k.publicKey }, kind: 'link' })}
          ${btn('Remove', { act: 'rmKey', data: { id: k.id }, kind: 'link' })}
          <a class="btn primary" href="#/apps/new/configure?repo=${encodeURIComponent(k.repo)}&source=deploy_key&deployKeyId=${encodeURIComponent(k.id)}">Continue to configure →</a>
        </div>
      </div>
      <pre class="code" style="white-space:pre-wrap;word-break:break-all">${k.publicKey}</pre>
      <p class="hint">On GitHub: open your repository → <b>Settings → Deploy keys → Add deploy key</b>, paste this public key, and leave "Allow write access" unchecked.</p>
    </div>`)) : html`<p class="muted">No deploy keys generated yet.</p>`}
  </div>`;

  const renderPublicUrlTab = () => html`<div class="col gap24">
    ${section('Public repository URL', html`<div class="col gap16">
      <p class="sm muted">Deploy any public HTTPS Git repository without connecting a GitHub account. For GitHub repositories, opsbox detects the default branch and build method automatically.</p>
      <div class="row">
        <input class="input grow" id="pub-url" placeholder="https://github.com/owner/repo" value="${pubUrl}"${busy ? ' disabled' : ''}>
        ${btn(busy ? 'Checking…' : 'Check repository', { act: 'checkPub', kind: 'primary', disabled: busy })}
      </div>
      ${pubErr ? html`<p class="form-err" role="alert">${pubErr}</p>` : ''}
      ${pubInfo ? html`<div class="list-row" style="border-top:1px solid var(--ls);border-bottom:0;padding-top:12px">
        <div>
          <div class="mono">${pubInfo.repo}</div>
          <div class="sm muted">${pubInfo.vis} · ${pubInfo.lang || 'Git'} · branch ${pubInfo.defaultBranch} · detected: ${pubInfo.build}</div>
        </div>
        <a class="btn primary" href="#/apps/new/configure?repo=${encodeURIComponent(pubInfo.repo)}&build=${encodeURIComponent(pubInfo.build)}&branch=${encodeURIComponent(pubInfo.defaultBranch || 'main')}&source=public_url">Continue to configure →</a>
      </div>` : ''}
    </div>`)}
  </div>`;

  let imgRef = '';

  const renderDockerImageTab = () => html`<div class="col gap24">
    ${section('Prebuilt Docker image', html`<div class="col gap16">
      <p class="sm muted">Deploy any public registry image (Docker Hub, GHCR, Quay) directly without a build step.</p>
      <div class="row">
        <input class="input grow mono" id="img-ref" placeholder="nginx:1.27-alpine or ghcr.io/owner/app:latest" value="${imgRef}">
        ${btn('Continue to configure →', { act: 'goImage', kind: 'primary' })}
      </div>
    </div>`)}
  </div>`;

  const renderBody = () => {
    if (tab === 0) return renderGithubTab();
    if (tab === 1) return renderDeployKeyTab();
    if (tab === 2) return renderPublicUrlTab();
    return renderDockerImageTab();
  };

  const paint = () => {
    root.querySelectorAll('[data-t]').forEach((b) => b.setAttribute('aria-selected', String(+b.dataset.t === tab)));
    root.querySelector('#body').innerHTML = renderBody().toString();
    const qInput = root.querySelector('#q');
    if (qInput) {
      qInput.addEventListener('input', (e) => {
        q = e.target.value;
        const l = root.querySelector('#l');
        if (l) l.innerHTML = renderRepoList().toString();
      });
    }
  };

  return {
    nav: 'apps', title: 'New service',
    html: html`<main class="narrow">${header('New service', 'Pick a repository or fallback source. opsbox detects how to build it.')}
      <div class="tabs" role="tablist">${['GitHub', 'Deploy key', 'Public repo URL', 'Docker image', 'Templates'].map((t, i) => html`<button type="button" role="tab" aria-selected="${String(i === tab)}" data-t="${i}">${t}</button>`)}</div>
      <div id="body"></div></main>`,
    mount(r) {
      root = r;
      paint();
      root.querySelectorAll('[data-t]').forEach((b) => b.addEventListener('click', () => {
        const idx = +b.dataset.t;
        if (idx === 4) { location.hash = '#/apps/new/templates'; return; }
        tab = idx;
        paint();
      }));
      root.addEventListener('input', (e) => {
        if (e.target.id === 'dk-repo') dkRepo = e.target.value;
        if (e.target.id === 'pub-url') pubUrl = e.target.value;
        if (e.target.id === 'img-ref') imgRef = e.target.value;
      });
      bindActions(root, {
        goTab: (el) => { tab = +el.dataset.i; paint(); },
        goImage: () => {
          const val = (root.querySelector('#img-ref')?.value || imgRef || '').trim();
          if (!val) { toast('Enter a Docker image reference.', 'err'); return; }
          location.hash = `#/apps/new/configure?image=${encodeURIComponent(val)}&source=docker_image&build=${encodeURIComponent('Docker image')}`;
        },
        refreshRepos: async () => {
          busy = true; ghErr = ''; paint();
          try {
            gh = await api.getGithubStatus(true);
            if (gh.connected && gh.installed) repoRes = await api.getRepos('', { refresh: true });
            toast('Repositories refreshed.');
          } catch (e) { ghErr = e.message; }
          busy = false; paint();
        },
        genKey: async () => {
          const val = (root.querySelector('#dk-repo')?.value || dkRepo || '').trim();
          dkRepo = val; dkErr = ''; busy = true; paint();
          try {
            await api.createDeployKey(val);
            dkRepo = '';
            keys = await api.listDeployKeys();
            toast('Deploy key generated. Add the public key to your repository.');
          } catch (e) { dkErr = e.message; }
          busy = false; paint();
        },
        copyKey: async (el) => {
          try { await navigator.clipboard?.writeText(el.dataset.key || ''); toast('Public key copied.'); }
          catch { toast('Select and copy the public key from the box.'); }
        },
        rmKey: async (el) => {
          if (!await confirmDialog({ title: 'Remove this deploy key?', body: 'This deletes the encrypted private key from opsbox.', confirm: 'Remove', danger: true })) return;
          try { await api.deleteDeployKey(el.dataset.id); keys = await api.listDeployKeys(); toast('Deploy key removed.'); paint(); }
          catch (e) { dkErr = e.message; paint(); }
        },
        checkPub: async () => {
          const val = (root.querySelector('#pub-url')?.value || pubUrl || '').trim();
          pubUrl = val; pubErr = ''; pubInfo = null; busy = true; paint();
          try { pubInfo = await api.inspectPublicRepo(val); }
          catch (e) { pubErr = e.message; }
          busy = false; paint();
        },
      });
    },
  };
}
