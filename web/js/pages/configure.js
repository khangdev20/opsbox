import { html, header, toast, setBusy, showFormError } from '../ui.js';
import { capacityFields, bindCapacity, capacityPayload } from '../capacity.js';
import * as api from '../api.js';

export async function view({ query }) {
  const sourceParam = query.source || '';
  const image = (query.image || '').trim();
  const deployKeyId = (query.deployKeyId || '').trim();
  const isImage = sourceParam === 'docker_image' || !!image;
  const sourceKind = isImage
    ? 'docker_image'
    : sourceParam === 'deploy_key' || deployKeyId
      ? 'deploy_key'
      : sourceParam === 'public_url' || sourceParam === 'git_url'
        ? 'git_url'
        : 'github';

  const repo = isImage ? image : (query.repo || (api.MODE === 'http' ? '' : 'acme/shop')).trim();
  if (api.MODE === 'http' && !isImage && !repo) {
    return {
      nav: 'apps', title: 'Configure',
      html: html`<main class="narrow">${header('Choose a repository', 'Select a deployment source before configuring your app.')}<a class="btn primary" href="#/apps/new">Choose a repository</a></main>`,
    };
  }
  const build = query.build || (isImage ? 'Docker image' : 'auto');
  const branch = query.branch || 'main';
  const rawName = (isImage ? image.split(':')[0].split('/').pop() : repo.replace(/\.git$/, '').split('/').filter(Boolean).pop()) || 'app';
  const def = rawName.toLowerCase().replace(/[^a-z0-9-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 63) || 'app';
  const builds = ['auto', 'Dockerfile', 'static site', 'Nixpacks'];
  const buildSel = builds.find((b) => build.toLowerCase().startsWith(b.toLowerCase())) || 'auto';
  const subtitle = isImage
    ? `Pull prebuilt image ${image}`
    : deployKeyId
      ? 'Using SSH deploy key'
      : sourceKind === 'git_url'
        ? 'Public Git repository URL'
        : 'GitHub repository';

  return {
    nav: 'apps', title: 'Configure',
    html: html`<main class="narrow">${header(`Configure ${repo}`, `${subtitle} · Review settings, then deploy.`)}
      <form id="f">
        <div class="field"><label for="name">Service name</label><input class="input" id="name" name="name" value="${def}" required><span class="hint">Lowercase letters, digits, and dashes (used for container & internal DNS).</span></div>
        ${isImage ? '' : html`
          <div class="field"><label for="branch">Branch</label><input class="input" id="branch" name="branch" value="${branch}"></div>
          <div class="field"><label for="build">Build method</label><select class="select" id="build" name="build">${builds.map((b) => html`<option value="${b}"${b === buildSel ? ' selected' : ''}>${b === 'auto' ? 'Auto-detect (Dockerfile / static / Nixpacks)' : b}</option>`)}</select><span class="hint">Detected: ${build}</span></div>
        `}
        <div class="field"><label for="port">Container port</label><input class="input" id="port" name="port" value="3000" inputmode="numeric"><span class="hint">HTTP listening port inside the container (use 0 for background workers).</span></div>
        <div class="field"><label for="healthPath">Health check path</label><input class="input" id="healthPath" name="healthPath" value="/"><span class="hint">Must return HTTP 2xx/3xx (or enter "tcp" for non-HTTP TCP check).</span></div>
        <div class="field"><label for="domain">Domain (optional)</label><input class="input" id="domain" name="domain" value="" placeholder="app.example.com"><span class="hint">Optional hostname routed via the reverse proxy once healthy.</span></div>
        ${isImage ? '' : html`<div class="item-row"><label><input type="checkbox" id="autoDeploy" name="autoDeploy" checked><span><b>Auto-deploy on push</b><span class="sm muted">Trigger a new deploy when commits arrive on ${branch}.</span></span></label></div>`}
        <div class="field"><label for="vars">Environment variables</label><textarea class="input" id="vars" name="vars" rows="4" placeholder="KEY=value (one per line)" style="height:auto;font-family:var(--mono)"></textarea></div>
        ${capacityFields(128)}
        <p id="err" class="form-err" role="alert" tabindex="-1"></p>
        <div class="row wiz-actions"><button class="btn primary" type="submit">Deploy</button><a class="btn link" href="#/apps/new">Back</a></div>
      </form></main>`,
    mount(root) {
      const f = root.querySelector('#f');
      this._capacity=bindCapacity(root, () => [{feature:'app', memory_mb:capacityPayload(root).memoryLimitMb}, ...(!isImage ? [{feature:'build'}] : [])]);
      if(!isImage) api.checkCapacity({feature:'build'}).then(({fit})=>{const b=root.querySelector('#build');if(fit.status==='blocked'&&b){b.disabled=true;b.title=[...(fit.reasons||[]),...(fit.suggestions||[])].join(' ');}});
      f.addEventListener('submit', async (e) => {
        e.preventDefault();
        const b = f.querySelector('button[type="submit"]');
        if (b.hasAttribute('data-busy')) return;
        setBusy(b, true); root.setAttribute('data-capacity-busy', '');
        root.querySelector('#err').textContent = '';
        const vars = f.vars.value.split('\n').map((l) => l.trim()).filter(Boolean).map((l) => {
          const i = l.indexOf('=');
          return i < 0 ? [l, ''] : [l.slice(0, i), l.slice(i + 1)];
        });
        const payload = {
          ...capacityPayload(root),
          sourceKind,
          name: f.name.value.trim(),
          port: Number(f.port.value.trim() || 0),
          healthPath: f.healthPath.value.trim() || '/',
          domain: f.domain.value.trim(),
          vars,
        };
        if (isImage) {
          payload.image = image;
        } else {
          payload.repo = repo;
          payload.branch = f.branch.value.trim() || 'main';
          payload.build = f.build.value;
          payload.autoDeploy = !!f.autoDeploy?.checked;
          if (deployKeyId) payload.deployKeyId = deployKeyId;
        }
        try {
          const res = await api.createService(payload);
          toast('Deploy started.');
          const qs = new URLSearchParams({ s: payload.name });
          if (res?.deployId) qs.set('log', res.deployId);
          location.hash = `#/apps?${qs.toString()}`;
        } catch (err) {
          // Only the button and the error line change; every field stays as typed.
          setBusy(b, false); root.removeAttribute('data-capacity-busy');
          showFormError(root.querySelector('#err'), err.message);
          this._capacity?.refresh();
        }
      });
    },
    unmount() { this._capacity?.dispose(); },
  };
}
