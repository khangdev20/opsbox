import { html, raw, ago, st, btn, tabs, table, section, header, sw, bindActions, confirmDialog, askForm, toast } from '../ui.js';
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
  let domLoadingFor = null;
  let domBusy = '';
  let domErr = '';
  let lastDns = null;
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
    const rawEdges = Array.isArray(proj.edges)
      ? proj.edges
      : [['web', 'api'], ['web', 'postgres'], ['api', 'postgres'], ['api', 'redis'], ['worker', 'redis'], ['worker', 'postgres']];
    const edges = rawEdges
      .map(([a, b]) => [proj.services.find((s) => s.id === a), proj.services.find((s) => s.id === b)]).filter(([a, b]) => a && b);
    const ed = edges.map(([a, b]) => {
      const x1 = a.x + 220, y1 = a.y + 52, x2 = b.x, y2 = b.y + 52, mx = (x1 + x2) / 2;
      const seg = (l, t, w, h) => `<div class="edge topo-edge ${h > w ? 'v' : 'h'}" style="left:${l}px;top:${t}px;width:${Math.max(2, w)}px;height:${Math.max(2, h)}px;animation-delay:-${((l + t) % 9) / 10}s"></div>`;
      return seg(x1, y1, mx - x1, 1) + seg(mx, Math.min(y1, y2), 1, Math.abs(y2 - y1)) + seg(mx, y2, x2 - mx, 1);
    }).join('');
    const maxY = Math.max(300, ...proj.services.map((s) => (s.y || 0) + 140));
    const nodes = proj.services.map((s) => {
      const [k, t] = KIND[s.state] || KIND.idle;
      return html`<button type="button" class="node topo-node ${entered ? '' : 'enter'} ${s.id === sel ? 'on' : ''}" style="left:${s.x}px;top:${s.y}px;animation-delay:${((s.x + s.y) / 2000).toFixed(2)}s" data-act="select" data-id="${s.id}" aria-pressed="${s.id === sel}"><b>${s.id}</b><span class="sm muted">${s.repo}</span>${st(k, t)}</button>`;
    });
    return html`<div style="overflow-x:auto"><div class="graph" style="height:${maxY}px">${raw(ed)}${nodes}</div></div>`;
  };

  const dnsSt = (d) => {
    if (domBusy === `dns:${d.name}`) return st('build', 'checking DNS…');
    return d.dns === 'ok' ? st('ok', 'points here') : st('warn', 'waiting for DNS');
  };
  const certSt = (d) => {
    if (domBusy === `cert:${d.name}`) return st('build', 'issuing certificate…');
    if (d.cert === 'valid') {
      const stage = d.certStaging ? " (staging)" : '';
      return st('ok', `valid · renews in ${d.renews ?? '?'} d${stage}`);
    }
    if (d.cert === 'expiring') {
      return html`<div>${st('warn', `expiring · ${d.renews ?? '?'} d`)}${d.certError ? html`<div class="sm" style="color:var(--danger)">${d.certError}</div>` : ''}</div>`;
    }
    if (d.cert === 'error') {
      return html`<div>${st('fail', 'certificate error')}${d.certError ? html`<div class="sm" style="color:var(--danger)">${d.certError}</div>` : ''}</div>`;
    }
    if (d.cert === 'pending') return st('build', 'issuing…');
    return st('off', 'not issued yet');
  };
  const domainsTab = (s) => {
    if (!doms || doms.service !== s.id) return html`<p class="muted">Loading…</p>`;
    const publicDoms = doms.domains || [];
    const servingDoms = publicDoms.filter((d) => !d.redirect);
    const pend = publicDoms.find((d) => d.dns === 'pending');
    const needCert = publicDoms.find((d) => d.dns === 'ok' && d.cert !== 'valid' && d.cert !== 'expiring');
    const tag = (x) => html`<span class="tag-sm">${x}</span>`;
    const proxyDown = api.MODE === 'http' && doms.proxyReachable === false;
    const isWorker = s.port === 0;
    const internalTarget = doms.internalUrl || doms.internalHost || s.id;
    const internalRow = [
      html`<span class="row" style="gap:8px;flex-wrap:wrap"><b>${doms.internalHost || s.id}</b>${tag('project network')}${doms.port > 0 ? html`<span class="mono sm muted">${internalTarget}</span>` : html`<span class="sm muted">worker (port 0)</span>`}</span>`,
      st('ok', doms.network || 'opsbox-default'),
      st('off', 'internal HTTP'),
      html`<span class="sm muted">—</span>`,
      html`<span class="row" style="justify-content:flex-end">${btn('Copy', { act: 'copydom', data: { n: internalTarget }, kind: 'link' })}</span>`,
    ];
    const domainRows = publicDoms.map((d) => [
      html`<span class="row" style="gap:8px;flex-wrap:wrap"><b>${d.name}</b>${d.primary ? tag('primary') : ''}${d.redirect ? tag(`redirects → ${d.redirect}`) : ''}${d.custom && !d.primary && !d.redirect ? tag('custom') : ''}</span>`,
      dnsSt(d),
      certSt(d),
      html`<span class="row" style="gap:10px;flex-wrap:wrap"><span class="row sm" style="gap:6px">${sw(`Force HTTPS for ${d.name}`, !!d.forceHttps, 'toggleForce', { n: d.name })}<span>Force HTTPS</span></span><span class="row sm" style="gap:6px">${sw(`HSTS for ${d.name}`, !!d.hsts, 'toggleHsts', { n: d.name })}<span>HSTS</span></span></span>`,
      html`<span class="row" style="justify-content:flex-end">${d.dns === 'pending' ? btn('Check DNS', { act: 'dns', data: { n: d.name }, kind: 'link', disabled: !!domBusy }) : (d.cert !== 'valid' && d.cert !== 'expiring' ? btn('Issue cert', { act: 'issuecert', data: { n: d.name }, kind: 'link', disabled: !!domBusy || proxyDown }) : btn('Renew', { act: 'issuecert', data: { n: d.name }, kind: 'link', disabled: !!domBusy || proxyDown }))}${!d.primary && !d.redirect ? btn('Make primary', { act: 'primary', data: { n: d.name }, kind: 'link', disabled: !!domBusy }) : ''}${btn('Remove', { act: 'rmdom', data: { n: d.name }, kind: 'link', disabled: !!domBusy })}</span>`,
    ]);
    const recRows = (doms.serverIps || []).map((ip) => {
      const typ = ip.includes(':') ? 'AAAA' : 'A';
      const found = lastDns && lastDns.domain === pend?.name
        ? ((lastDns.resolvedIps || []).includes(ip) ? st('ok', 'found') : st('warn', 'not found'))
        : st('warn', 'waiting');
      return [typ, pend?.name || '', html`<span class="row" style="gap:8px"><span class="mono">${ip}</span>${btn('Copy', { act: 'copydom', data: { n: ip }, kind: 'link' })}</span>`, found];
    });
    const statusCard = pend
      ? html`<section class="sec"><div class="sec-h"><h2>Finish setting up ${pend.name}</h2></div>
          <p class="muted sm">Point an A or AAAA record for <span class="mono">${pend.name}</span> to this server where your DNS is managed (Cloudflare, Route 53, Namecheap…). Usually live within minutes.</p>
          ${recRows.length ? table('DNS records', [['Type'], ['Name'], ['Value'], ['Status']], recRows) : html`<div class="note">Could not detect this server's public IP automatically. Set <span class="mono">[proxy] public_ips</span> in <span class="mono">/etc/opsbox/config.toml</span> or point your DNS record to this server's IP.</div>`}
          ${lastDns && lastDns.domain === pend.name && !lastDns.pointsHere ? html`<p class="sm muted">${lastDns.message}</p>` : ''}
          <div class="row">${btn('Check DNS now', { act: 'dns', data: { n: pend.name }, kind: 'primary', disabled: !!domBusy })}<span class="sm muted">The certificate is issued as soon as DNS points here.</span></div>
          <div class="note"><b>Using Cloudflare?</b> Set the record to DNS only (grey cloud) until the certificate is issued.</div></section>`
      : !publicDoms.length
        ? html`<section class="sec"><div class="sec-h"><h2>No public domains yet</h2></div><p class="muted sm">Add a domain on the right to route HTTP and HTTPS traffic to <span class="mono">${s.id}</span>. Other containers on <span class="mono">${doms.network || 'opsbox-default'}</span> can already reach it at <span class="mono">${internalTarget}</span>.</p></section>`
        : needCert
          ? html`<section class="sec"><div class="sec-h"><h2>Issue TLS certificate</h2></div><p class="muted sm">DNS for <span class="mono">${needCert.name}</span> points here. Issue its Let's Encrypt certificate to enable HTTPS.</p><div class="row">${btn(`Issue certificate for ${needCert.name}`, { act: 'issuecert', data: { n: needCert.name }, kind: 'primary', disabled: !!domBusy || proxyDown })}</div></section>`
          : html`<section class="sec"><div class="sec-h"><h2>All domains are live</h2></div><p class="muted sm">Every domain points here and has a valid certificate.</p></section>`;
    return html`<div style="display:flex;flex-direction:column;gap:28px">
      ${table('Domains', [['Domain'], ['DNS'], ['HTTPS'], ['Policy'], ['', 'r']], [...domainRows, internalRow])}
      <div class="cols">
        <div class="w3">${statusCard}</div>
        <div><section class="sec"><div class="sec-h"><h2>Add a domain</h2></div>
          ${isWorker ? html`<div class="note">This service runs on port 0 (background worker) and has no HTTP port for public domains.</div>` : ''}
          <div class="field"><label for="nd">Domain</label><input class="input" id="nd" placeholder="app.acme.co"${isWorker || !!domBusy ? ' disabled' : ''}><span class="hint">A whole domain or a subdomain. No https://.</span></div>
          <div class="field"><label for="nw">www</label><select class="select" id="nw"${isWorker || !!domBusy ? ' disabled' : ''}><option value="none">No www (this domain only)</option><option value="www_to_bare">Redirect www to the bare domain</option><option value="bare_to_www">Redirect the bare domain to www</option><option value="serve_both">Serve both bare and www</option></select></div>
          ${servingDoms.length ? html`<div class="field"><label for="nr">Redirect to</label><select class="select" id="nr"${isWorker || !!domBusy ? ' disabled' : ''}><option value="">Serve the app directly</option>${servingDoms.map((d) => html`<option value="${d.name}">Redirect to ${d.name}</option>`)}</select><span class="hint">Optional: forward all requests from this domain to an existing domain.</span></div>` : ''}
          <div class="item-row"><label><input type="checkbox" id="nf" checked${isWorker || !!domBusy ? ' disabled' : ''}><span><b>Force HTTPS</b><span class="sm muted">Redirect http:// to https:// once the certificate is issued</span></span></label></div>
          <div class="item-row"><label><input type="checkbox" id="nh" checked${isWorker || !!domBusy ? ' disabled' : ''}><span><b>Enable HSTS</b><span class="sm muted">Send Strict-Transport-Security over HTTPS</span></span></label></div>
          <p id="derr" role="alert" style="color:var(--danger);font-size:13px">${domErr}</p>
          <div>${btn('Add domain', { act: 'adddom', kind: 'primary', disabled: isWorker || !!domBusy })}</div></section>
          <section class="sec"><div class="sec-h"><h2>Certificates</h2></div>
          ${proxyDown ? html`<div class="row" style="margin-bottom:10px">${st('fail', 'Reverse proxy unavailable')}<span class="sm muted">Start opsbox-proxy to issue or renew TLS certificates.</span></div>` : ''}
          <dl class="sm" style="display:grid;grid-template-columns:1fr auto;gap:10px;margin:0"><dt class="muted">Issuer</dt><dd style="margin:0">Let's Encrypt (HTTP-01)</dd><dt class="muted">Renewal</dt><dd style="margin:0">30 days before expiry</dd><dt class="muted">If renewal fails</dt><dd style="margin:0">Shown on Overview</dd></dl></section></div>
      </div></div>`;
  };

  const detail = () => {
    const s = svc();
    if (!s) return '';
    const isDb = s.kind === 'db';
    const items = isDb ? ['Overview', 'Variables', 'Settings'] : ['Deploys', 'Variables', 'Domains & SSL', 'Settings'];
    const t = Math.min(tab, items.length - 1);
    const varsBody = () => html`${s.vars.length ? table('Variables', [['Name'], ['Value'], ['', 'r']], s.vars.map(([k, v, isSec]) => [
      html`<span class="mono">${k}</span>${isSec ? html` <span class="tag-sm">secret</span>` : ''}`,
      html`<span class="mono muted">${isSec === true || (isSec === undefined && /URL|SECRET|KEY|PASS/.test(k)) ? '••••••••' : v}</span>`,
      btn('Remove', { act: 'rmvar', data: { k }, kind: 'link' }),
    ])) : html`<p class="muted">No variables yet.</p>`}<div style="margin-top:12px">${btn('Add variable', { act: 'addvar' })}</div>`;
    const settingsBody = () => html`<dl class="sm" style="display:grid;grid-template-columns:140px 1fr;gap:8px">
        <dt class="muted">Source</dt><dd class="mono">${s.templateId ? `template:${s.templateId}` : s.composeStack ? `compose:${s.composeStack}` : (s.sourceKind || 'github')} · ${s.repo}</dd>
        <dt class="muted">Branch / Version</dt><dd class="mono">${s.branch || '—'}</dd>
        <dt class="muted">Build</dt><dd>${s.build || '—'}</dd>
        <dt class="muted">Health check</dt><dd class="mono">${s.healthPath || '/'}</dd>
        <dt class="muted">Network</dt><dd class="mono">${s.internalOnly ? 'Project network only (internal)' : 'Loopback proxy enabled'}</dd>
        <dt class="muted">Auto-deploy</dt><dd>${s.autoDeploy === false ? 'Off' : 'On'}</dd>
        <dt class="muted">Domain</dt><dd class="mono">${s.domain || '—'}</dd>
        <dt class="muted">Port</dt><dd class="mono">${s.port || '—'}${s.hostPort ? ` (127.0.0.1:${s.hostPort})` : ''}</dd>
      </dl>
      <div class="row" style="margin-top:16px">
        ${s.sourceKind !== 'docker_image' && !s.templateId && !s.composeStack ? btn(s.autoDeploy === false ? 'Enable auto-deploy' : 'Disable auto-deploy', { act: 'toggleAuto' }) : ''}
        ${btn('Delete service', { act: 'delsvc', kind: 'danger' })}
      </div>`;
    let body;
    if (isDb && t === 0) body = html`<dl class="sm" style="display:grid;grid-template-columns:140px 1fr;gap:8px">
      <dt class="muted">Engine</dt><dd class="mono">${s.repo}</dd>
      <dt class="muted">Host on network</dt><dd class="mono">${s.id}:${s.port || '—'}</dd>
      <dt class="muted">Volume / Backup</dt><dd>${s.note || 'Persistent named volume'}${s.backup ? ` · ${s.backup.enabled ? `auto ${s.backup.method} (${s.backup.cron})` : 'auto backup off'}` : ''}</dd>
    </dl><p class="sm muted" style="margin-top:12px">Credentials are encrypted at rest. Reach this database from services in the same project via <span class="mono">${s.id}:${s.port || ''}</span>.</p>`;
    else if (isDb && t === 1) body = varsBody();
    else if (isDb) body = settingsBody();
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
    else if (t === 1) body = varsBody();
    else if (t === 2) body = s.internalOnly
      ? html`<p class="muted">This service runs on the internal project network only (<span class="mono">${s.id}:${s.port || '—'}</span>) without an internet-facing host port.</p>`
      : domainsTab(s);
    else body = settingsBody();
    const canDomain = !isDb && !s.internalOnly && s.port > 0;
    return section(s.id, html`${tabs(items, t, 'tab')}<div style="padding-top:16px">${body}</div>`, html`<span class="row">${canDomain ? btn('Domains', { act: 'gotodomains', kind: 'link' }) : ''}${s.domain ? html`<a class="btn link mono" href="${s.domainScheme || 'http'}://${s.domain}" target="_blank" rel="noopener">${s.domain} ↗</a>` : ''}${btn('Redeploy', { act: 'redeploy', kind: 'primary' })}</span>`);
  };

  const loadDomains = async (targetSvc) => {
    if (!targetSvc || domLoadingFor === targetSvc) return;
    domLoadingFor = targetSvc;
    try {
      const d = await api.getDomains(targetSvc);
      if (sel === targetSvc) doms = d;
    } finally {
      if (domLoadingFor === targetSvc) domLoadingFor = null;
      paint();
    }
  };
  const refreshDomainsAndProj = async () => {
    const [p, d] = await Promise.all([api.getProject(), api.getDomains(sel)]);
    proj = p;
    sel = svc()?.id || '';
    if (d.service === sel) doms = d;
    paint();
  };
  const paint = () => {
    if (!root) return;
    root.querySelector('#g').innerHTML = graph().toString();
    entered = true;
    root.querySelector('#d').innerHTML = detail().toString();
    if (tab === 2 && sel && (!doms || doms.service !== sel) && domLoadingFor !== sel) loadDomains(sel);
  };
  const reload = async () => { proj = await api.getProject(); sel = svc()?.id || ''; if (tab === 2 && sel) doms = await api.getDomains(sel); paint(); };

  return {
    nav: 'apps', title: 'Apps',
    html: html`<main>${header('Apps', `${proj.name} · ${proj.env} · ${proj.services.length} services`, html`<span class="row"><a class="btn" href="#/apps/new/templates">+ Database</a><a class="btn primary" href="#/apps/new">New service</a></span>`)}<div id="g"></div><div id="d"></div></main>`,
    mount(r) {
      root = r;
      paint();
      if (logDeploy) openLogFor(logDeploy);
      root.addEventListener('change', (e) => {
        if (e.target.id === 'nr') {
          const nw = root.querySelector('#nw');
          if (nw) nw.disabled = !!e.target.value;
        }
      });
      bindActions(root, {
        select: (el) => { sel = el.dataset.id; tab = 0; stopWs(); logDeploy = null; logLines = []; doms = null; domBusy = ''; domErr = ''; lastDns = null; paint(); },
        tab: (el) => { tab = +el.dataset.i; paint(); },
        gotodomains: () => { tab = 2; paint(); root.querySelector('#d').scrollIntoView({ behavior: 'smooth', block: 'start' }); },
        adddom: async () => {
          domErr = '';
          const e = root.querySelector('#derr');
          if (e) e.textContent = '';
          try {
            const redirectTo = root.querySelector('#nr')?.value || '';
            const r2 = await api.addDomain(sel, {
              name: root.querySelector('#nd').value,
              www: redirectTo ? 'none' : root.querySelector('#nw').value,
              redirectTo,
              force: root.querySelector('#nf').checked,
              hsts: root.querySelector('#nh').checked,
            });
            toast(`${r2.name} added. Point its DNS here.`);
            await refreshDomainsAndProj();
          } catch (err) {
            domErr = err.message;
            if (e) e.textContent = err.message;
          }
        },
        dns: async (el) => {
          const name = el.dataset.n;
          domBusy = `dns:${name}`; domErr = ''; paint();
          try {
            const d = await api.checkDns(sel, name);
            lastDns = d;
            if (d.dns === 'ok') {
              const cur = (doms?.domains || []).find((x) => x.name === name);
              if (cur && (cur.cert === 'valid' || cur.cert === 'expiring')) {
                toast(`DNS points here for ${name}.`, 'ok');
              } else {
                domBusy = `cert:${name}`; paint();
                await api.issueDomainCert(sel, name);
                toast(`DNS found. Certificate issued for ${name}.`, 'ok');
              }
            } else {
              toast('Not pointing here yet. Check your DNS records and try again.', 'err');
            }
          } catch (err) {
            domErr = err.message;
            toast(err.message, 'err');
          } finally {
            domBusy = '';
            await refreshDomainsAndProj().catch(() => paint());
          }
        },
        issuecert: async (el) => {
          const name = el.dataset.n;
          domBusy = `cert:${name}`; domErr = ''; paint();
          try {
            await api.issueDomainCert(sel, name);
            toast(`Certificate issued for ${name}.`, 'ok');
          } catch (err) {
            domErr = err.message;
            toast(err.message, 'err');
          } finally {
            domBusy = '';
            await refreshDomainsAndProj().catch(() => paint());
          }
        },
        toggleForce: async (el) => {
          const next = el.getAttribute('aria-checked') !== 'true';
          try {
            await api.updateDomain(sel, el.dataset.n, { forceHttps: next });
            toast(next ? `Force HTTPS enabled for ${el.dataset.n}.` : `Force HTTPS disabled for ${el.dataset.n}.`);
          } finally {
            await refreshDomainsAndProj().catch(() => paint());
          }
        },
        toggleHsts: async (el) => {
          const next = el.getAttribute('aria-checked') !== 'true';
          try {
            await api.updateDomain(sel, el.dataset.n, { hsts: next });
            toast(next ? `HSTS enabled for ${el.dataset.n}.` : `HSTS disabled for ${el.dataset.n}.`);
          } finally {
            await refreshDomainsAndProj().catch(() => paint());
          }
        },
        rmdom: async (el) => {
          if (await confirmDialog({ title: `Remove ${el.dataset.n}?`, body: 'Visitors to this domain stop reaching the app.', confirm: 'Remove', danger: true })) {
            await api.removeDomain(sel, el.dataset.n);
            toast(`${el.dataset.n} removed.`);
            await refreshDomainsAndProj();
          }
        },
        primary: async (el) => {
          await api.makePrimary(sel, el.dataset.n);
          toast(`${el.dataset.n} is now the primary domain.`);
          await refreshDomainsAndProj();
        },
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
