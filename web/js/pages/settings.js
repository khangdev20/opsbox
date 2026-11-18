import { html, st, btn, table, section, header, sw, bindActions, confirmDialog, askForm, toast, download, bytes, dropzone, esc } from '../ui.js';
import * as api from '../api.js';
import { networkCard, networkActions } from '../network.js';
import { noOfferText, checkedLine, checkToast } from '../update-status.js';

const TZ = ['Asia/Ho_Chi_Minh', 'UTC', 'Australia/Brisbane'];
const SWAP = ['None', '1 GB', '2 GB', '4 GB'];
const LIVE = api.MODE === 'http';
const normDom = (s) => String(s || '').trim().toLowerCase().replace(/^https:\/\//, '').replace(/\/$/, '');
export async function view({ query = {} } = {}) {
  let ghErr = '', ghOrg = '';
  let uninstallBusy = false, uninstallGone = false;
  let importDoc = null, importName = '', importPass = '', importPreview = null, importBusy = false, importErr = '';
  const clearHashQuery = () => {
    try { history.replaceState(null, '', '#/settings'); } catch { /* ignore */ }
  };
  if (query.github_code && query.github_state) {
    try {
      await api.completeGithubManifest(query.github_code, query.github_state);
      toast('GitHub App created. Choose repositories on GitHub to finish connecting.');
    } catch (e) { ghErr = e.message; }
    clearHashQuery();
  } else if (query.github_installation_id) {
    try {
      await api.syncGithubInstallation(query.github_installation_id);
      toast('GitHub repositories connected.');
    } catch (e) { ghErr = e.message; }
    clearHashQuery();
  } else if (query.github_error) {
    ghErr = query.github_error === 'cancelled' ? 'GitHub connection was cancelled.' : 'GitHub connection did not finish.';
    clearHashQuery();
  }
  let d = await api.getSettings(), root;
  let proxyNetwork = await api.getProxyNetwork().catch((e) => ({ error: e.message })), proxyBusy = false;
  let disk = await api.getDisk().catch((e) => ({ error: e.message })), growing = '';
  let dns = null, busy = '', panelErr = '', ghBusy = false;
  const upd = () => d.opsboxUpdate || { current: '0.1.0', channel: d.channel || 'stable', auto: !!d.autoUpdate, window: 3, updateAvailable: false, latest: null, reason: null, checkedAt: null, previousVersion: null, lastOutcome: null };
  const panelInfo = () => d.panelInfo || { domain: d.panel || null, activeDomain: null, certStatus: 'none', proxyReachable: true, rawPortOpen: true, listen: '0.0.0.0:8443' };
  const hours = Array.from({ length: 24 }, (_, i) => i);
  const canIssueFor = (val) => {
    const pi = panelInfo();
    if (busy || (LIVE && pi.proxyReachable === false)) return false;
    const norm = normDom(val);
    if (!norm) return false;
    const dnsOk = !!(dns && dns.pointsHere && dns.domain === norm);
    const activeLive = !!(pi.activeDomain && pi.activeDomain === norm && (pi.certStatus === 'valid' || pi.certStatus === 'expiring'));
    return dnsOk || activeLive;
  };
  const renderCertStatus = (pi) => {
    if (LIVE && pi.proxyReachable === false) {
      return html`<div class="row">${st('fail', 'Reverse proxy unavailable')}<span class="sm muted">Start opsbox-proxy to check proxy status or issue certificates.</span></div>`;
    }
    if (pi.certStatus === 'valid') {
      const days = pi.certDaysRemaining ?? '?';
      const stage = pi.certStaging ? " (Let's Encrypt staging)" : '';
      return html`<div class="row">${st('ok', `Valid · ${days} days left${stage}`)}${pi.activeDomain ? html`<a href="https://${pi.activeDomain}" target="_blank" rel="noopener" class="mono sm">https://${pi.activeDomain}</a>` : ''}</div>`;
    }
    if (pi.certStatus === 'expiring') {
      return html`<div class="row">${st('warn', `Expiring soon · ${pi.certDaysRemaining ?? '?'} days left`)}<span class="sm muted">Renews automatically 30 days before expiry.</span></div>`;
    }
    if (pi.certStatus === 'pending') {
      return html`<div class="row">${st('build', 'Issuing certificate...')}</div>`;
    }
    if (pi.certStatus === 'error') {
      return html`<div class="col gap6">${st('fail', 'Certificate error')}${pi.certError ? html`<span class="sm err-t">${pi.certError}</span>` : ''}</div>`;
    }
    return '';
  };
  const renderGithubSection = (pi) => {
    const gh = d.github && typeof d.github === 'object'
      ? d.github
      : (d.github ? { connected: true, installed: true, account: 'acme-co', appSlug: 'opsbox-demo', manageUrl: 'https://github.com/settings/installations', webhookConfigured: true, webhookActive: true, webhookUrl: 'https://panel.example.com/api/v1/github/webhook' } : { connected: false, installed: false });
    const keys = d.deployKeys || [];
    const selNote = gh.repositorySelection === 'all'
      ? 'all repositories'
      : gh.repositorySelection === 'selected'
        ? 'selected repositories'
        : 'installed';
    let mainBlock;
    if (!gh.connected) {
      mainBlock = html`<div class="col gap12">
        <div class="row spread">
          <div><b>Not connected</b><div class="sm muted">Create a private GitHub App owned by your account or organization via the manifest flow.</div></div>
        </div>
        <div class="row">
          <input class="input grow" id="gh-org" placeholder="Organization name (optional — leave blank for personal account)" value="${ghOrg}"${ghBusy ? ' disabled' : ''}>
          ${btn(ghBusy ? 'Connecting…' : 'Connect GitHub', { act: 'ghConnect', kind: 'primary', disabled: ghBusy })}
        </div>
        ${!pi.activeDomain ? html`<div class="note"><b>Note on webhooks:</b> No panel domain is configured yet. You can still connect GitHub to browse repositories, and set a panel domain above whenever you want automatic push webhooks.</div>` : ''}
      </div>`;
    } else if (!gh.installed) {
      mainBlock = html`<div class="col gap12">
        <div class="row spread">
          <div>
            <div class="row" style="gap:8px"><b>${gh.appName || gh.appSlug || 'GitHub App'}</b>${st('warn', 'Awaiting repository access')}</div>
            <div class="sm muted mono">App: ${gh.appSlug} · owner: ${gh.owner} · install it on your account or organization to pick repositories</div>
          </div>
          <div class="row">
            ${gh.installUrl ? html`<a class="btn primary" href="${gh.installUrl}" target="_blank" rel="noopener">Choose repositories on GitHub ↗</a>` : ''}
            ${btn('Refresh', { act: 'ghRefresh', disabled: ghBusy })}
            ${btn('Disconnect', { act: 'ghDisconnect', kind: 'danger', disabled: ghBusy })}
          </div>
        </div>
        ${!pi.activeDomain ? html`<div class="note"><b>Push webhooks inactive:</b> Set a panel domain in <b>Panel address</b> above so GitHub can deliver push events to <span class="mono">https://&lt;your-domain&gt;/api/v1/github/webhook</span>.</div>` : ''}
      </div>`;
    } else {
      mainBlock = html`<div class="col gap12">
        <div class="row spread">
          <div>
            <div class="row" style="gap:8px"><b>github.com/${gh.account || gh.owner}</b>${st('ok', 'Connected')}</div>
            <div class="sm muted mono">GitHub App (${gh.appSlug}) · read-only code access · ${selNote}</div>
          </div>
          <div class="row">
            ${gh.manageUrl ? html`<a class="btn" href="${gh.manageUrl}" target="_blank" rel="noopener">Manage access ↗</a>` : ''}
            ${btn('Refresh', { act: 'ghRefresh', disabled: ghBusy })}
            ${btn('Disconnect', { act: 'ghDisconnect', kind: 'danger', disabled: ghBusy })}
          </div>
        </div>
        ${gh.webhookConfigured
          ? html`<div class="sm muted mono">Webhook: ${gh.webhookUrl} · push events (HMAC-SHA256)${gh.lastWebhookAt ? ` · last delivery: ${gh.lastWebhookEvent || 'ok'}` : ''}</div>`
          : !pi.activeDomain
            ? html`<div class="note"><b>Push webhooks inactive:</b> Set a panel domain in <b>Panel address</b> above so GitHub can deliver push events to <span class="mono">https://&lt;your-domain&gt;/api/v1/github/webhook</span>.</div>`
            : html`<div class="note row spread"><span>Panel domain <b class="mono">${pi.activeDomain}</b> is active. Enable GitHub push webhooks now.</span>${btn('Enable push webhook', { act: 'ghSyncWebhook', kind: 'primary', disabled: ghBusy })}</div>`}
      </div>`;
    }
    const keysBlock = keys.length
      ? html`<div class="col gap6" style="border-top:1px solid var(--ls);padding-top:14px">
          <div class="sm"><b>Per-repo SSH deploy keys</b></div>
          ${table('Deploy keys', [['Repository'], ['Fingerprint'], ['', 'r']], keys.map((k) => [
            html`<span class="mono">${k.repo}</span>`,
            html`<span class="mono sm muted">${k.fingerprint}</span>`,
            btn('Remove', { act: 'ghRmKey', data: { id: k.id }, kind: 'link' }),
          ]))}
        </div>`
      : '';
    return html`<div class="col gap12">${ghErr ? html`<p class="form-err" role="alert">${ghErr}</p>` : ''}${mainBlock}${keysBlock}</div>`;
  };
  const body = () => {
    const u = upd();
    const pi = panelInfo();
    const certLive = !!(pi.activeDomain && (pi.certStatus === 'valid' || pi.certStatus === 'expiring'));
    const curNorm = normDom(d.panel);
    const dnsOk = !!(dns && dns.pointsHere && dns.domain === curNorm);
    const canIssue = canIssueFor(d.panel);
    const notesLink = u.latest?.notesUrl ? html` · <a href="${u.latest.notesUrl}" target="_blank" rel="noopener">release notes</a>` : '';
    const why = noOfferText(u);
    const statusText = u.updateAvailable && u.latest
      ? html`opsbox ${u.current} · <b>${u.latest.version} available</b>${notesLink}${u.migrationNote ? html`<br><span class="sm muted">${u.migrationNote}</span>` : ''}`
      : html`${why.text}${why.detail ? html`<br><span class="sm muted">${why.detail}</span>` : ''}`;
    const chanSource = { setting: 'set here', config: 'set when opsbox was installed', default: 'chosen automatically' }[u.channelSource] || '';
    return html`
    ${section('General', html`<div style="max-width:520px;display:flex;flex-direction:column;gap:16px">
      <div class="field"><label for="sh">Hostname</label><input class="input" id="sh" value="${d.hostname}"></div>
      <div class="field"><label for="sz">Time zone</label><select class="select" id="sz">${(d.timezones || TZ).map((z) => html`<option${z === d.tz ? ' selected' : ''}>${z}</option>`)}</select><span class="hint">Used for cron jobs, backups and alert quiet hours.</span></div>
      <p id="err" class="form-err" role="alert"></p><div>${btn('Save', { act: 'save', kind: 'primary' })}</div></div>`)}
    <div id="network-card">${networkCard(proxyNetwork, proxyBusy)}</div>
    ${section('Panel address', html`<div class="col gap16">
      <div class="field"><label for="sp">Panel domain</label>
        <div class="row">
          <input class="input grow" id="sp" placeholder="panel.example.com" value="${d.panel || ''}"${busy ? ' disabled' : ''}>
          ${btn('Check DNS', { act: 'checkDns', kind: dnsOk ? '' : 'primary', disabled: !!busy })}
          ${btn(certLive && pi.activeDomain === curNorm ? 'Renew certificate' : 'Issue certificate', { act: 'issueCert', kind: dnsOk ? 'primary' : '', disabled: !canIssue })}
          ${pi.activeDomain ? btn('Remove', { act: 'removeDomain', kind: 'link', disabled: !!busy }) : ''}
        </div>
        <span class="hint">Point a DNS A or AAAA record to this server, click Check DNS, then Issue certificate.</span>
      </div>
      ${busy ? html`<div class="row">${st('build', busy)}</div>` : ''}
      ${panelErr ? html`<p class="form-err" role="alert">${panelErr}</p>` : ''}
      ${renderCertStatus(pi)}
      ${dns ? html`<div class="col gap12">
        <div class="row">${dns.pointsHere ? st('ok', 'points here') : st('warn', 'waiting for DNS')}<span class="sm muted">${dns.message}</span></div>
        ${dns.records?.length ? table('DNS records', [['Type'], ['Name'], ['Value'], ['', 'r']], dns.records.map((r) => [html`<span class="mono">${r.type}</span>`, html`<span class="mono">${r.name}</span>`, html`<span class="mono">${r.value}</span>`, btn('Copy', { act: 'copyRec', data: { val: r.value }, kind: 'link' })])) : ''}
      </div>` : ''}
      ${certLive && pi.rawPortOpen ? html`<div class="row spread" style="border-top:1px solid var(--ls);padding-top:14px">
        <div style="max-width:560px"><div><b>Close raw port 8443?</b></div><p class="muted sm">The panel is live at <a href="https://${pi.activeDomain}" target="_blank" rel="noopener" class="mono">https://${pi.activeDomain}</a>. Restrict port 8443 to <span class="mono">127.0.0.1:8443</span> so traffic only enters via HTTPS on port 443.</p></div>
        ${btn('Close port 8443', { act: 'closeRawPort', disabled: !!busy })}
      </div>` : ''}
      ${!pi.rawPortOpen ? html`<div class="row spread" style="border-top:1px solid var(--ls);padding-top:14px">
        <div class="row">${st('ok', 'Port 8443 restricted to localhost')}<span class="mono sm muted">${pi.listen || '127.0.0.1:8443'}</span></div>
        ${btn('Re-open port 8443', { act: 'openRawPort', kind: 'link', disabled: !!busy })}
      </div>` : ''}
    </div>`)}
    ${section('API tokens', html`${table('API tokens', [['Name'], ['Can do'], ['Last used'], ['', 'r']], d.tokens.map((t) => [html`<span class="mono">${t.name}</span>`, t.can, t.last, btn('Revoke', { act: 'revoke', data: { id: t.id }, kind: 'link' })]))}<div>${btn('Create token', { act: 'token' })}</div>`)}
    ${section('Memory and disk', html`<div style="max-width:520px;display:flex;flex-direction:column;gap:16px"><p class="muted sm">Swap protects small instances from running out of memory. To get more disk space, enlarge the volume at your cloud provider first; opsbox then grows the partition and filesystem for you.</p>
      <div class="field"><label for="sw">Swap file</label><select class="select" id="sw" data-act="swap">${(SWAP.includes(d.swap) ? SWAP : [...SWAP, d.swap]).map((v) => html`<option${v === d.swap ? ' selected' : ''}>${v}</option>`)}</select></div>
      ${diskBlock()}</div>`)}
    ${section('GitHub', renderGithubSection(pi))}
    ${section('opsbox updates', html`<div class="list-row"><span>${statusText}<br><span class="sm muted" id="ucheck">${checkedLine(u)}</span></span><div class="row">${btn('Check now', { act: 'check' })}${u.updateAvailable ? btn('Update now', { act: 'updateNow', kind: 'primary' }) : ''}${u.previousVersion ? btn(`Roll back to ${u.previousVersion}`, { act: 'rollback' }) : ''}</div></div>${u.lastOutcome ? html`<p class="sm muted">${u.lastOutcome.message}</p>` : ''}<div class="list-row"><span>Release channel${chanSource ? html` <span class="sm muted">(${chanSource})</span>` : ''}</span><select class="select" id="uch" style="width:auto"><option value="stable"${u.channel === 'stable' ? ' selected' : ''}>stable</option><option value="beta"${u.channel === 'beta' ? ' selected' : ''}>beta</option></select></div><div class="list-row"><span>Update opsbox automatically</span><div class="row">${sw('Update opsbox automatically', u.auto, 'autoup')}<select class="select" id="uwin" aria-label="Daily update window" style="width:auto">${hours.map((h) => html`<option value="${h}"${h === (u.window ?? 3) ? ' selected' : ''}>${String(h).padStart(2, '0')}:00 UTC</option>`)}</select></div></div><p class="muted sm">If a new version fails its health check after 2 minutes, opsbox rolls back to the previous one by itself.</p>`)}
    ${section('Export, import and uninstall', html`
      <p class="muted sm" style="max-width:620px">Export saves your services, domains, databases (not their data), backups, alerts, firewall rules and people as a file you can import on a new server. Passwords, keys and tokens stay out of the file unless you protect them with a passphrase.</p>
      <div class="row">${btn('Export configuration', { act: 'export' })}</div>
      <div class="col gap12" style="max-width:620px;margin-top:20px">
        <p class="muted sm">Import recreates configuration on this server. Existing services and other objects are never overwritten. Services that still need secrets are created but not deployed until you enter those secrets.</p>
        ${dropzone({ id: 'imp-file', accept: 'application/json,.json', label: 'Drop an opsbox-config file here or', browse: 'browse' })}
        ${importName ? html`<p class="sm muted">Chosen: <span class="mono">${esc(importName)}</span></p>` : ''}
        <div class="field"><label for="imp-pass">Passphrase for sealed secrets</label><input class="input" type="password" id="imp-pass" value="${esc(importPass)}" autocomplete="off" placeholder="Leave empty if the file has no secrets"><p class="hint">Required only when the export was sealed with a passphrase.</p></div>
        ${importErr ? html`<p class="sm" style="color:var(--danger)" role="alert">${esc(importErr)}</p>` : ''}
        <div class="row">${btn('Preview import', { act: 'importPreview', disabled: !importDoc || importBusy })}${btn('Apply import', { act: 'importApply', kind: 'primary', disabled: !importPreview || importBusy })}</div>
        ${importPreview ? importPreviewBlock(importPreview) : ''}
      </div>
      <p class="muted sm" style="max-width:620px;margin-top:20px">Uninstall removes opsbox but leaves your apps and data running. Containers keep their restart policy so they survive reboots without opsbox.</p>
      ${uninstallGone ? html`<div class="col gap12" style="max-width:620px;border:1px solid var(--ls);padding:16px;border-radius:8px" role="status"><b>opsbox is being removed</b><p class="muted sm">Services are stopping. This page will stop responding in a moment. Your apps keep running on their ports.</p></div>` : html`<div class="row">${btn('Uninstall opsbox…', { act: 'uninstall', kind: 'danger', disabled: !!uninstallBusy })}</div>`}`)}`;
  };
  const importPreviewBlock = (p) => {
    const list = (title, items) => {
      const flat = [];
      if (Array.isArray(items)) items.forEach((x) => flat.push(typeof x === 'string' ? x : (x.name || x.domain || x.key || x.id || JSON.stringify(x))));
      else if (items && typeof items === 'object') Object.entries(items).forEach(([k, v]) => (Array.isArray(v) ? v : []).forEach((n) => flat.push(`${k}: ${n}`)));
      if (!flat.length) return '';
      return html`<div><b class="sm">${title}</b><ul class="sm" style="margin:6px 0 0;padding-left:1.2em">${flat.slice(0, 40).map((t) => html`<li>${esc(t)}</li>`)}${flat.length > 40 ? html`<li class="muted">…and ${flat.length - 40} more</li>` : ''}</ul></div>`;
    };
    const needs = (p.needsInput || []).map((e) => {
      if (e.kind === 'serviceVar') return `${e.service}.${e.key} (secret)`;
      if (e.kind === 'dns') return `DNS: ${e.domain}`;
      if (e.kind === 'github') return 'Connect GitHub';
      if (e.kind === 'deployKey') return `Deploy key for ${e.service || e.repo || 'a repository'}`;
      if (e.kind === 'backupDestination') return `Backup destination ${e.name || e.id}`;
      if (e.kind === 'alertChannel') return `Alert channel ${e.id}`;
      return e.reason || e.kind;
    });
    const skipped = (p.skipped || []).map((e) => `${e.kind}: ${e.name}${e.reason ? ` — ${e.reason}` : ''}`);
    return html`<div id="imp-plan" class="col gap12" style="border:1px solid var(--ls);padding:14px;border-radius:8px" role="status">
      ${list('Will create', p.willCreate)}
      ${list('Needs input', needs)}
      ${list('Skipped (already exists)', skipped)}
      ${p.secretsDecrypted ? html`<p class="sm muted">Sealed secrets were opened with your passphrase.</p>` : ''}
    </div>`;
  };
  // One block per filesystem opsbox watches (GET /system/disk). Grow shows only when there is space to take.
  const diskBlock = () => {
    if (disk.error) return html`<p class="sm muted" role="status">Could not read the disk: ${disk.error}</p>`;
    return html`${(disk.volumes || []).map((v) => html`<div class="col gap12" style="border-top:1px solid var(--ls);padding-top:14px">
      <div class="row spread"><span><b class="mono">${v.mount}</b> <span class="sm muted mono">${v.device} · ${v.fs}</span></span><span class="mono sm muted">filesystem ${bytes(v.fs_bytes)}${v.disk_bytes ? ` · disk ${bytes(v.disk_bytes)}` : ''}</span></div>
      ${v.growable ? html`<div class="row spread">${st('warn', `${bytes(v.growable_bytes)} not used yet`)}${btn(growing === v.mount ? 'Growing…' : 'Grow filesystem to fill disk', { act: 'grow', data: { mount: v.mount }, kind: 'primary', disabled: !!growing })}</div>
        <p class="sm muted">The disk is bigger than the filesystem. opsbox grows the partition, then the filesystem, while everything keeps running.${(v.steps || []).includes('install_growpart') ? ' It installs the growpart tool first.' : ''}</p>`
      : v.refusal ? html`<div class="row">${st('warn', 'Cannot grow automatically')}</div><p class="sm muted">${v.refusal}</p>`
      : html`<p class="sm muted">All of the disk is in use. To get more space, enlarge the volume at your cloud provider first, then come back here.</p>`}
    </div>`)}`;
  };
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  const refreshNetwork = async () => {
    proxyNetwork = await api.getProxyNetwork().catch((e) => ({ error: e.message }));
    const card = root?.querySelector('#network-card');
    if (card) card.innerHTML = networkCard(proxyNetwork, proxyBusy).toString();
  };
  const reload = async () => { d = await api.getSettings(); disk = await api.getDisk().catch((e) => ({ error: e.message })); await refreshNetwork(); paint(); };
  return {
    unmount() { clearInterval(this._networkTimer); root = null; },
    nav: null, title: 'Settings',
    html: html`<main class="narrow">${header('Settings', `${d.hostname} · opsbox 0.1.0`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      if (query.section === 'network') root.querySelector('#network')?.scrollIntoView();
      if (query.section === 'tokens') [...root.querySelectorAll('.sec h2')].find((h) => h.textContent === 'API tokens')?.closest('.sec').scrollIntoView();
      this._networkTimer = setInterval(() => { if (!proxyBusy) refreshNetwork(); }, 5000);
      root.addEventListener('input', (e) => {
        if (e.target.id === 'sp') {
          d.panel = e.target.value;
          const b = root.querySelector('[data-act="issueCert"]');
          if (b) b.disabled = !canIssueFor(d.panel);
        }
        if (e.target.id === 'gh-org') ghOrg = e.target.value;
        if (e.target.id === 'imp-pass') {
          importPass = e.target.value;
          if (importPreview) {
            importPreview = null;
            const applyBtn = root.querySelector('[data-act="importApply"]');
            if (applyBtn) applyBtn.disabled = true;
            root.querySelector('#imp-plan')?.remove();
          }
        }
      });
      root.addEventListener('change', async (e) => {
        // A select that calls the API is locked while it runs, so a second change cannot overlap.
        const t = e.target, locked = t.matches?.('#sw,#uch,#uwin');
        if (locked) { if (t.hasAttribute('data-busy')) return; t.disabled = true; t.setAttribute('data-busy', '1'); }
        try { await onChange(e); } finally { if (locked && t.isConnected) { t.disabled = false; t.removeAttribute('data-busy'); } }
      });
      const readImportFile = async (file) => {
        importErr = ''; importPreview = null; importDoc = null; importName = '';
        if (!file) { paint(); return; }
        importName = file.name || 'opsbox-config.json';
        try {
          const text = await file.text();
          const doc = JSON.parse(text);
          if (!doc || typeof doc !== 'object') throw new Error('The file is not a JSON object.');
          importDoc = doc;
        } catch (err) {
          importErr = err.message || 'Could not read that file as JSON.';
        }
        paint();
      };
      const onChange = async (e) => {
        if (e.target.id === 'sw') { toast('Changing swap. This can take a minute.'); try { await api.setSwap(e.target.value); toast('Swap updated.'); } catch (err) { toast(err.message, 'err'); } await reload(); }
        if (e.target.id === 'uch') { try { d.opsboxUpdate = await api.setOpsboxAutoUpdate({ channel: e.target.value }); toast(`Channel set to ${e.target.value}.`); } catch (err) { toast(err.message, 'err'); } paint(); }
        if (e.target.id === 'uwin') { try { d.opsboxUpdate = await api.setOpsboxAutoUpdate({ window: Number(e.target.value) }); toast('Update window saved.'); } catch (err) { toast(err.message, 'err'); } paint(); }
        if (e.target.id === 'imp-file') await readImportFile(e.target.files?.[0]);
      };
      bindActions(root, {
        ...networkActions(refreshNetwork, (value) => { proxyBusy = value; const card = root?.querySelector('#network-card'); if (card) card.innerHTML = networkCard(proxyNetwork, proxyBusy).toString(); }),
        save: async () => { try { await api.saveSettings({ hostname: root.querySelector('#sh').value, tz: root.querySelector('#sz').value }); toast('Saved.'); await reload(); } catch (e) { root.querySelector('#err').textContent = e.message; } },
        checkDns: async () => {
          const val = root.querySelector('#sp')?.value ?? '';
          d.panel = val; busy = 'Checking DNS...'; panelErr = ''; paint();
          try {
            dns = await api.checkPanelDns(val);
            d.panel = dns.domain;
            toast(dns.pointsHere ? 'DNS points here.' : 'Waiting for DNS.', dns.pointsHere ? 'ok' : '');
          } catch (e) { dns = null; panelErr = e.message; }
          busy = ''; paint();
        },
        issueCert: async () => {
          const val = root.querySelector('#sp')?.value ?? '';
          d.panel = val; busy = 'Issuing certificate (HTTP-01)...'; panelErr = ''; paint();
          try {
            const cert = await api.issuePanelCert(val);
            toast(`Certificate issued for ${cert?.domain || normDom(val)}.`);
            busy = '';
            await reload();
          } catch (e) { panelErr = e.message; busy = ''; paint(); }
        },
        removeDomain: async () => {
          const pi = panelInfo();
          const bodyText = !pi.rawPortOpen
            ? 'Port 8443 is currently localhost-only. Removing the panel domain leaves only local SSH-tunnel access until you re-open port 8443.'
            : 'The reverse proxy will stop serving the panel on HTTPS port 443.';
          if (!await confirmDialog({ title: 'Remove panel domain?', body: bodyText, confirm: 'Remove', danger: true })) return;
          try {
            await api.removePanelDomain(!pi.rawPortOpen);
            dns = null; panelErr = '';
            toast('Panel domain removed.');
            await reload();
          } catch (e) {
            if (e.code === 'confirmation_required' && await confirmDialog({ title: 'Confirm domain removal', body: e.message, confirm: 'Remove anyway', danger: true })) {
              await api.removePanelDomain(true);
              dns = null; panelErr = '';
              toast('Panel domain removed.');
              await reload();
            } else {
              panelErr = e.message; paint();
            }
          }
        },
        closeRawPort: async () => {
          const pi = panelInfo();
          const bodyText = pi.activeDomain
            ? `The panel will stay reachable at https://${pi.activeDomain}. Direct port 8443 will move to 127.0.0.1:8443 and the panel will restart briefly.`
            : 'Direct port 8443 will move to 127.0.0.1:8443 and the panel will restart briefly.';
          if (!await confirmDialog({ title: 'Close raw port 8443?', body: bodyText, confirm: 'Close port 8443' })) return;
          try {
            await api.setPanelLocalOnly(true, false);
            toast(pi.activeDomain ? `Closing port 8443. Open https://${pi.activeDomain} after the brief restart.` : 'Closing port 8443. The panel is restarting...');
            setTimeout(() => reload().catch(() => {}), 1500);
          } catch (e) { panelErr = e.message; paint(); }
        },
        openRawPort: async () => {
          if (!await confirmDialog({ title: 'Re-open port 8443?', body: 'Direct TLS access on port 8443 will accept external connections again after a brief restart.', confirm: 'Re-open port 8443' })) return;
          try {
            await api.setPanelLocalOnly(false, true);
            toast('Re-opening port 8443. The panel is restarting...');
            setTimeout(() => reload().catch(() => {}), 1500);
          } catch (e) { panelErr = e.message; paint(); }
        },
        copyRec: async (el) => { try { await navigator.clipboard?.writeText(el.dataset.val || ''); toast('Copied.'); } catch { toast('Copy the value from the table.'); } },
        token: async () => {
          let svcScopes = [];
          try {
            const p = await api.getProject();
            svcScopes = (p.services || []).map((s) => `deploy:${s.id}`);
          } catch { /* ignore */ }
          const out = await askForm({
            title: 'Create API token',
            fields: [
              { name: 'n', label: 'Name', placeholder: 'ci-deploy' },
              { name: 'c', label: 'Can do', options: ['deploy only', ...svcScopes, 'read-only metrics', 'full access'], value: 'deploy only' },
              { name: 'svc', label: 'Or specific service name (overrides above)', placeholder: 'web', hint: 'Creates a token scoped strictly to deploy:<service>.' },
            ],
            submit: 'Create',
          }, (o) => {
            const custom = (o.svc || '').trim();
            const scope = custom ? `deploy:${custom}` : o.c;
            return api.createToken(o.n.trim(), scope);
          });
          if (out) {
            await confirmDialog({ title: 'Copy your token now', body: `${out.secret} — it is shown only once.`, confirm: 'I have copied it' });
            await reload();
          }
        },
        revoke: async (el) => { if (await confirmDialog({ title: 'Revoke this token?', body: 'Anything using it stops working.', confirm: 'Revoke', danger: true })) { await api.revokeToken(el.dataset.id); await reload(); } },
        grow: async (el) => {
          const v = (disk.volumes || []).find((x) => x.mount === el.dataset.mount);
          if (!v || growing) return;
          const needsTool = (v.steps || []).includes('install_growpart');
          let finished;
          // The dialog closes once the server accepted the request; the task dock then shows each step.
          const out = await askForm({
            title: `Grow ${v.mount} by ${bytes(v.growable_bytes)}?`,
            body: `opsbox grows the partition and then the filesystem while your apps keep running. Enter your password and type grow to confirm.${needsTool ? ' The growpart tool (package cloud-guest-utils) is missing and will be installed first.' : ''}`,
            fields: [
              { name: 'password', label: 'Account password', type: 'password' },
              { name: 'confirm', label: 'Type grow to confirm', placeholder: 'grow' },
              ...(needsTool ? [{ name: 'install_tools', label: 'Install growpart if it is missing', type: 'checkbox', value: true }] : []),
            ],
            submit: 'Grow filesystem',
          }, (o) => new Promise((resolve, reject) => {
            finished = api.growFs({ mount: v.mount, password: o.password, confirm: o.confirm.trim(), installTools: !!o.install_tools, onStarted: () => resolve(true) });
            finished.catch(reject);
          }));
          if (!out) return;
          growing = v.mount; paint();
          try {
            const r = await finished;
            const from = r?.before?.fs_bytes, to = r?.after?.fs_bytes;
            toast(from && to ? `Filesystem grown: ${bytes(from)} → ${bytes(to)}.` : 'Filesystem grown.');
          } catch (e) { toast(e.message, 'err'); }
          growing = '';
          await reload();
        },
        ghConnect: async () => {
          ghBusy = true; ghErr = '';
          const org = (root.querySelector('#gh-org')?.value || ghOrg || '').trim();
          paint();
          try {
            const res = await api.startGithubManifest(org);
            if (res?.demoConnected) { ghBusy = false; await reload(); return; }
            const form = document.createElement('form');
            form.method = 'POST';
            form.action = res.actionUrl;
            const inp = document.createElement('input');
            inp.type = 'hidden';
            inp.name = 'manifest';
            inp.value = res.manifest;
            form.append(inp);
            document.body.append(form);
            form.submit();
          } catch (e) { ghErr = e.message; ghBusy = false; paint(); }
        },
        ghRefresh: async () => {
          ghBusy = true; ghErr = ''; paint();
          try { d.github = await api.getGithubStatus(true); toast('GitHub status refreshed.'); }
          catch (e) { ghErr = e.message; }
          ghBusy = false; paint();
        },
        ghSyncWebhook: async () => {
          ghBusy = true; ghErr = ''; paint();
          try { d.github = await api.syncGithubWebhook(); toast('GitHub push webhook enabled.'); }
          catch (e) { ghErr = e.message; }
          ghBusy = false; paint();
        },
        ghDisconnect: async () => {
          if (!await confirmDialog({ title: 'Disconnect GitHub?', body: 'Existing services keep running but new repository syncing and GitHub App webhooks stop.', confirm: 'Disconnect', danger: true })) return;
          ghBusy = true; ghErr = ''; paint();
          try { await api.disconnectGithub(); toast('GitHub disconnected.'); await reload(); }
          catch (e) { ghErr = e.message; }
          ghBusy = false; paint();
        },
        ghRmKey: async (el) => {
          if (!await confirmDialog({ title: 'Remove this deploy key?', body: 'Future deploys for this repository using this key will fail until a new deploy key is added.', confirm: 'Remove', danger: true })) return;
          try { await api.deleteDeployKey(el.dataset.id); toast('Deploy key removed.'); await reload(); }
          catch (e) { ghErr = e.message; paint(); }
        },
        check: async () => { try { const u = await api.checkOpsboxUpdate(); d.opsboxUpdate = u; paint(); toast(checkToast(u), u.reason === 'network_error' || u.reason === 'rate_limited' ? 'err' : undefined); } catch (e) { toast(e.message, 'err'); } },
        updateNow: async () => { const u = upd(); if (u.latest && await confirmDialog({ title: `Update to opsbox ${u.latest.version}?`, body: 'The panel will restart briefly. If the health check fails after 2 minutes, it rolls back automatically.', confirm: 'Update now' })) { toast('Downloading and verifying update...'); try { await api.applyOpsboxUpdate(u.latest.version); toast('Update started. Reconnecting...'); } catch (e) { toast(e.message, 'err'); } } },
        rollback: async () => { const u = upd(); if (u.previousVersion && await confirmDialog({ title: `Roll back to opsbox ${u.previousVersion}?`, body: 'The panel will restart with the previous version.', confirm: 'Roll back', danger: true })) { try { await api.rollbackOpsboxUpdate(u.previousVersion); toast('Rolling back...'); } catch (e) { toast(e.message, 'err'); } } },
        autoup: async (el) => { try { d.opsboxUpdate = await api.setOpsboxAutoUpdate({ auto: el.getAttribute('aria-checked') !== 'true' }); paint(); } catch (e) { toast(e.message, 'err'); } },
        export: async () => {
          let file;
          const out = await askForm({
            title: 'Export configuration',
            body: 'The file lists your services, domains, databases (not their data), backups, alerts, firewall rules and people. Passwords, keys and tokens are left out and listed as "needs re-entry". Type a passphrase to include the ones opsbox can read back, sealed with that passphrase.',
            fields: [
              { name: 'password', label: 'Account password', type: 'password' },
              { name: 'passphrase', label: 'Passphrase for secrets (optional)', type: 'password', hint: 'Leave empty to export without secrets. At least 8 characters. opsbox does not keep it: lose it and the secrets in the file cannot be opened.' },
              { name: 'passphrase2', label: 'Repeat the passphrase', type: 'password' },
            ],
            submit: 'Export',
          }, async (o) => {
            if ((o.passphrase || o.passphrase2) && o.passphrase !== o.passphrase2) { const e = new Error('The two passphrases are not the same.'); e.field = 'passphrase2'; throw e; }
            file = await api.exportConfig({ password: o.password, passphrase: o.passphrase });
          });
          if (!out || !file) return;
          download(file.filename, file.text, 'application/json');
          const n = file.needsReentry.length;
          toast(file.secretsIncluded ? `Exported with secrets sealed by your passphrase. ${n} item${n === 1 ? '' : 's'} still need${n === 1 ? 's' : ''} re-entry.` : `Exported without secrets. ${n} item${n === 1 ? '' : 's'} will need re-entry on a new server.`);
        },
        importPreview: async () => {
          if (!importDoc || importBusy) return;
          importErr = ''; importBusy = true; paint();
          try {
            const out = await askForm({
              title: 'Preview import',
              body: 'Enter your account password to preview what this file will create on this server. Nothing is changed yet.',
              fields: [{ name: 'password', label: 'Account password', type: 'password' }],
              submit: 'Preview',
            }, async (o) => {
              importPreview = await api.importPreview({ password: o.password, passphrase: importPass, document: importDoc });
            });
            if (!out) { importBusy = false; paint(); return; }
          } catch (e) { importErr = e.message; importPreview = null; }
          importBusy = false; paint();
        },
        importApply: async () => {
          if (!importDoc || !importPreview || importBusy) return;
          importErr = '';
          let done;
          const out = await askForm({
            title: 'Apply import?',
            body: 'This creates missing services, templates, compose projects, databases, backup destinations, alert rules and firewall rules from the file. Existing objects are left alone. Services that still need secrets are created but not deployed.',
            fields: [{ name: 'password', label: 'Account password', type: 'password' }],
            submit: 'Apply',
          }, async (o) => {
            importBusy = true; paint();
            done = await api.importConfig({ password: o.password, passphrase: importPass, document: importDoc });
          });
          importBusy = false;
          if (!out || !done) { paint(); return; }
          const created = done.result?.created || {};
          const n = Object.values(created).reduce((a, v) => a + (Array.isArray(v) ? v.length : 0), 0);
          const need = done.result?.needsSecret || [];
          toast(need.length
            ? `Imported ${n} item${n === 1 ? '' : 's'}. Enter secrets for: ${need.join(', ')}, then deploy.`
            : `Imported ${n} item${n === 1 ? '' : 's'}. Review services and deploy when ready.`);
          importPreview = null;
          paint();
        },
        uninstall: async () => {
          if (uninstallBusy || uninstallGone) return;
          let plan;
          try { plan = await api.uninstallPlan({ purge: false }); }
          catch (e) { toast(e.message, 'err'); return; }
          const parts = [];
          if (plan.removed?.length) parts.push(`Removed: ${plan.removed.join('; ')}.`);
          if (plan.kept?.length) parts.push(`Kept: ${plan.kept.join('; ')}.`);
          if (plan.domains?.length) parts.push(`Domains that stop on 80/443: ${plan.domains.join(', ')}.`);
          if (plan.adopted?.willRestore) parts.push(`Adopted web server ${plan.adopted.holder} will be restored to ports 80/443 first.`);
          (plan.warnings || []).forEach((w) => parts.push(w));
          parts.push('Data under /var/lib/opsbox and /etc/opsbox is kept. Enter your password and type uninstall to confirm.');
          const out = await askForm({
            title: 'Uninstall opsbox?',
            body: parts.join(' '),
            fields: [
              { name: 'password', label: 'Account password', type: 'password' },
              { name: 'confirm', label: 'Type uninstall to confirm', placeholder: 'uninstall' },
            ],
            submit: 'Uninstall',
            danger: true,
          }, (o) => api.uninstallExec({ password: o.password, confirm: (o.confirm || '').trim(), purge: false }));
          if (!out) return;
          uninstallBusy = true;
          uninstallGone = true;
          paint();
        },
      });
    },
  };
}
