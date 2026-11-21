import { adoptedDomains, proxyModeNotice, networkActions } from '../network.js';
import { html, raw, ago, st, btn, tabs, table, section, header, sw, bindActions, confirmDialog, askForm, toast, patch, empty, announce, iconAct } from '../ui.js';
import { editMemory, withCapacityOverride } from '../capacity.js';
import * as api from '../api.js';
import { listeners } from '../store.js';
import { withReveal, copyPlain, hideRevealed } from '../secrets.js';

const KIND = { active: ['ok', 'running'], building: ['build', 'building'], failed: ['fail', 'failed'], idle: ['off', 'idle'] };
const DST = { live: ['ok', 'live'], replaced: ['off', 'replaced'], failed: ['fail', 'failed'], building: ['build', 'building'] };
const LINK_ICON = '<svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" style="vertical-align:-2px;margin-right:4px" aria-hidden="true"><path d="M6.5 9.5a3.5 3.5 0 0 0 5 0l2-2a3.5 3.5 0 0 0-5-5l-1 1"/><path d="M9.5 6.5a3.5 3.5 0 0 0-5 0l-2 2a3.5 3.5 0 0 0 5 5l1-1"/></svg>';
const OWN_LOGIN = 'Its own database and login (recommended)';
const SHARED_LOGIN = 'The shared main login';
const varRefTarget = (val) => {
  const m = typeof val === 'string' && val.match(/^\$\{\{\s*([a-z0-9-]+)\.([A-Z_][A-Z0-9_]*)\s*\}\}$/);
  return m ? { svc: m[1], key: m[2] } : null;
};
const networkModeOf = (s) => s.networkMode || (s.internalOnly ? 'internal' : s.publicPort ? 'public' : 'loopback');
const exposureLabel = (s) => {
  const m = networkModeOf(s);
  if (m === 'internal') return 'Private project network only (no host port)';
  if (m === 'public') return `Public internet port (0.0.0.0:${s.hostPort || s.port || '—'})`;
  return `Loopback proxy only (127.0.0.1:${s.hostPort || s.port || '—'})`;
};

