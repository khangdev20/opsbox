import { html, raw, esc, ago, st, btn, tabs, table, section, header, bindActions, confirmDialog, askForm, toast } from '../ui.js';
import * as api from '../api.js';
import { listeners } from '../store.js';

const KIND = { active: ['ok', 'running'], building: ['build', 'building'], failed: ['fail', 'failed'], idle: ['off', 'idle'] };
const DST = { live: ['ok', 'live'], replaced: ['off', 'replaced'], failed: ['fail', 'failed'], building: ['build', 'building'] };

export async function view({ query }) {
  let proj = await api.getProject();
  let sel = query.s || proj.services[0]?.id || 'web';
  let tab = query.tab === 'domains' ? 2 : 0;
  let logDeploy = query.log || null;
  let logLines = [];
  let logLoading = false;
  let closeLogWs = null;
  let root;
  let doms = null;
  let entered = false;

  const svc = () => proj.services.find((s) => s.id === sel) || proj.services[0];

  const stopWs = () => {
    if (closeLogWs) {
      closeLogWs();
      closeLogWs = null;
    }
  };

  const openLogFor = async (deployId) => {
    stopWs();
    logDeploy = deployId;
    if (!deployId) {
      logLines = [];
      paint();
      return;
    }
    const s = svc();
    const dep = s?.deploys?.find((d) => d.id === deployId);
    if (!s || !dep) return;
    logLoading = true;
    paint();
    try {
      const res = await api.fetchBuildLog(s, dep);
      if (logDeploy !== deployId) return;
      logLines = (res.lines || []).map((l) => l.line || String(l));
      logLoading = false;
      paint();
      if (dep.status === 'building' && api.MODE === 'http') {
        closeLogWs = api.streamDeployLog(
          deployId,
          (msg) => {
            if (msg.line && !logLines.includes(msg.line)) {
              logLines.push(msg.line);
            } else if (msg.line) {
              logLines.push(msg.line);
            }
            const pre = root?.querySelector('#build-log');
            if (pre) {
              pre.textContent = logLines.join('\n');
              pre.scrollTop = pre.scrollHeight;
            }
          },
          () => {
            stopWs();
            reload();
          },
        );
      }
    } catch (err) {
      logLoading = false;
      logLines = [`Could not load build log: ${err.message}`];
      paint();
    }
  };

  const graph = () => {
    if (!proj.services.length) {
      return section('Services', html`<div class="col gap12"><p class="muted">No services deployed yet.</p><div><a class="btn primary" href="#/apps/new">Deploy your first service</a></div></div>`);
    }
    const edges = [['web', 'api'], ['web', 'postgres'], ['api', 'postgres'], ['api', 'redis'], ['worker', 'redis'], ['worker', 'postgres']]
      .map(([a, b]) => [proj.services.find((s) => s.id === a), proj.services.find((s) => s.id === b)]).filter(([a, b]) => a && b);
    const ed = edges.map(([a, b]) => {
      const x1 = a.x + 220, y1 = a.y + 52, x2 = b.x, y2 = b.y + 52, mx = (x1 + x2) / 2;
      const seg = (l, t, w, h) => `<div class="edge topo-edge ${h > w ? 'v' : 'h'}" style="left:${l}px;top:${t}px;width:${Math.max(2, w)}px;height:${Math.max(2, h)}px;animation-delay:-${((l + t) % 9) / 10}s"></div>`;
      return seg(x1, y1, mx - x1, 1) + seg(mx, Math.min(y1, y2), 1, Math.abs(y2 - y1)) + seg(mx, y2, x2 - mx, 1);
    }).join('');
    const nodes = proj.services.map((s) => {
      const [k, t] = KIND[s.state] || KIND.idle;
      return html`<button type="button" class="node topo-node ${entered ? '' : 'enter'} ${s.id === sel ? 'on' : ''}" style="left:${s.x}px;top:${s.y}px;animation-delay:${((s.x + s.y) / 2000).toFixed(2)}s" data-act="select" data-id="${s.id}" aria-pressed="${s.id === sel}"><b>${s.id}</b><span class="sm muted">${s.repo}</span>${st(k, s.kind === 'db' ? 'running' : t)}</button>`;
    });
    return html`<div style="overflow-x:auto"><div class="graph">${raw(ed)}${nodes}</div></div>`;
  };

  const dnsSt = (d) => d.internal ? st('ok', 'built in') : d.dns === 'ok' ? st('ok', 'points here') : st('warn', 'waiting for DNS');
  const certSt = (d) => d.cert === 'valid' ? st('ok', `valid · renews in ${d.renews} d`) : d.cert === 'internal' ? st('ok', 'internal CA') : st('off', 'not issued yet');
  const domainsTab = () => {
    if (!doms) return html`<p class="muted">Loading…</p>`;
    const pend = doms.find((d) => d.dns === 'pending');
    const tag = (x) => html`<span class="tag-sm">${x}</span>`;
    return html`<div style="display:flex;flex-direction:column;gap:28px">
      ${api.MODE === 'http' ? html`<div class="note"><b>Sample preview:</b> Multi-domain DNS & Let's Encrypt TLS provisioning arrive in Prompt 10. Single-domain HTTP routing configured at service creation or Settings is active now.</div>` : ''}
      ${table('Domains', [['Domain'], ['DNS'], ['HTTPS'], ['', 'r']], doms.map((d) => [
        html`<span class="row" style="gap:8px"><b>${d.name}</b>${d.primary ? tag('primary') : ''}${d.redirect ? tag(`redirects → ${d.redirect}`) : ''}${d.custom && !d.primary ? tag('custom') : ''}${d.internal ? tag('private network') : ''}</span>`,
        dnsSt(d), certSt(d),
        html`<span class="row" style="justify-content:flex-end">${d.internal ? btn('Copy', { act: 'copydom', data: { n: d.name }, kind: 'link' }) : html`${d.dns === 'pending' ? btn('Check DNS', { act: 'dns', data: { n: d.name }, kind: 'link' }) : ''}${!d.primary && d.dns === 'ok' && !d.redirect ? btn('Make primary', { act: 'primary', data: { n: d.name }, kind: 'link' }) : ''}${btn('Remove', { act: 'rmdom', data: { n: d.name }, kind: 'link' })}`}</span>`]))}
      <div class="cols">
        <div class="w3">${pend ? html`<section class="sec"><div class="sec-h"><h2>Finish setting up ${pend.name}</h2></div>
          <p class="muted sm">Add this record where ${pend.name.split('.').slice(-2).join('.')} is managed (Cloudflare, Route 53, Namecheap…). Usually live within minutes.</p>
          ${table('DNS records', [['Type'], ['Name'], ['Value'], ['Status']], [['A', pend.name.split('.').slice(0, -2).join('.') || '@', html`<span class="row" style="gap:8px"><span class="mono">203.0.113.10</span>${btn('Copy', { act: 'copydom', data: { n: '203.0.113.10' }, kind: 'link' })}</span>`, st('warn', 'not found')], ['AAAA', pend.name.split('.').slice(0, -2).join('.') || '@', html`<span class="row" style="gap:8px"><span class="mono">2001:db8::10</span>${btn('Copy', { act: 'copydom', data: { n: '2001:db8::10' }, kind: 'link' })}</span>`, st('ok', 'found')]])}
          <div class="row">${btn('Check DNS now', { act: 'dns', data: { n: pend.name }, kind: 'primary' })}<span class="sm muted">The certificate is issued as soon as DNS points here.</span></div>
          <div class="note"><b>Using Cloudflare?</b> Set the record to DNS only (grey cloud) until the certificate is issued.</div></section>`
          : html`<section class="sec"><div class="sec-h"><h2>All domains are live</h2></div><p class="muted sm">Every domain points here and has a valid certificate.</p></section>`}</div>
        <div><section class="sec"><div class="sec-h"><h2>Add a domain</h2></div>
          <div class="field"><label for="nd">Domain</label><input class="input" id="nd" placeholder="app.acme.co"><span class="hint">A whole domain or a subdomain. No https://.</span></div>
          <div class="field"><label for="nw">www</label><select class="select" id="nw"><option>Redirect www to the bare domain</option><option>Redirect the bare domain to www</option><option>Serve both</option><option>No www</option></select></div>
          <div class="item-row"><label><input type="checkbox" id="nf" checked><span><b>Force HTTPS</b><span class="sm muted">Redirect http:// to https://</span></span></label></div>
          <p id="derr" role="alert" style="color:var(--danger);font-size:13px"></p>
          <div>${btn('Add domain', { act: 'adddom', kind: 'primary' })}</div></section>
          <section class="sec"><div class="sec-h"><h2>Certificates</h2></div><dl class="sm" style="display:grid;grid-template-columns:1fr auto;gap:10px;margin:0"><dt class="muted">Issuer</dt><dd style="margin:0">Let's Encrypt (automatic)</dd><dt class="muted">Renewal</dt><dd style="margin:0">30 days before expiry</dd><dt class="muted">If renewal fails</dt><dd style="margin:0">Alert via Slack + email</dd></dl></section></div>
      </div></div>`;
  };

  const detail = () => {
    const s = svc();
    if (!s) return '';
    const items = s.kind === 'db' ? ['Overview'] : ['Deploys', 'Variables', 'Domains & SSL', 'Settings'];
    const t = Math.min(tab, items.length - 1);
    let body;
    if (s.kind === 'db') body = html`<p class="muted">${s.repo}</p><p class="sm muted">${s.note}. Connection string is available to services in this project as an environment variable.</p>`;
    else if (t === 0) body = html`${s.deploys.length ? table('Deploys', [['Commit'], ['Message'], ['Status'], ['When'], ['', 'r']], s.deploys.map((d) => {
        const [k, l] = DST[d.status] || ['off', d.status];
        const shortCommit = d.commit ? d.commit.slice(0, 7) : d.id.slice(0, 7);
        const trig = d.trigger ? html` <span class="tag-sm">${d.trigger}</span>` : '';
        const errHint = d.status === 'failed' && d.error ? html`<div class="sm" style="color:var(--danger)">${d.error}</div>` : '';
        return [
          html`<span class="mono">${shortCommit}</span>`,
          html`${d.msg}${trig}${errHint}`,
          st(k, l),
          ago(d.at),
          html`<span class="row" style="justify-content:flex-end">${btn(logDeploy === d.id ? 'Hide log' : 'Log', { act: 'log', data: { id: d.id }, kind: 'link' })}${d.status === 'replaced' ? btn('Roll back', { act: 'rollback', data: { id: d.id }, kind: 'link' }) : ''}</span>`,
        ];
      })) : html`<p class="muted">No deploys yet.</p>`}
      ${logDeploy ? html`<div style="margin-top:14px"><pre class="code" id="build-log" aria-label="Build log">${logLoading ? 'Loading build log…' : (logLines.join('\n') || '(no log lines yet)')}</pre></div>` : ''}`;
    else if (t === 1) body = html`${s.vars.length ? table('Variables', [['Name'], ['Value'], ['', 'r']], s.vars.map(([k, v, isSec]) => [
        html`<span class="mono">${k}</span>${isSec ? html` <span class="tag-sm">secret</span>` : ''}`,
        html`<span class="mono muted">${isSec === true || (isSec === undefined && /URL|SECRET|KEY|PASS/.test(k)) ? '••••••••' : v}</span>`,
        btn('Remove', { act: 'rmvar', data: { k }, kind: 'link' }),
      ])) : html`<p class="muted">No variables yet.</p>`}<div style="margin-top:12px">${btn('Add variable', { act: 'addvar' })}</div>`;
    else if (t === 2) body = domainsTab(s);
    else body = html`<dl class="sm" style="display:grid;grid-template-columns:140px 1fr;gap:8px">
        <dt class="muted">Source</dt><dd class="mono">${s.sourceKind || 'github'} · ${s.repo}</dd>
        <dt class="muted">Branch</dt><dd class="mono">${s.branch || '—'}</dd>
        <dt class="muted">Build</dt><dd>${s.build}</dd>
        <dt class="muted">Health check</dt><dd class="mono">${s.healthPath || '/'}</dd>
        <dt class="muted">Auto-deploy</dt><dd>${s.autoDeploy === false ? 'Off' : 'On'}</dd>
        <dt class="muted">Domain</dt><dd class="mono">${s.domain || '—'}</dd>
        <dt class="muted">Port</dt><dd class="mono">${s.port || '—'}${s.hostPort ? ` (127.0.0.1:${s.hostPort})` : ''}</dd>
      </dl>
      <div class="row" style="margin-top:16px">
        ${s.sourceKind !== 'docker_image' ? btn(s.autoDeploy === false ? 'Enable auto-deploy' : 'Disable auto-deploy', { act: 'toggleAuto' }) : ''}
        ${btn('Delete service', { act: 'delsvc', kind: 'danger' })}
      </div>`;
    return section(s.id, html`${tabs(items, t, 'tab')}<div style="padding-top:16px">${body}</div>`, s.kind === 'app' ? html`<span class="row">${btn('Domains', { act: 'gotodomains', kind: 'link' })}${s.domain ? html`<a class="btn link mono" href="http://${s.domain}" target="_blank" rel="noopener">${s.domain} ↗</a>` : ''}${btn('Redeploy', { act: 'redeploy', kind: 'primary' })}</span>` : '');
  };

  const paint = () => {
    if (!root) return;
    root.querySelector('#g').innerHTML = graph().toString();
    entered = true;
    root.querySelector('#d').innerHTML = detail().toString();
    if (tab === 2 && !doms) api.getDomains(sel).then((d) => { doms = d; paint(); });
  };
  const reload = async () => { proj = await api.getProject(); sel = svc()?.id || ''; paint(); };

  return {
    nav: 'apps', title: 'Apps',
    html: html`<main>${header('Apps', `${proj.name} · ${proj.env} · ${proj.services.length} services`, html`<span class="row"><a class="btn" href="#/apps/new/templates">+ Database</a><a class="btn primary" href="#/apps/new">New service</a></span>`)}<div id="g"></div><div id="d"></div></main>`,
    mount(r) {
      root = r;
      paint();
      if (logDeploy) openLogFor(logDeploy);
      bindActions(root, {
        select: (el) => { sel = el.dataset.id; tab = 0; stopWs(); logDeploy = null; logLines = []; doms = null; paint(); },
        tab: (el) => { tab = +el.dataset.i; paint(); },
        gotodomains: () => { tab = 2; paint(); root.querySelector('#d').scrollIntoView({ behavior: 'smooth', block: 'start' }); },
        adddom: async () => { const e = root.querySelector('#derr'); try { const r2 = await api.addDomain(sel, { name: root.querySelector('#nd').value, www: root.querySelector('#nw').value, force: root.querySelector('#nf').checked }); toast(`${r2.name} added. Point its DNS here.`); doms = await api.getDomains(sel); paint(); } catch (err) { e.textContent = err.message; } },
        dns: async (el) => { const d = await api.checkDns(sel, el.dataset.n); toast(d.dns === 'ok' ? `DNS found. Certificate issued for ${d.name}.` : 'Not pointing here yet. Checked again in a minute.', d.dns === 'ok' ? 'ok' : 'err'); doms = await api.getDomains(sel); paint(); },
        rmdom: async (el) => { if (await confirmDialog({ title: `Remove ${el.dataset.n}?`, body: 'Visitors to this domain stop reaching the app.', confirm: 'Remove', danger: true })) { await api.removeDomain(sel, el.dataset.n); doms = await api.getDomains(sel); paint(); } },
        primary: async (el) => { await api.makePrimary(sel, el.dataset.n); doms = await api.getDomains(sel); paint(); },
        copydom: async (el) => { try { await navigator.clipboard.writeText(el.dataset.n); toast('Copied.'); } catch { toast(el.dataset.n); } },
        log: (el) => { const id = el.dataset.id; openLogFor(logDeploy === id ? null : id); },
        redeploy: async () => {
          const r2 = await api.redeploy(sel);
          toast('Deploy started.');
          tab = 0;
          await reload();
          if (r2?.id) await openLogFor(r2.id);
        },
        rollback: async (el) => {
          if (await confirmDialog({ title: `Roll back ${sel}?`, body: `Traffic moves to deploy ${el.dataset.id.slice(0, 7)}. The current image stays available.`, confirm: 'Roll back' })) {
            const r2 = await api.rollback(sel, el.dataset.id);
            toast('Rollback started.');
            await reload();
            if (r2?.id) await openLogFor(r2.id);
          }
        },
        addvar: async () => {
          const out = await askForm({
            title: 'Add variable',
            fields: [
              { name: 'k', label: 'Name', placeholder: 'API_KEY' },
              { name: 'v', label: 'Value' },
              { name: 'sec', label: 'Visibility', options: ['Auto (mask secret-looking names)', 'Secret (masked & --secret mount)', 'Plaintext (visible & build arg)'], value: 'Auto (mask secret-looking names)' },
            ],
            submit: 'Add',
          }, (o) => {
            const isSec = o.sec.startsWith('Secret') ? true : o.sec.startsWith('Plaintext') ? false : undefined;
            return api.addVar(sel, o.k.trim(), o.v, isSec);
          });
          if (out !== null) await reload();
        },
        rmvar: async (el) => { await api.removeVar(sel, el.dataset.k); await reload(); },
        toggleAuto: async () => {
          const s = svc();
          if (!s) return;
          await api.updateService(sel, { autoDeploy: !(s.autoDeploy !== false) });
          toast('Auto-deploy updated.');
          await reload();
        },
        delsvc: async () => {
          if (await confirmDialog({ title: `Delete ${sel}?`, body: 'The service stops and its container is removed. This cannot be undone.', confirm: 'Delete', danger: true, typed: sel })) {
            stopWs();
            logDeploy = null;
            await api.removeService(sel);
            proj = await api.getProject();
            sel = proj.services[0]?.id || '';
            paint();
            toast('Service deleted.');
          }
        },
      });
      this._t = () => {
        api.getProject().then((p) => {
          const changed = p.services.length !== proj.services.length
            || p.services.some((s, i) => s.state !== proj.services[i]?.state || s.deploys?.[0]?.status !== proj.services[i]?.deploys?.[0]?.status);
          if (changed) { proj = p; paint(); }
        }).catch(() => {});
      };
      listeners.add(this._t);
    },
    unmount() { stopWs(); listeners.delete(this._t); },
  };
}
