// Database page (Databases part B): apps and logins, databases and users, extensions,
// table sizes, slow queries, connection pooler, major upgrades and data tools.
import { html, st, btn, table, section, header, bytes, bindActions, confirmDialog, formDialog, toast } from '../ui.js';
import { withCapacityOverride } from '../capacity.js';
import * as api from '../api.js';

const DATA_TOOLS = [
  ['adminer', 'Adminer', 'Browse and edit any database on this server. You sign in to it with a login from this page.'],
  ['pgweb', 'pgweb', 'Opens already connected to the main database.'],
];
const EXT_HELP = {
  vector: 'pgvector: store and search AI embeddings.',
  postgis: 'PostGIS: maps, distances and locations.',
  pg_stat_statements: 'Slow-query tracking for this page.',
  pgcrypto: 'Hashing and encryption functions.',
  'uuid-ossp': 'Generate UUIDs.',
  pg_trgm: 'Fast fuzzy text search.',
  citext: 'Case-insensitive text columns.',
  hstore: 'Key-value columns.',
};
const FLAVOR_NOTE = {
  vector: 'Not in this PostgreSQL image. Add a database from the "PostgreSQL + pgvector" template.',
  postgis: 'Not in this PostgreSQL image. Add a database from the "PostgreSQL + PostGIS" template.',
};

const fmtMs = (n) => (n >= 1000 ? `${(n / 1000).toFixed(1)} s` : `${n.toFixed(1)} ms`);