export async function view({ query }) {
  let proj = await api.getProject();
  let sel = query.s || proj.services[0]?.id || 'web';
  let tab = query.tab === 'domains' ? 2 : 0;
  let logDeploy = query.log || null;
  let logLines = [];
  let logLoading = false;
  /** @type {Record<string, string>} plaintext secrets shown until hide / timeout / nav */
  let revealedVars = {};
  let closeLogWs = null;
  let root;
  let doms = null;
  const domLoading = new Set();
  const domLoadErrors = new Map();
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
      return section('Services', html`${empty('No services deployed yet.', html`<a class="btn primary" href="#/apps/new">Deploy your first service</a>`)}`);
    }
    const rawEdges = Array.isArray(proj.edges)
      ? proj.edges
      : [['web', 'api'], ['web', 'postgres'], ['api', 'postgres'], ['api', 'redis'], ['worker', 'redis'], ['worker', 'postgres']];
    const edges = rawEdges
      .map(([a, b]) => [proj.services.find((s) => s.id === a), proj.services.find((s) => s.id === b)]).filter(([a, b]) => a && b);
    const seg = (l, t, w, h) => `<div class="edge topo-edge ${h > w ? 'v' : 'h'}" style="left:${l}px;top:${t}px;width:${Math.max(2, w)}px;height:${Math.max(2, h)}px;animation-delay:-${((l + t) % 9) / 10}s"></div>`;
    const ed = edges.map(([a, b]) => {
      const ax = a.x || 0, ay = a.y || 0, bx = b.x || 0, by = b.y || 0;
      if (ax + 220 <= bx) {
        const x1 = ax + 220, y1 = ay + 52, x2 = bx, y2 = by + 52, mx = (x1 + x2) / 2;
        return seg(x1, y1, mx - x1, 1) + seg(mx, Math.min(y1, y2), 1, Math.abs(y2 - y1)) + seg(mx, y2, x2 - mx, 1);
      }
      if (bx + 220 <= ax) {
        const x1 = ax, y1 = ay + 52, x2 = bx + 220, y2 = by + 52, mx = (x1 + x2) / 2;
        return seg(x2, y2, mx - x2, 1) + seg(mx, Math.min(y1, y2), 1, Math.abs(y2 - y1)) + seg(mx, y1, x1 - mx, 1);
      }
      const x1 = ax + 110, x2 = bx + 110;
      const [y1, y2] = ay <= by ? [ay + 104, by] : [ay, by + 104];
      const my = (y1 + y2) / 2;
      const topA = Math.min(y1, my), topB = Math.min(my, y2);
      return seg(x1, topA, 1, Math.abs(my - y1)) + seg(Math.min(x1, x2), my, Math.abs(x2 - x1), 1) + seg(x2, topB, 1, Math.abs(y2 - my));
    }).join('');
    const maxX = Math.max(960, ...proj.services.map((s) => (s.x || 0) + 240));
    const maxY = Math.max(300, ...proj.services.map((s) => (s.y || 0) + 140));
    const nodes = proj.services.map((s) => {
      const [k, t] = KIND[s.state] || KIND.idle;
      return html`<button type="button" class="node topo-node ${entered ? '' : 'enter'} ${s.id === sel ? 'on' : ''}" style="left:${s.x}px;top:${s.y}px;animation-delay:${((s.x + s.y) / 2000).toFixed(2)}s" data-act="select" data-id="${s.id}" aria-pressed="${s.id === sel}"><b>${s.id}</b><span class="sm muted">${s.repo}</span>${st(k, t)}${s.overCapacity ? st('warn', 'over capacity') : ''}</button>`;
    });
    return html`<div style="overflow-x:auto"><div class="graph" style="width:${maxX}px;height:${maxY}px">${raw(ed)}${nodes}</div></div>`;
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
      return html`<div>${st('warn', `expiring · ${d.renews ?? '?'} d`)}${d.certError ? html`<div class="sm err-t">${d.certError}</div>` : ''}</div>`;
    }
    if (d.cert === 'error') {
      return html`<div>${st('fail', 'certificate error')}${d.certError ? html`<div class="sm err-t">${d.certError}</div>` : ''}</div>`;
    }
    if (d.cert === 'pending') return st('build', 'issuing…');
    return st('off', 'not issued yet');
  };
  const domainsTab = (s) => {
    if (domLoadErrors.has(s.id)) return html`<div role="alert"><p class="form-err">Could not load domains: ${domLoadErrors.get(s.id)}</p>${btn('Retry', { act: 'retrydomains' })}</div>`;
    if (!doms || doms.service !== s.id) return html`<p class="muted">Loading…</p>`;
    const publicDoms = doms.domains || [];
    const servingDoms = publicDoms.filter((d) => !d.redirect);
    const pend = publicDoms.find((d) => d.dns === 'pending');
    const needCert = publicDoms.find((d) => d.dns === 'ok' && d.cert !== 'valid' && d.cert !== 'expiring');
    const tag = (x) => html`<span class="tag-sm">${x}</span>`;
    const proxyDisabled = doms.proxyMode && doms.proxyMode !== 'builtin';
    const proxyDown = proxyDisabled || (api.MODE === 'http' && doms.proxyReachable === false);
    const isWorker = s.port === 0;
    const internalTarget = doms.internalUrl || doms.internalHost || s.id;
    const internalRow = [
      html`<span class="row" style="gap:8px;flex-wrap:wrap"><b>${doms.internalHost || s.id}</b>${tag('project network')}${doms.port > 0 ? html`<span class="mono sm muted">${internalTarget}</span>` : html`<span class="sm muted">worker (port 0)</span>`}</span>`,
      st('ok', doms.network || 'opsbox-default'),
      st('off', 'internal HTTP'),
      html`<span class="sm muted">—</span>`,
      html`<span class="acts">${iconAct('copydom', 'Copy internal address', 'copy', { n: internalTarget })}</span>`,
    ];
    const domainRows = publicDoms.map((d) => [
      html`<span class="row" style="gap:8px;flex-wrap:wrap"><b>${d.name}</b>${d.primary ? tag('primary') : ''}${d.redirect ? tag(`redirects → ${d.redirect}`) : ''}${d.custom && !d.primary && !d.redirect ? tag('custom') : ''}</span>`,
      dnsSt(d),
      certSt(d),
      html`<span class="row" style="gap:10px;flex-wrap:wrap"><span class="row sm" style="gap:6px">${sw(`Force HTTPS for ${d.name}`, !!d.forceHttps, 'toggleForce', { n: d.name })}<span>Force HTTPS</span></span><span class="row sm" style="gap:6px">${sw(`HSTS for ${d.name}`, !!d.hsts, 'toggleHsts', { n: d.name })}<span>HSTS</span></span></span>`,
      html`<span class="acts">${d.dns === 'pending' ? iconAct('dns', `Check DNS for ${d.name}`, 'globe', { n: d.name }, { disabled: !!domBusy }) : iconAct('issuecert', `${d.cert !== 'valid' && d.cert !== 'expiring' ? 'Issue certificate' : 'Renew certificate'} for ${d.name}`, 'cert', { n: d.name }, { disabled: !!domBusy || proxyDown })}${!d.primary && !d.redirect ? iconAct('primary', `Make ${d.name} the primary domain`, 'star', { n: d.name }, { disabled: !!domBusy }) : ''}${iconAct('rmdom', `Remove ${d.name}`, 'del', { n: d.name }, { danger: true, disabled: !!domBusy })}</span>`,
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
      ${api.MODE === 'http' ? proxyModeNotice(doms, !!domBusy) : ''}
      ${adoptedDomains(doms.adopted || [])}
      ${table('Domains', [['Domain'], ['DNS'], ['HTTPS'], ['Policy'], ['', 'r']], [...domainRows, internalRow])}
      <div class="cols">
        <div class="w3">${proxyDisabled ? html`<p>Configure domains, redirects and HTTPS in your external proxy. Changes here are stored for later use.</p>` : statusCard}</div>
        <div><section class="sec"><div class="sec-h"><h2>Add a domain</h2></div>
          ${isWorker ? html`<div class="note">This service runs on port 0 (background worker) and has no HTTP port for public domains.</div>` : ''}
          <div class="field"><label for="nd">Domain</label><input class="input" id="nd" placeholder="app.acme.co"${isWorker || !!domBusy ? ' disabled' : ''}><span class="hint">A whole domain or a subdomain. No https://.</span></div>
          <div class="field"><label for="nw">www</label><select class="select" id="nw"${isWorker || !!domBusy ? ' disabled' : ''}><option value="none">No www (this domain only)</option><option value="www_to_bare">Redirect www to the bare domain</option><option value="bare_to_www">Redirect the bare domain to www</option><option value="serve_both">Serve both bare and www</option></select></div>
          ${servingDoms.length ? html`<div class="field"><label for="nr">Redirect to</label><select class="select" id="nr"${isWorker || !!domBusy ? ' disabled' : ''}><option value="">Serve the app directly</option>${servingDoms.map((d) => html`<option value="${d.name}">Redirect to ${d.name}</option>`)}</select><span class="hint">Optional: forward all requests from this domain to an existing domain.</span></div>` : ''}
          <div class="item-row"><label><input type="checkbox" id="nf" checked${isWorker || !!domBusy ? ' disabled' : ''}><span><b>Force HTTPS</b><span class="sm muted">Redirect http:// to https:// once the certificate is issued</span></span></label></div>
          <div class="item-row"><label><input type="checkbox" id="nh" checked${isWorker || !!domBusy ? ' disabled' : ''}><span><b>Enable HSTS</b><span class="sm muted">Send Strict-Transport-Security over HTTPS</span></span></label></div>
          <p id="derr" class="form-err" role="alert">${domErr}</p>
          <div>${btn('Add domain', { act: 'adddom', kind: 'primary', disabled: isWorker || !!domBusy })}</div></section>
          <section class="sec"><div class="sec-h"><h2>Certificates</h2></div>
          ${proxyDown ? html`<div class="row mb10">${st('fail', 'Reverse proxy unavailable')}<span class="sm muted">${proxyDisabled ? 'Manage certificates in your external proxy.' : 'Start opsbox-proxy to issue or renew TLS certificates.'}</span></div>` : ''}
          <dl class="sm" style="display:grid;grid-template-columns:1fr auto;gap:10px;margin:0"><dt class="muted">Issuer</dt><dd style="margin:0">Let's Encrypt (HTTP-01)</dd><dt class="muted">Renewal</dt><dd style="margin:0">30 days before expiry</dd><dt class="muted">If renewal fails</dt><dd style="margin:0">Shown on Overview</dd></dl></section></div>
      </div></div>`;
  };

  const detail = () => {
    const s = svc();
    if (!s) return '';
    const isDb = s.kind === 'db';
    const items = isDb ? ['Overview', 'Variables', 'Settings'] : ['Deploys', 'Variables', 'Domains & SSL', 'Settings'];
    const t = Math.min(tab, items.length - 1);
    const dbsInProj = proj.services.filter((x) => x.kind === 'db' && x.id !== s.id);
    const varsBody = () => html`${s.vars.length ? table('Variables', [['Name'], ['Value'], ['', 'r']], s.vars.map(([k, v, isSec, isRefFlag]) => {
      const ref = varRefTarget(v);
      const isRef = Boolean(isRefFlag || ref);
      const hasTarget = ref && proj.services.some((x) => x.id === ref.svc);
      const secret = isSec === true || (isSec === undefined && /URL|SECRET|KEY|PASS/.test(k));
      const shown = revealedVars[k];
      const valCell = isRef
        ? (hasTarget
          ? html`<button type="button" class="btn link mono" data-act="select" data-id="${ref.svc}" title="Open ${ref.svc}">${raw(LINK_ICON)}${v}</button>`
          : html`<span class="mono" style="color:var(--accent)">${raw(LINK_ICON)}${v}</span>`)
        : shown
          ? html`<span class="mono">${shown}</span>`
          : html`<span class="mono muted">${secret ? (v && String(v).startsWith('••••') ? v : '••••••••') : v}</span>`;
      const acts = isRef
        ? btn('Remove', { act: 'rmvar', data: { k }, kind: 'link' })
        : secret
          ? html`<span class="acts">${btn(shown ? 'Hide' : 'Reveal', { act: shown ? 'hidevar' : 'revealvar', data: { k }, kind: 'link' })}${shown ? btn('Copy', { act: 'copyvar', data: { k }, kind: 'link' }) : ''}${btn('Remove', { act: 'rmvar', data: { k }, kind: 'link' })}</span>`
          : btn('Remove', { act: 'rmvar', data: { k }, kind: 'link' });
      return [
        html`<span class="mono">${k}</span>${secret && !isRef ? html` <span class="tag-sm">secret</span>` : ''}${isRef ? html` <span class="tag-sm">linked</span>` : ''}`,
        valCell,
        acts,
      ];
    })) : html`<p class="muted">No variables yet.</p>`}<div class="row mt12">${btn('Add variable', { act: 'addvar' })}${!isDb && dbsInProj.length ? btn('Connect database', { act: 'conndb' }) : ''}</div>`;
    const settingsBody = () => html`<dl class="sm" style="display:grid;grid-template-columns:140px 1fr;gap:8px">
        <dt class="muted">Source</dt><dd class="mono">${s.templateId ? `template:${s.templateId}` : s.composeStack ? `compose:${s.composeStack}` : (s.sourceKind || 'github')} · ${s.repo}</dd>
        ${s.sourceKind === 'docker_image' && !isDb && !s.templateId && !s.composeStack ? html`<dt class="muted">Image</dt><dd><span class="mono">${s.imageRef || s.repo}</span> ${btn('Change image…', { act: 'editImage', kind: 'link' })}<p class="sm muted mt6">Redeploy after saving to pull and run the new image.</p></dd>` : ''}
        <dt class="muted">Branch / Version</dt><dd class="mono">${s.branch || '—'}</dd>
        <dt class="muted">Build</dt><dd>${s.build || '—'}</dd>
        <dt class="muted">Health check</dt><dd class="mono">${s.healthPath || '/'}</dd>
        <dt class="muted">Network</dt><dd class="mono">${exposureLabel(s)}</dd>
        <dt class="muted">Auto-deploy</dt><dd>${s.autoDeploy === false ? 'Off' : 'On'}</dd>
        <dt class="muted">Domain</dt><dd class="mono">${s.domain || '—'}</dd>
        <dt class="muted">Port</dt><dd class="mono">${s.port || '—'}${s.hostPort ? ` (${networkModeOf(s) === 'public' ? '0.0.0.0' : '127.0.0.1'}:${s.hostPort})` : ''}</dd>
      </dl>
      <div class="row mt16">
        ${isDb ? btn('Network exposure…', { act: 'setexposure' }) : ''}
        ${s.sourceKind !== 'docker_image' && !s.templateId && !s.composeStack ? btn(s.autoDeploy === false ? 'Enable auto-deploy' : 'Disable auto-deploy', { act: 'toggleAuto' }) : ''}
        ${/^(adminer|pgweb)$/.test(s.templateId || '') && s.state === 'active' ? html`<a class="btn primary" href="/tools/${encodeURIComponent(s.id)}/" target="_blank" rel="noopener">Open ${s.templateId}</a>` : ''}
        ${btn('Memory limit…', { act: 'memory' })}
        ${btn('Delete service', { act: 'delsvc', kind: 'danger' })}
      </div>`;
    let body;
    if (isDb && t === 0) {
      const connectedApps = s.connectedApps || [];
      const staleApps = connectedApps.filter((c) => c.needsRedeploy);
      body = html`<dl class="sm" style="display:grid;grid-template-columns:140px 1fr;gap:8px">
        <dt class="muted">Engine</dt><dd class="mono">${s.repo}</dd>
        <dt class="muted">Host on network</dt><dd class="mono">${s.id}:${s.port || '—'}</dd>
        <dt class="muted">Network exposure</dt><dd class="mono">${exposureLabel(s)} ${btn('Change', { act: 'setexposure', kind: 'link' })}</dd>
        <dt class="muted">Credentials rotated</dt><dd>${s.credentialsRotatedAt ? ago(s.credentialsRotatedAt) : 'Never'}</dd>
        <dt class="muted">Volume / Backup</dt><dd>${s.note || 'Persistent named volume'}${s.backup ? ` · ${s.backup.enabled ? `auto ${s.backup.method} (${s.backup.cron})` : 'auto backup off'}` : ''}</dd>
      </dl>
      <div class="row mt14">
        ${/^postgres|^postgis$/.test(s.templateId || '') ? html`<a class="btn primary" href="#/databases/${encodeURIComponent(s.id)}">Manage database</a>` : ''}
        ${btn('Rotate credentials', { act: 'rotatedb', disabled: s.state !== 'active' })}
        ${staleApps.length ? btn(`Redeploy ${staleApps.length} stale app${staleApps.length === 1 ? '' : 's'}`, { act: 'redeployStale', kind: 'primary' }) : ''}
      </div>
      ${s.state !== 'active' ? html`<p class="sm muted mt6">Database must be running to rotate credentials.</p>` : ''}
      <div class="mt18">
        ${connectedApps.length ? table('Connected apps', [['App'], ['Variables'], ['Status'], ['', 'r']], connectedApps.map((c) => [
          html`<button type="button" class="btn link mono" data-act="select" data-id="${c.id}">${c.id}</button>`,
          html`${(c.vars || []).map((k) => html`<span class="mono tag-sm">${k}</span> `)}`,
          c.needsRedeploy ? st('warn', 'Needs redeploy') : st('ok', 'Up to date'),
          html`<span class="acts">${c.needsRedeploy ? btn('Redeploy', { act: 'redeployApp', data: { id: c.id }, kind: 'primary' }) : ''}${iconAct('disconndb', `Disconnect ${c.id}`, 'unlink', { app: c.id }, { danger: true })}</span>`,
        ])) : html`<p class="sm muted">No connected apps yet. Open an app's <b>Variables</b> tab and click <b>Connect database</b>.</p>`}
      </div>`;
    }
    else if (isDb && t === 1) body = varsBody();
    else if (isDb) body = settingsBody();
    else if (t === 0) body = html`${s.deploys.length ? table('Deploys', [['Commit'], ['Message'], ['Status'], ['When'], ['', 'r']], s.deploys.map((d) => {
        const [k, l] = DST[d.status] || ['off', d.status];
        const shortCommit = d.commit ? d.commit.slice(0, 7) : d.id.slice(0, 7);
        const trig = d.trigger ? html` <span class="tag-sm">${d.trigger}</span>` : '';
        const errHint = d.status === 'failed' && d.error ? html`<div class="sm err-t">${d.error}</div>` : '';
        return Object.assign([
          html`<span class="mono">${shortCommit}</span>`,
          html`${d.msg}${trig}${errHint}`,
          st(k, l),
          ago(d.at),
          html`<span class="acts">${iconAct('log', logDeploy === d.id ? 'Hide build log' : 'Show build log', 'log', { id: d.id })}${d.status === 'replaced' ? iconAct('rollback', 'Roll back to this deploy', 'rollback', { id: d.id }) : ''}</span>`,
        ], { meta: { key: `d-${d.id}`, sig: d.status } });
      })) : empty('No deploys yet.')}
      ${logDeploy ? html`<div class="mt14"><pre class="code" id="build-log" aria-label="Build log">${logLoading ? 'Loading build log…' : (logLines.join('\n') || '(no log lines yet)')}</pre></div>` : ''}`;
    else if (t === 1) body = varsBody();
    else if (t === 2) body = s.internalOnly
      ? html`<p class="muted">This service runs on the internal project network only (<span class="mono">${s.id}:${s.port || '—'}</span>) without an internet-facing host port.</p>`
      : domainsTab(s);
    else body = settingsBody();
    const canDomain = !isDb && !s.internalOnly && s.port > 0;
    return section(s.id, html`${tabs(items, t, 'tab')}<div style="padding-top:16px">${body}</div>`, html`<span class="row">${canDomain ? btn('Domains', { act: 'gotodomains', kind: 'link' }) : ''}${s.domain ? html`<a class="btn link mono" href="${s.domainScheme || 'http'}://${s.domain}" target="_blank" rel="noopener">${s.domain} ↗</a>` : ''}${btn('Redeploy', { act: 'redeploy', kind: 'primary' })}</span>`);
  };

  const loadDomains = async (targetSvc) => {
    if (!targetSvc || domLoading.has(targetSvc)) return;
    domLoading.add(targetSvc);
    if (domLoadErrors.delete(targetSvc)) paint();
    try {
      const d = await api.getDomains(targetSvc);
      if (sel === targetSvc) doms = d;
    } catch (err) {
      domLoadErrors.set(targetSvc, err.message || 'Try again.');
    } finally {
      domLoading.delete(targetSvc);
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
  // Switching service or tab is a different view (nothing enters or flashes); refreshes animate in place.
  let fresh = false;
  const paint = () => {
    if (!root) return;
    const opts = fresh ? { animate: false } : undefined; fresh = false;
    patch(root.querySelector('#g'), graph().toString(), opts);
    entered = true;
    patch(root.querySelector('#d'), detail().toString(), opts);
    if (tab === 2 && sel && (!doms || doms.service !== sel) && !domLoading.has(sel) && !domLoadErrors.has(sel)) loadDomains(sel);
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
        ...networkActions(refreshDomainsAndProj, (busy) => { domBusy = busy ? 'proxy' : ''; paint(); }),
        memory: async () => { await editMemory(svc()); await reload(); },
        select: (el) => { fresh = true; sel = el.dataset.id; tab = 0; stopWs(); logDeploy = null; logLines = []; revealedVars = {}; hideRevealed(); doms = null; domBusy = ''; domErr = ''; lastDns = null; paint(); },
        tab: (el) => { fresh = true; tab = +el.dataset.i; paint(); },
        retrydomains: () => loadDomains(sel),
        adoptedTls: async (el) => { try { await api.adoptedTls(el.dataset.domain); toast('opsbox now handles TLS.'); await refreshDomainsAndProj(); } catch (e) { toast(e.message, 'err'); } },
        gotodomains: () => { fresh = true; tab = 2; paint(); root.querySelector('#d').scrollIntoView({ behavior: 'smooth', block: 'start' }); },
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
          const r2 = await withCapacityOverride((b) => api.redeploy(sel, b), {});
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
              { name: 'v', label: 'Value', hint: 'Supports ${{ service.KEY }} references to other services in this project.' },
              { name: 'sec', label: 'Visibility', options: ['Auto (mask secret-looking names)', 'Secret (masked & --secret mount)', 'Plaintext (visible & build arg)'], value: 'Auto (mask secret-looking names)' },
            ],
            submit: 'Add',
          }, (o) => {
            const isSec = o.sec.startsWith('Secret') ? true : o.sec.startsWith('Plaintext') ? false : undefined;
            return api.addVar(sel, o.k.trim(), o.v, isSec);
          });
          if (out !== null) await reload();
        },
        conndb: async () => {
          const dbs = proj.services.filter((x) => x.kind === 'db' && x.id !== sel);
          if (!dbs.length) {
            toast('Add a database first from New service → Templates.', 'err');
            return;
          }
          const out = await askForm({
            title: `Connect database to ${sel}`,
            body: 'Inserts a ${{ db.KEY }} reference variable resolved securely inside opsbox-priv at deploy time without duplicating secrets.',
            fields: [
              { name: 'database', label: 'Database', options: dbs.map((d) => d.id), value: dbs[0].id },
              { name: 'scope', label: 'Variables to inject', options: ['Primary connection URL (e.g. DATABASE_URL)', 'All connection variables (URL, host, user, password, DB)'], value: 'Primary connection URL (e.g. DATABASE_URL)' },
              { name: 'asKey', label: 'Variable name override (optional)', placeholder: 'Leave empty for default (DATABASE_URL / REDIS_URL)' },
              ...(dbs.some((x) => /^postgres|^postgis$/.test(x.templateId || '')) ? [{ name: 'login', label: 'PostgreSQL login', options: [OWN_LOGIN, SHARED_LOGIN], value: OWN_LOGIN, hint: 'Its own database and login means this app cannot read the other apps\' data. Variable choices above apply to the shared login only.' }] : []),
            ],
            submit: 'Connect',
          }, (o) => api.connectDatabase(sel, {
            database: o.database,
            includeAll: o.scope.startsWith('All'),
            asKey: o.asKey?.trim() || undefined,
            perAppUser: o.login === SHARED_LOGIN ? false : undefined,
          }));
          if (out !== null) {
            toast(`Connected ${out.database} to ${sel}.`);
            await reload();
            if (out.needsRedeploy && await confirmDialog({ title: `Redeploy ${sel}?`, body: `${sel} is currently running. Redeploy now so the container receives the connected database reference.`, confirm: 'Redeploy now' })) {
              const r2 = await withCapacityOverride((b) => api.redeploy(sel, b), {});
              toast('Deploy started.');
              tab = 0;
              await reload();
              if (r2?.id) await openLogFor(r2.id);
            }
          }
        },
        disconndb: async (el) => {
          const appId = el.dataset.app;
          if (!await confirmDialog({ title: `Disconnect ${appId}?`, body: `Removes reference variables linking ${appId} to ${sel}. ${appId} will need a redeploy to pick up the removal.`, confirm: 'Disconnect', danger: true })) return;
          await api.disconnectDatabase(appId, sel);
          toast(`Disconnected ${appId} from ${sel}.`);
          await reload();
        },
        rotatedb: async () => {
          const s = svc();
          const count = s?.connectedApps?.length || 0;
          const warnApps = count ? ` ${count} connected service${count === 1 ? '' : 's'} will need to be redeployed to use the new password.` : '';
          if (!await confirmDialog({ title: `Rotate credentials for ${sel}?`, body: `Updates the password inside the running ${sel} container and re-seals credentials at rest.${warnApps}`, confirm: 'Rotate credentials' })) return;
          const res = await api.rotateDatabase(sel);
          toast(`Credentials rotated for ${sel}.`);
          await reload();
          const stale = (res.affectedApps || []).filter((a) => a.needsRedeploy);
          if (stale.length && await confirmDialog({ title: 'Redeploy connected apps?', body: `${stale.map((a) => a.id).join(', ')} use${stale.length === 1 ? 's' : ''} rotated credentials and need a redeploy.`, confirm: 'Redeploy affected apps' })) {
            for (const a of stale) {
              await api.redeploy(a.id);
            }
            toast(`Started redeploy for ${stale.map((a) => a.id).join(', ')}.`);
            await reload();
          }
        },
        redeployApp: async (el) => {
          await api.redeploy(el.dataset.id);
          toast(`Redeploy started for ${el.dataset.id}.`);
          await reload();
        },
        redeployStale: async () => {
          const s = svc();
          const stale = (s?.connectedApps || []).filter((a) => a.needsRedeploy);
          for (const a of stale) {
            await api.redeploy(a.id);
          }
          toast(`Started redeploy for ${stale.map((a) => a.id).join(', ')}.`);
          await reload();
        },
        setexposure: async () => {
          const s = svc();
          if (!s) return;
          const curMode = networkModeOf(s);
          const labels = {
            internal: 'Private project network only (Recommended)',
            loopback: 'Loopback only (127.0.0.1)',
            public: 'Public internet port (0.0.0.0 — NOT recommended)',
          };
          const out = await formDialog({
            title: `Network exposure for ${sel}`,
            body: 'Databases stay on the private project network by default so no ports are exposed on the host.',
            fields: [
              { name: 'mode', label: 'Exposure mode', options: [labels.internal, labels.loopback, labels.public], value: labels[curMode] || labels.internal },
            ],
            submit: 'Save',
          });
          if (!out) return;
          const mode = out.mode.startsWith('Public') ? 'public' : out.mode.startsWith('Loopback') ? 'loopback' : 'internal';
          let confirmed = false;
          if (mode === 'public') {
            confirmed = await confirmDialog({
              title: `Publish ${sel} port ${s.port || ''} to the internet?`,
              body: 'Exposing a database port on 0.0.0.0 makes it reachable from the internet and subject to automated scanners and brute-force attacks. Only continue if you have restricted access via a firewall.',
              confirm: 'Publish publicly',
              danger: true,
            });
            if (!confirmed) return;
          }
          const res = await api.setServiceExposure(sel, { mode, confirm: confirmed });
          toast(`Updated ${sel} network exposure to ${res.mode}.`);
          await reload();
          if (res.redeployRequired && await confirmDialog({ title: `Restart ${sel} to apply port change?`, body: 'Docker port bindings apply when the container is restarted via Redeploy.', confirm: 'Redeploy now' })) {
            await withCapacityOverride((b) => api.redeploy(sel, b), {});
            toast(`Redeploying ${sel}…`);
            await reload();
          }
        },
        rmvar: async (el) => { await api.removeVar(sel, el.dataset.k); delete revealedVars[el.dataset.k]; await reload(); },
        revealvar: async (el) => {
          const k = el.dataset.k;
          try {
            await withReveal(
              () => api.revealVar(sel, k).then((r) => r.value),
              (plain) => {
                if (plain == null) delete revealedVars[k];
                else revealedVars[k] = plain;
                paint();
              },
            );
          } catch (e) { toast(e.message || 'Could not reveal.', 'err'); }
        },
        hidevar: (el) => { delete revealedVars[el.dataset.k]; hideRevealed(); paint(); },
        copyvar: async (el) => {
          const plain = revealedVars[el.dataset.k];
          if (plain) await copyPlain(plain);
          else {
            try {
              const r = await withReveal(() => api.revealVar(sel, el.dataset.k).then((x) => x.value), () => {});
              if (r) await copyPlain(r);
            } catch (e) { toast(e.message || 'Could not copy.', 'err'); }
          }
        },
        editImage: async () => {
          const s = svc();
          if (!s) return;
          const out = await askForm({
            title: `Change image for ${sel}`,
            fields: [{ name: 'image', label: 'Docker image', value: s.imageRef || s.repo,
              hint: 'Redeploy after saving to pull and run the new image.' }],
            submit: 'Save',
          }, (o) => withCapacityOverride((b) => api.updateService(sel, b), { image: o.image.trim() }));
          if (out !== null) {
            toast('Image saved. Redeploy to apply it.');
            await reload();
          }
        },
        toggleAuto: async () => {
          const s = svc();
          if (!s) return;
          await api.updateService(sel, { autoDeploy: !(s.autoDeploy !== false) });
          toast('Auto-deploy updated.');
          await reload();
        },
        delsvc: async () => {
          const s = svc();
          const refs = (s?.connectedApps || []).map((a) => a.id);
          const extra = refs.length ? ` Warning: ${refs.join(', ')} still reference this database!` : '';
          if (await confirmDialog({ title: `Delete ${sel}?`, body: `The service stops and its container is removed. This cannot be undone.${extra}`, confirm: 'Delete', danger: true, typed: sel })) {
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
        if (tab === 2 && domLoadErrors.has(sel)) loadDomains(sel);
        api.getProject().then((p) => {
          // A finished deploy is worth saying out loud; a changing metric is not.
          p.services.forEach((s) => {
            const was = proj.services.find((x) => x.id === s.id)?.deploys?.[0];
            const now = s.deploys?.[0];
            if (was && now && was.id === now.id && was.status === 'building' && now.status !== 'building') announce(now.status === 'failed' ? `Deploy of ${s.id} failed.` : `${s.id} deployed.`);
          });
          const changed = p.services.length !== proj.services.length
            || p.services.some((s, i) => s.state !== proj.services[i]?.state || s.deploys?.[0]?.status !== proj.services[i]?.deploys?.[0]?.status || JSON.stringify(s.connectedApps || []) !== JSON.stringify(proj.services[i]?.connectedApps || []));
          if (changed) { proj = p; paint(); }
        }).catch(() => {});
      };
      listeners.add(this._t);
    },
    unmount() { stopWs(); listeners.delete(this._t); },
  };
}
