import { html, btn, table, section, header, bindActions, formDialog, toast } from '../ui.js';
import * as api from '../api.js';

const YML = `services:
  web:
    image: nginx:1.27-alpine
    ports: ["127.0.0.1:3000:80"]
    depends_on:
      - cache
  cache:
    image: redis:7-alpine
`;

const NEW_TABS = [
  ['GitHub', '#/apps/new'],
  ['Deploy key', '#/apps/new?tab=deploy-key'],
  ['Public repo URL', '#/apps/new?tab=public'],
  ['Docker image', '#/apps/new?tab=image'],
  ['Templates', '#/apps/new/templates'],
];

export async function view() {
  const catalog = await api.getTemplates();
  const databases = catalog.databases || [];
  const tools = catalog.tools || [];
  const all = [...databases, ...tools];
  let pick = all[0] || { id: 'postgres', name: 'PostgreSQL', versions: ['16.4'], port: 5432 };
  let busy = '';
  let root;

  const rows = (heading, list) => table(heading, [['Name'], ['Needs'], ['', 'r']], list.map((t) => {
    const r = [
      html`<b>${t.name}</b><div class="sm muted">${t.description || t.desc}</div>`,
      html`<span class="mono sm">${t.resources || t.res}</span>`,
      btn('Select →', { act: 'pick', data: { id: t.id }, kind: t.id === pick.id ? 'primary' : '' }),
    ];
    r.meta = { cls: t.id === pick.id ? 'sel' : '' };
    return r;
  }));

  const form = () => {
    const backupDefault = pick.backup?.enabledByDefault ?? (pick.category === 'databases');
    const internalDefault = pick.internalByDefault ?? true;
    const isUmami = pick.id === 'umami' || pick.requiresDatabase;
    return section(`Add ${pick.name}`, html`
      <div class="field"><label for="tn">Name</label><input class="input" id="tn" value="${pick.id}-main" autocomplete="off"><span class="hint">Other services reach it at <span class="mono">${pick.id}-main:${pick.port}</span> on the project network.</span></div>
      <div class="field"><label for="tv">Version</label><select class="select" id="tv">${(pick.versions || []).map((v) => html`<option>${v}</option>`)}</select></div>
      <div class="field"><label for="td">Disk for data</label><select class="select" id="td">${['1 GB', '5 GB', '20 GB'].map((v) => html`<option>${v}</option>`)}</select></div>
      ${isUmami ? html`<div class="field"><label for="tdb">PostgreSQL DATABASE_URL</label><input class="input mono" id="tdb" placeholder="postgresql://app:password@pg-main:5432/app" autocomplete="off"><span class="hint">Umami stores analytics in an existing PostgreSQL instance.</span></div>` : ''}
      <div class="item-row"><label><input type="checkbox" id="tb"${backupDefault ? ' checked' : ''}><span><b>Back up daily at 02:00</b></span></label></div>
      <div class="item-row"><label><input type="checkbox" id="ti"${internalDefault ? ' checked' : ''}><span><b>Only reachable from other services</b><span class="sm muted">Keep off the internet (project network only)</span></span></label></div>
      <p class="sm muted">Credentials are generated, encrypted at rest, and shown once upon creation.</p>
      <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p>
      <div>${btn(busy === 'tpl' ? `Starting ${pick.name}…` : `Add ${pick.name}`, { act: 'add', kind: 'primary', disabled: busy === 'tpl' })}</div>`);
  };

  const paint = () => {
    if (!root) return;
    root.querySelector('#db').innerHTML = rows('Databases', databases).toString();
    root.querySelector('#tl').innerHTML = rows('Tools', tools).toString();
    root.querySelector('#fm').innerHTML = form().toString();
  };

  return {
    nav: 'apps',
    title: 'Templates',
    html: html`<main>${header('New service', 'Databases, tools, or a Docker Compose stack', html`<a class="btn" href="#/apps">Cancel</a>`)}
      <div class="tabs" role="tablist">${NEW_TABS.map(([label], i) => html`<button type="button" role="tab" aria-selected="${i === 4}" data-act="src" data-i="${i}">${label}</button>`)}</div>
      <div class="cols"><div class="w3"><div id="db"></div><div id="tl" style="margin-top:32px"></div>
        ${section('Your own Compose file', html`
          <p class="muted sm">Paste a docker-compose.yml. Validated via <span class="mono">docker compose config</span> and attached to your project network.</p>
          <div class="field"><label for="cn">Stack name <span class="muted">(optional)</span></label><input class="input" id="cn" placeholder="shop" autocomplete="off"></div>
          <div class="editor" style="border-radius:8px"><textarea id="yml" spellcheck="false" aria-label="Compose file" style="min-height:190px">${YML}</textarea></div>
          <p id="cerr" role="alert" style="color:var(--danger);font-size:13px"></p>
          <div>${btn(busy === 'cmp' ? 'Deploying Compose stack…' : 'Deploy compose file', { act: 'compose', kind: 'primary', disabled: busy === 'cmp' })}</div>`)}
      </div><div id="fm"></div></div></main>`,
    mount(r) {
      root = r;
      paint();
      bindActions(root, {
        src: (el) => {
          const i = +el.dataset.i;
          if (i !== 4 && NEW_TABS[i]) location.hash = NEW_TABS[i][1];
        },
        pick: (el) => {
          pick = all.find((t) => t.id === el.dataset.id) || pick;
          paint();
        },
        add: async () => {
          const errEl = root.querySelector('#err');
          if (errEl) errEl.textContent = '';
          const cfg = {
            name: root.querySelector('#tn')?.value.trim(),
            version: root.querySelector('#tv')?.value,
            disk: root.querySelector('#td')?.value,
            backup: !!root.querySelector('#tb')?.checked,
            internalOnly: !!root.querySelector('#ti')?.checked,
            databaseUrl: root.querySelector('#tdb')?.value.trim(),
          };
          busy = 'tpl';
          paint();
          try {
            const res = await api.addTemplate(pick.id, cfg);
            toast(`${pick.name} is running.`);
            if (res.connectionUrl) {
              const out = await formDialog({
                title: `${pick.name} is running`,
                body: `Copy this ${res.connectionVar || 'connection URL'} now — OpsBox encrypts it at rest and will not show the plaintext secret again.`,
                fields: [{ name: 'url', label: res.connectionVar || 'Connection URL', value: res.connectionUrl }],
                submit: 'Copy & open Apps',
              });
              if (out?.url) {
                try { await navigator.clipboard?.writeText(out.url); toast('Copied to clipboard.'); } catch { /* ignore */ }
              }
            }
            location.hash = `#/apps?s=${encodeURIComponent(res.id)}`;
          } catch (e) {
            busy = '';
            paint();
            const curErr = root.querySelector('#err');
            if (curErr) curErr.textContent = e.message;
          }
        },
        compose: async () => {
          const stack = root.querySelector('#cn')?.value.trim();
          const yaml = root.querySelector('#yml')?.value || '';
          const cerr = root.querySelector('#cerr');
          if (cerr) cerr.textContent = '';
          busy = 'cmp';
          const cbtn = root.querySelector('[data-act="compose"]');
          if (cbtn) cbtn.textContent = 'Deploying Compose stack…';
          try {
            const res = await api.deployCompose(yaml, { stack });
            toast(`Deployed ${(res.names || []).join(', ')}.`);
            location.hash = res.names?.[0] ? `#/apps?s=${encodeURIComponent(res.names[0])}` : '#/apps';
          } catch (e) {
            busy = '';
            if (cbtn) cbtn.textContent = 'Deploy compose file';
            if (cerr) cerr.textContent = e.message;
          }
        },
      });
    },
  };
}
