import { html, raw, btn, tabs, table, section, header, bindActions, toast } from '../ui.js';
import * as api from '../api.js';

const YML = 'services:\n  web:\n    image: ghcr.io/acme/shop:latest\n    ports: ["3000:3000"]\n  cache:\n    image: redis:7\n';
export async function view() {
  let pick = api.TEMPLATES.databases[0], root;
  const rows = (list) => table('Templates', [['Name'], ['Needs'], ['', 'r']], list.map((t) => { const r = [html`<b>${t.name}</b><div class="sm muted">${t.desc}</div>`, html`<span class="mono sm">${t.res}</span>`, btn('Add →', { act: 'pick', data: { id: t.id }, kind: t.id === pick.id ? 'primary' : '' })]; r.meta = { cls: t.id === pick.id ? 'sel' : '' }; return r; }));
  const form = () => section(`Add ${pick.name}`, html`
    <div class="field"><label for="tn">Name</label><input class="input" id="tn" value="${pick.id}-main"><span class="hint">Other services reach it at <span class="mono">${pick.id}-main:${pick.port}</span>.</span></div>
    <div class="field"><label for="tv">Version</label><select class="select" id="tv">${pick.versions.map((v) => html`<option>${v}</option>`)}</select></div>
    <div class="field"><label for="td">Disk for data</label><select class="select" id="td">${['1 GB', '5 GB', '20 GB'].map((v) => html`<option>${v}</option>`)}</select></div>
    <div class="item-row"><label><input type="checkbox" id="tb" checked><span><b>Back up daily at 02:00</b></span></label></div>
    <div class="item-row"><label><input type="checkbox" id="ti" checked><span><b>Only reachable from other services</b><span class="sm muted">Not exposed to the internet</span></span></label></div>
    <p class="sm muted">Password is generated, stored encrypted and shown once as DATABASE_URL.</p>
    <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p>
    <div>${btn(`Add ${pick.name}`, { act: 'add', kind: 'primary' })}</div>`);
  const paint = () => { root.querySelector('#db').innerHTML = rows(api.TEMPLATES.databases).toString(); root.querySelector('#tl').innerHTML = rows(api.TEMPLATES.tools).toString(); root.querySelector('#fm').innerHTML = form().toString(); };
  return {
    nav: 'apps', title: 'Templates',
    html: html`<main>${header('New service', 'Databases and tools run as containers in shop-platform', html`<a class="btn" href="#/apps">Cancel</a>`)}
      <div class="tabs" role="tablist">${['GitHub', 'Public repo URL', 'Docker image', 'Templates'].map((t, i) => html`<button type="button" role="tab" aria-selected="${i === 3}" data-act="src" data-i="${i}">${t}</button>`)}</div>
      <div class="cols"><div class="w3"><div id="db"></div><div id="tl" style="margin-top:32px"></div>
        ${section('Your own Compose file', html`<p class="muted sm">Paste a docker-compose.yml. Each service appears on the Apps graph with logs, variables and rollback.</p><div class="editor" style="border-radius:2px"><textarea id="yml" spellcheck="false" aria-label="Compose file" style="min-height:170px">${YML}</textarea></div><p id="cerr" role="alert" style="color:var(--danger);font-size:13px"></p><div>${btn('Deploy compose file', { act: 'compose', kind: 'primary' })}</div>`)}
      </div><div id="fm"></div></div></main>`,
    mount(r) {
      root = r; paint();
      bindActions(root, {
        src: (el) => { if (+el.dataset.i !== 3) location.hash = '#/apps/new'; },
        pick: (el) => { pick = [...api.TEMPLATES.databases, ...api.TEMPLATES.tools].find((t) => t.id === el.dataset.id); paint(); },
        add: async () => {
          try { const r = await api.addTemplate(pick.id, { name: root.querySelector('#tn').value, version: root.querySelector('#tv').value, disk: root.querySelector('#td').value, backup: root.querySelector('#tb').checked }); toast(`${pick.name} is running.`); location.hash = `#/apps?s=${r.id}`; }
          catch (e) { root.querySelector('#err').textContent = e.message; }
        },
        compose: async () => { try { const r = await api.deployCompose(root.querySelector('#yml').value); toast(`Deployed ${r.names.join(', ')}.`); location.hash = '#/apps'; } catch (e) { root.querySelector('#cerr').textContent = e.message; } },
      });
    },
  };
}