export async function view({ path, query }) {
  const id = decodeURIComponent((path.match(/^\/databases\/([^/]+)/) || [])[1] || '');
  let dbName = query.db || '';
  let d = await api.getDatabase(id, { database: dbName || undefined });
  let proj = await api.getProject().catch(() => ({ services: [] }));
  let root;
  let busy = '';

  const reload = async () => {
    d = await api.getDatabase(id, { database: dbName || undefined });
    proj = await api.getProject().catch(() => proj);
    paint();
  };
  const run = async (label, fn) => {
    busy = label; paint();
    try { return await fn(); } catch (e) { toast(e.message || String(e), 'err'); return null; } finally { busy = ''; await reload(); }
  };

  const tools = () => (proj.services || []).filter((s) => s.kind === 'tool' && DATA_TOOLS.some(([t]) => t === s.templateId));

  const overview = () => {
    const l = d.live;
    if (!l) return section('Status', html`<p class="muted">${d.liveError || 'Details are not available right now.'}</p>`);
    const pctUsed = Math.min(100, Math.round((l.connections.current / Math.max(1, l.connections.max)) * 100));
    return section('Overview', html`<dl class="sm" style="display:grid;grid-template-columns:170px 1fr;gap:8px">
      <dt class="muted">Server</dt><dd class="mono">${d.flavor} · ${l.serverVersion}</dd>
      <dt class="muted">Name on the network</dt><dd class="mono">${d.id}:5432</dd>
      <dt class="muted">Connections</dt><dd>${l.connections.current} of ${l.connections.max} in use${pctUsed >= 80 ? html` ${st('warn', 'almost full')}` : ''}</dd>
      <dt class="muted">Databases</dt><dd>${l.databases.length} · ${bytes(l.databases.reduce((a, x) => a + x.sizeBytes, 0))}</dd>
      <dt class="muted">Data volume</dt><dd class="mono">${d.dataVolume}</dd>
    </dl>
    <p class="sm muted mt10">Each app you connect gets its own database and login. Other apps cannot open it.</p>`);
  };

  const appsSection = () => {
    const rows = (d.apps || []).map((a) => [
      html`<button type="button" class="btn link mono" data-act="open" data-id="${a.app}">${a.app}</button>`,
      html`<span class="mono">${a.database}</span>`,
      html`<span class="mono">${a.user}</span>`,
      a.connections == null ? html`<span class="muted">—</span>` : html`${a.connections}`,
      d.pooler.enabled ? btn(a.pooled ? 'Pooled' : 'Direct', { act: 'pooled', data: { app: a.app, on: a.pooled ? '' : '1' }, kind: 'link' }) : html`<span class="muted">Direct</span>`,
      html`<span class="row end">${btn('Show connection string', { act: 'reveal', data: { app: a.app, pooled: a.pooled ? '1' : '' }, kind: 'link' })}${btn('New password', { act: 'rotateapp', data: { app: a.app }, kind: 'link' })}${btn('Remove', { act: 'rmapp', data: { app: a.app }, kind: 'link' })}</span>`,
    ]);
    return section('Apps and logins', html`${rows.length
      ? table('Apps using this database', [['App'], ['Database'], ['Login'], ['Open connections'], ['Route'], ['', 'r']], rows)
      : html`<p class="muted">No app is using this database yet. Open an app's <b>Variables</b> tab and choose <b>Connect database</b>: it gets its own database and a login that can only open that database.</p>`}
      <div class="row mt12">${btn('Show administrator connection string', { act: 'reveal', data: { app: '' }, kind: '' })}</div>
      <p class="sm muted mt6">Connection strings are shown after you enter a fresh two-factor code, and each time is recorded in the audit log.</p>`);
  };

  const dbsSection = () => {
    const l = d.live; if (!l) return '';
    return section('Databases and users', html`
      ${table('Databases', [['Name'], ['Owner'], ['Size', 'r'], ['', 'r']], l.databases.map((x) => [html`<span class="mono">${x.name}</span>`, html`<span class="mono">${x.owner}</span>`, bytes(x.sizeBytes), btn(x.name === l.database ? 'Showing' : 'Inspect tables', { act: 'inspect', data: { db: x.name }, kind: 'link', disabled: x.name === l.database })]))}
      <div class="mt16"></div>
      ${table('Users and what they can open', [['Login'], ['Can open'], ['Connections', 'r']], l.users.map((u) => [
        html`<span class="mono">${u.name}</span>${u.superuser ? html` <span class="tag-sm">full access</span>` : ''}${u.canLogin ? '' : html` <span class="tag-sm">group</span>`}`,
        u.superuser ? html`<span class="muted">Everything</span>` : html`${(u.databases || []).map((x) => html`<span class="mono tag-sm">${x}</span> `)}`,
        String(u.connections),
      ]))}`);
  };

  const extSection = () => {
    const l = d.live; if (!l) return '';
    return section(`Extensions in ${l.database}`, table('Extensions', [['Extension'], ['What it does'], ['Status'], ['', 'r']], l.extensions.map((e) => {
      const missing = !e.defaultVersion && !e.installed;
      return [
        html`<span class="mono">${e.name}</span>`,
        html`<span class="sm">${EXT_HELP[e.name] || e.comment || ''}</span>`,
        e.installed ? st('ok', `on · ${e.installedVersion || ''}`) : missing ? html`<span class="sm muted">${FLAVOR_NOTE[e.name] || 'Not available'}</span>` : st('off', 'off'),
        missing ? '' : btn(e.installed ? 'Turn off' : 'Turn on', { act: 'ext', data: { name: e.name, on: e.installed ? '' : '1' }, kind: e.installed ? 'link' : '' }),
      ];
    })), '');
  };
  const extNotAvailable = () => {
    const l = d.live; if (!l) return '';
    const have = new Set(l.extensions.map((e) => e.name));
    const wanted = ['vector', 'postgis'].filter((n) => !have.has(n));
    return wanted.length ? html`<p class="sm muted">${wanted.map((n) => FLAVOR_NOTE[n]).join(' ')}</p>` : '';
  };

  const tablesSection = () => {
    const l = d.live; if (!l) return '';
    return section(`Biggest tables in ${l.database}`, l.tables.length
      ? table('Biggest tables', [['Table'], ['Rows (about)', 'r'], ['Size', 'r']], l.tables.map((t) => [html`<span class="mono">${t.schema}.${t.table}</span>`, String(t.rowsEstimate), bytes(t.totalBytes)]))
      : html`<p class="muted">No tables yet in this database.</p>`);
  };

  const slowSection = () => {
    const l = d.live; if (!l) return '';
    const s = l.statStatements;
    let body;
    if (!s.loaded) body = html`<p class="muted">Slow-query tracking is not loaded in this database yet. Redeploy it once from <b>Apps</b> (a brief restart), then turn it on here.</p>`;
    else if (!s.enabled) body = html`<p class="muted">Tracking is available but off.</p><div>${btn('Turn on slow-query tracking', { act: 'ext', data: { name: 'pg_stat_statements', on: '1' }, kind: 'primary' })}</div>`;
    else if (!l.slowQueries.length) body = html`<p class="muted">Nothing recorded yet. Queries appear here as your apps run them.</p>`;
    else body = table('Slowest queries', [['Query'], ['Calls', 'r'], ['Average', 'r'], ['Total', 'r']], l.slowQueries.map((q) => [html`<span class="mono sm" style="white-space:pre-wrap;word-break:break-word">${q.query}</span>`, String(q.calls), fmtMs(q.meanMs), fmtMs(q.totalMs)]));
    return section('Slowest queries', body);
  };

  const poolerSection = () => {
    const p = d.pooler;
    return section('Connection pooler (PgBouncer)', html`
      <p class="sm muted">Lets many app connections share a few database connections. Turn it on if you see "too many connections". Optional: most small apps do not need it.</p>
      <dl class="sm" style="display:grid;grid-template-columns:170px 1fr;gap:8px">
        <dt class="muted">Status</dt><dd>${p.enabled ? st('ok', `on · ${p.mode} mode`) : st('off', 'off')}</dd>
        ${p.enabled ? html`<dt class="muted">Name on the network</dt><dd class="mono">${p.host}:${p.port}</dd>` : ''}
      </dl>
      <div class="row mt12">${p.enabled ? btn('Turn off', { act: 'pooler', data: { on: '' }, disabled: !!busy }) : btn(busy === 'pooler' ? 'Starting…' : 'Turn on', { act: 'pooler', data: { on: '1' }, kind: 'primary', disabled: !!busy || !d.running })}</div>
      ${p.enabled ? html`<p class="sm muted mt6">Use the <b>Route</b> column above to send an app through the pooler, then redeploy that app.</p>` : ''}`);
  };

  const upgradeSection = () => {
    const ups = d.upgrades || [];
    return section('Version', html`<p class="sm">Running <b class="mono">${d.version}</b>.</p>
      ${ups.length
        ? html`<p class="sm muted">Upgrading to a newer major version takes a safety backup first, then moves your data to a new volume and checks it. If anything looks wrong nothing is switched. The database is offline for a few minutes.</p><div class="row">${ups.map((u) => btn(`Upgrade to PostgreSQL ${u.major}…`, { act: 'upgrade', data: { v: u.version }, disabled: !!busy || !d.running }))}</div>`
        : html`<p class="sm muted">This is the newest major version offered.</p>`}`);
  };

  const toolsSection = () => {
    const t = tools();
    const have = new Set(t.map((s) => s.templateId));
    return section('Data tools', html`
      <p class="sm muted">Look at and edit your data in the browser. Tools open only while you are signed in to opsbox and are never published on the internet.</p>
      ${t.length ? html`<div class="row mb10">${t.map((s) => html`<a class="btn primary" href="/tools/${encodeURIComponent(s.id)}/" target="_blank" rel="noopener">Open ${s.id}</a>`)}</div>` : ''}
      <div class="row">${DATA_TOOLS.filter(([tid]) => !have.has(tid)).map(([tid, name, hint]) => html`<span>${btn(`Add ${name}`, { act: 'addtool', data: { t: tid }, disabled: !!busy })}<span class="sm muted" style="margin-left:6px">${hint}</span></span>`)}</div>`);
  };

  const body = () => html`
    ${d.liveError && !d.live ? html`<div class="note warn" role="status">${d.liveError}</div>` : ''}
    <div class="cols"><div class="w3">
      ${overview()}${appsSection()}${dbsSection()}${extSection()}${extNotAvailable()}${tablesSection()}${slowSection()}
    </div><div>
      ${upgradeSection()}${poolerSection()}${toolsSection()}
    </div></div>`;

  const paint = () => { if (root) root.querySelector('#dbbody').innerHTML = body().toString(); };

  const code = async (title, bodyText) => {
    const out = await formDialog({ title, body: bodyText, fields: [{ name: 'code', label: 'Two-factor code', placeholder: '123456' }], submit: 'Show' });
    return out ? out.code.trim() : null;
  };

  return {
    nav: 'apps',
    title: `Database ${id}`,
    html: html`<main>${header(id, `${d.flavor || 'PostgreSQL'} · ${d.version}`, html`<a class="btn" href="#/apps?s=${encodeURIComponent(id)}">Back to Apps</a>`)}<div id="dbbody">${body()}</div></main>`,
    mount(r) {
      root = r;
      bindActions(root, {
        open: (el) => { location.hash = `#/apps?s=${encodeURIComponent(el.dataset.id)}`; },
        inspect: async (el) => { dbName = el.dataset.db; await reload(); },
        reveal: async (el) => {
          const app = el.dataset.app || '';
          const c = await code(app ? `Connection string for ${app}` : 'Administrator connection string', 'Enter the current code from your authenticator app. Each code works once.');
          if (!c) return;
          try {
            const r2 = await api.revealConnectionString(id, { code: c, app, pooled: el.dataset.pooled === '1' });
            await formDialog({ title: `Connection string (${r2.login})`, body: r2.note, fields: [{ name: 'url', label: 'Connection string', value: r2.connectionString }], submit: 'Done' });
          } catch (e) { toast(e.message, 'err'); }
        },
        rotateapp: async (el) => {
          const app = el.dataset.app;
          if (!await confirmDialog({ title: `New password for ${app}'s login?`, body: `${app} keeps running with the old password until you redeploy it.`, confirm: 'Change password' })) return;
          const res = await run('rotate', () => api.rotateAppLogin(id, app));
          if (res) toast(`New password set for ${app}. Redeploy ${app} to use it.`);
        },
        rmapp: async (el) => {
          const app = el.dataset.app;
          const row = (d.apps || []).find((a) => a.app === app);
          const out = await formDialog({
            title: `Remove ${app}'s login`,
            body: 'The login stops working. You can keep the database and its data, or delete both for good.',
            fields: [{ name: 'what', label: 'What to remove', options: ['Login only (keep the data)', 'Login and database (delete the data)'], value: 'Login only (keep the data)' }, { name: 'confirm', label: `To delete the data, type ${row ? row.database : 'the database name'}`, placeholder: 'Leave empty to keep the data' }],
            submit: 'Remove', danger: true,
          });
          if (!out) return;
          const drop = out.what.startsWith('Login and');
          if (await run('rmapp', () => api.removeAppLogin(id, app, { dropDatabase: drop, confirm: out.confirm.trim() }))) toast(drop ? 'Database deleted.' : 'Login removed. The data is kept.');
        },
        pooled: async (el) => {
          const on = el.dataset.on === '1';
          const res = await run('pooled', () => api.setAppPooled(id, el.dataset.app, on));
          if (res) toast(`${el.dataset.app} now connects ${on ? 'through the pooler' : 'directly'}.${res.needsRedeploy ? ' Redeploy it to apply.' : ''}`);
        },
        ext: async (el) => {
          const on = el.dataset.on === '1';
          if (!on && !await confirmDialog({ title: `Turn off ${el.dataset.name}?`, body: 'Anything that uses it in this database stops working.', confirm: 'Turn off', danger: true })) return;
          let override={};
          if(on && ['vector','postgis','pg_stat_statements'].includes(el.dataset.name)) {
            const memory=(proj.services.find((s)=>s.id===id)?.memoryLimitMb || 256)+64;
            const out=await formDialog({title:`Enable ${el.dataset.name}`,body:'This reserves 64 MB more memory for the database.',fields:[],submit:'Enable extension',capacity:{memory,fixed:true,request:()=>({feature:'database',memory_mb:memory,replacing:id})}});
            if(!out)return;override=out.capacity;
          }
          if (await run('ext', () => withCapacityOverride((b) => api.setExtension(id, el.dataset.name, b), { ...override, enabled: on, database: d.live?.database }))) toast(`${el.dataset.name} is ${on ? 'on' : 'off'}.`);
        },
        pooler: async (el) => {
          const on = el.dataset.on === '1';
          if (!on && !await confirmDialog({ title: 'Turn off the pooler?', body: 'Apps using the pooler are pointed back at the database directly. Redeploy them afterwards.', confirm: 'Turn off', danger: true })) return;
          let override={};
          if(on){const out=await formDialog({title:'Add connection pooler',body:'The pooler needs 128 MB of physical memory. Swap does not count.',fields:[],submit:'Start pooler',capacity:{memory:128,fixed:true,request:()=>({feature:'template',database:true,memory_mb:128})}});if(!out)return;override=out.capacity;}
          if (await run('pooler', () => withCapacityOverride((b) => api.setPooler(id, b), { ...override, enabled: on, mode: 'transaction' }))) toast(on ? 'Connection pooler is running.' : 'Connection pooler removed.');
        },
        upgrade: async (el) => {
          const v = el.dataset.v;
          if (!await confirmDialog({ title: `Upgrade ${id} to PostgreSQL ${parseInt(v, 10) || v}?`, body: `${id} is stopped for a few minutes. opsbox first takes a safety backup, moves your data to a new volume and checks it. If anything looks wrong, nothing is switched and ${id} keeps running as before. Your current data volume is kept until you delete it.`, confirm: 'Back up and upgrade' })) return;
          const res = await run('upgrade', () => withCapacityOverride((b) => api.upgradeDatabase(id, b), { version: v, confirm: true }));
          if (res) toast(`Upgraded to ${res.result?.version || v}. Old data kept in ${res.result?.oldVolume || 'the previous volume'}.`);
        },
        addtool: async (el) => {
          const t = el.dataset.t;
          const name = `${id}-${t}`;
          const cfg = { name, internalOnly: false, backup: false };
          if (t === 'pgweb') cfg.databaseUrl = `\${{ ${id}.DATABASE_URL }}`;
          if (await run('tool', () => api.addTemplate(t, cfg))) toast(`${t} is starting. Open it from this page when it is running.`);
        },
      });
    },
  };
}
