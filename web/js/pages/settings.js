import { html, st, btn, table, section, header, sw, bindActions, confirmDialog, askForm, toast, download } from '../ui.js';
import * as api from '../api.js';
import { networkCard, networkActions } from '../network.js';

const TZ = ['Asia/Ho_Chi_Minh', 'UTC', 'Australia/Brisbane'];
const SWAP = ['None', '1 GB', '2 GB', '4 GB'];
const LIVE = api.MODE === 'http';
const sampleBadge = () => LIVE ? html`<span class="tag-sm sample" style="align-self:flex-start">sample data</span>` : '';
const normDom = (s) => String(s || '').trim().toLowerCase().replace(/^https:\/\//, '').replace(/\/$/, '');
export async function view({ query = {} } = {}) {
  let ghErr = '', ghOrg = '';
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
  let dns = null, busy = '', panelErr = '', ghBusy = false;
  const upd = () => d.opsboxUpdate || { current: '0.1.0', channel: d.channel || 'stable', auto: !!d.autoUpdate, window: 3, updateAvailable: false, latest: null, previousVersion: null, lastOutcome: null };
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
      return html`<div class="col gap6">${st('fail', 'Certificate error')}${pi.certError ? html`<span class="sm" style="color:var(--danger)">${pi.certError}</span>` : ''}</div>`;
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
    return html`<div class="col gap12">${ghErr ? html`<p role="alert" style="color:var(--danger);font-size:13px">${ghErr}</p>` : ''}${mainBlock}${keysBlock}</div>`;
  };
  const body = () => {
    const u = upd();
    const pi = panelInfo();
    const certLive = !!(pi.activeDomain && (pi.certStatus === 'valid' || pi.certStatus === 'expiring'));
    const curNorm = normDom(d.panel);
    const dnsOk = !!(dns && dns.pointsHere && dns.domain === curNorm);
    const canIssue = canIssueFor(d.panel);
    const notesLink = u.latest?.notesUrl ? html` · <a href="${u.latest.notesUrl}" target="_blank" rel="noopener">release notes</a>` : '';
    const statusText = u.updateAvailable && u.latest
      ? html`opsbox ${u.current} · <b>${u.latest.version} available</b>${notesLink}`
      : html`opsbox ${u.current} · up to date`;
    return html`
    ${section('General', html`<div style="max-width:520px;display:flex;flex-direction:column;gap:16px">
      <div class="field"><label for="sh">Hostname</label><input class="input" id="sh" value="${d.hostname}"></div>
      <div class="field"><label for="sz">Time zone</label><select class="select" id="sz">${(d.timezones || TZ).map((z) => html`<option${z === d.tz ? ' selected' : ''}>${z}</option>`)}</select><span class="hint">Used for cron jobs, backups and alert quiet hours.</span></div>
      <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p><div>${btn('Save', { act: 'save', kind: 'primary' })}</div></div>`)}
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
      ${panelErr ? html`<p role="alert" style="color:var(--danger);font-size:13px">${panelErr}</p>` : ''}
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
    ${section('Memory and disk', html`<div style="max-width:520px;display:flex;flex-direction:column;gap:16px"><p class="muted sm">Swap protects small instances from running out of memory. Disk growth needs the volume enlarged at your cloud provider first.</p>
      <div class="field"><label for="sw">Swap file</label><select class="select" id="sw" data-act="swap">${(SWAP.includes(d.swap) ? SWAP : [...SWAP, d.swap]).map((v) => html`<option${v === d.swap ? ' selected' : ''}>${v}</option>`)}</select></div>
      <div class="row">${sampleBadge()}${btn('Grow filesystem to fill disk', { act: 'grow', disabled: LIVE || d.fsGrown })}<span class="mono sm muted">disk 120 GB · filesystem ${d.fsGrown ? '120' : '100'} GB</span></div></div>`)}
    ${section('GitHub', renderGithubSection(pi))}
    ${section('opsbox updates', html`<div class="list-row"><span>${statusText}</span><div class="row">${btn('Check now', { act: 'check' })}${u.updateAvailable ? btn('Update now', { act: 'updateNow', kind: 'primary' }) : ''}${u.previousVersion ? btn(`Roll back to ${u.previousVersion}`, { act: 'rollback' }) : ''}</div></div>${u.lastOutcome ? html`<p class="sm muted">${u.lastOutcome.message}</p>` : ''}<div class="list-row"><span>Release channel</span><select class="select" id="uch" style="width:auto"><option value="stable"${u.channel === 'stable' ? ' selected' : ''}>stable</option><option value="beta"${u.channel === 'beta' ? ' selected' : ''}>beta</option></select></div><div class="list-row"><span>Update opsbox automatically</span><div class="row">${sw('Update opsbox automatically', u.auto, 'autoup')}<select class="select" id="uwin" aria-label="Daily update window" style="width:auto">${hours.map((h) => html`<option value="${h}"${h === (u.window ?? 3) ? ' selected' : ''}>${String(h).padStart(2, '0')}:00 UTC</option>`)}</select></div></div><p class="muted sm">If a new version fails its health check after 2 minutes, opsbox rolls back to the previous one by itself.</p>`)}
    ${section('Export and uninstall', html`${sampleBadge()}<p class="muted sm" style="max-width:620px">Export saves services and settings as a file you can import on a new server. Uninstall removes opsbox but leaves your apps and data running.</p><div class="row">${btn('Export configuration', { act: 'export', disabled: LIVE })}${btn('Uninstall opsbox', { act: 'uninstall', kind: 'danger', disabled: LIVE })}</div>`)}`;
  };
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  const refreshNetwork = async () => {
    proxyNetwork = await api.getProxyNetwork().catch((e) => ({ error: e.message }));
    const card = root?.querySelector('#network-card');
    if (card) card.innerHTML = networkCard(proxyNetwork, proxyBusy).toString();
  };
  const reload = async () => { d = await api.getSettings(); await refreshNetwork(); paint(); };
  return {
    unmount() { clearInterval(this._networkTimer); root = null; },
    nav: null, title: 'Settings',
    html: html`<main class="narrow">${header('Settings', `${d.hostname} · opsbox 0.1.0`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      if (query.section === 'network') root.querySelector('#network')?.scrollIntoView();
      this._networkTimer = setInterval(() => { if (!proxyBusy) refreshNetwork(); }, 5000);
      root.addEventListener('input', (e) => {
        if (e.target.id === 'sp') {
          d.panel = e.target.value;
          const b = root.querySelector('[data-act="issueCert"]');
          if (b) b.disabled = !canIssueFor(d.panel);
        }
        if (e.target.id === 'gh-org') ghOrg = e.target.value;
      });
      root.addEventListener('change', async (e) => {
        if (e.target.id === 'sw') { toast('Changing swap. This can take a minute.'); try { await api.setSwap(e.target.value); toast('Swap updated.'); } catch (err) { toast(err.message, 'err'); } await reload(); }
        if (e.target.id === 'uch') { try { d.opsboxUpdate = await api.setOpsboxAutoUpdate({ channel: e.target.value }); toast(`Channel set to ${e.target.value}.`); } catch (err) { toast(err.message, 'err'); } paint(); }
        if (e.target.id === 'uwin') { try { d.opsboxUpdate = await api.setOpsboxAutoUpdate({ window: Number(e.target.value) }); toast('Update window saved.'); } catch (err) { toast(err.message, 'err'); } paint(); }
      });
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
        grow: async () => { await api.growFs(); toast('Filesystem grown.'); await reload(); },
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
        check: async () => { try { const u = await api.checkOpsboxUpdate(); d.opsboxUpdate = u; paint(); toast(u.updateAvailable && u.latest ? `opsbox ${u.latest.version} is available.` : 'opsbox is up to date.'); } catch (e) { toast(e.message, 'err'); } },
        updateNow: async () => { const u = upd(); if (u.latest && await confirmDialog({ title: `Update to opsbox ${u.latest.version}?`, body: 'The panel will restart briefly. If the health check fails after 2 minutes, it rolls back automatically.', confirm: 'Update now' })) { toast('Downloading and verifying update...'); try { await api.applyOpsboxUpdate(u.latest.version); toast('Update started. Reconnecting...'); } catch (e) { toast(e.message, 'err'); } } },
        rollback: async () => { const u = upd(); if (u.previousVersion && await confirmDialog({ title: `Roll back to opsbox ${u.previousVersion}?`, body: 'The panel will restart with the previous version.', confirm: 'Roll back', danger: true })) { try { await api.rollbackOpsboxUpdate(u.previousVersion); toast('Rolling back...'); } catch (e) { toast(e.message, 'err'); } } },
        autoup: async (el) => { try { d.opsboxUpdate = await api.setOpsboxAutoUpdate({ auto: el.getAttribute('aria-checked') !== 'true' }); paint(); } catch (e) { toast(e.message, 'err'); } },
        export: async () => { download('opsbox-config.json', await api.exportConfig(), 'application/json'); toast('Exported.'); },
        uninstall: async () => { if (await confirmDialog({ title: 'Uninstall opsbox?', body: 'The panel is removed. Your apps and data keep running.', confirm: 'Uninstall', danger: true, typed: 'uninstall' })) toast('The demo does not uninstall anything.'); },
      });
    },
  };
}
