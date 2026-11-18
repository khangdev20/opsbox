import { html, btn, section, st, confirmDialog, toast, openModal } from './ui.js';
import * as api from './api.js';

export const holderName = (h) => h.container ? `${h.process} (container “${h.container.name}”, image ${h.container.image})` : `${h.process} (PID ${h.pid}${h.unit ? `, ${h.unit}` : ''})`;
export function networkBanner(data) {
  const n = data?.network;
  if (!n?.port_conflict) return '';
  return section('Web ports are busy', html`<div role="status"><p>opsbox is running on ${n.listen_http} and ${n.listen_https}. ${n.holders.map((h) => `Port ${h.port}: ${holderName(h)}.`).join(' ')}</p>${n.holders.filter((h, i, all) => (h.container || h.unit) && all.findIndex((x) => (x.container?.id || x.unit) === (h.container?.id || h.unit)) === i).map((h) => btn('Move behind opsbox · recommended', { act: 'webMove', kind: 'primary', data: { kind: h.container ? 'container' : 'unit', id: h.container?.id || h.unit } }))}<a class="btn" href="#/settings?section=network">Let opsbox take ports 80/443</a>${btn('Keep it and use other ports', { act: 'proxyKeep' })}<a class="btn link" href="#/settings?section=network">Fix in Settings → Network</a></div>`);
}
export function networkCard(data, busy = false) {
  const n = data?.network;
  return section('Network', html`<div id="network" class="col gap12">
    ${!n ? html`<p role="status">${data?.error || 'Proxy status is unavailable.'}</p>` : html`
      ${st(n.port_conflict ? 'warn' : 'ok', n.port_conflict ? 'Web ports are busy; apps are running on other ports' : 'Web proxy is running')}
      <p class="sm">Serving HTTP at ${n.listen_http} and HTTPS at ${n.listen_https}.</p>
      ${n.port_conflict ? html`${n.holders.filter((h, i, all) => (h.container || h.unit) && all.findIndex((x) => (x.container?.id || x.unit) === (h.container?.id || h.unit)) === i).map((h) => btn('Move behind opsbox · recommended', { act: 'webMove', kind: 'primary', data: { kind: h.container ? 'container' : 'unit', id: h.container?.id || h.unit }, disabled: busy }))}<p class="sm muted">opsbox checks the usual ports every 30 seconds and switches back when they are free.</p>
        ${n.conflict_ports.map((port) => html`<div class="col gap12"><p>Port ${port}: ${n.holders.filter((h) => h.port === port).map(holderName).join('; ') || 'The owner could not be identified, or access is restricted.'}</p>
          ${n.holders.filter((h, i, all) => h.port === port && (h.container || h.unit) && all.findIndex((x) => (x.container?.id || x.unit) === (h.container?.id || h.unit)) === i).map((h) => btn(h.container ? `Let opsbox take ports ${n.holders.filter((x) => x.container?.id === h.container.id).map((x) => x.port).filter((p, i, all) => all.indexOf(p) === i).join('/')} — stop ${h.container.name}` : `Stop and disable ${h.unit}`, { act: 'proxyTakeover', data: { port, kind: h.container ? 'container' : 'unit', id: h.container?.id || h.unit, name: h.container?.name || h.unit }, disabled: busy }))}
        </div>`)}
        <div>${btn('Keep it and use other ports', { act: 'proxyKeep', disabled: busy })}</div>` : ''}
      <p class="sm muted">Automatic HTTPS for domains needs ports 80 and 443. Containers and their volumes are always kept.</p>`}
    ${(data?.undo || []).map((r) => html`<div class="list-row"><span>Previous ${r.target.kind === 'container' ? 'container restart policy' : 'service state'}: ${r.previous_policy}</span>${btn('Undo takeover', { act: 'proxyUndo', data: { id: r.id, name: r.target.id }, disabled: busy })}</div>`)}
    ${(data?.moves || []).filter((r) => r.active).map((r) => html`<div class="list-row"><span>${r.holder} moved to private ports ${r.http_port}/${r.https_port}</span>${btn('Undo move', { act: 'webUndo', data: { id: r.id }, disabled: busy })}</div>`)}
    ${adoptedDomains(data?.adopted?.domains || [])}
    ${btn('Refresh network status', { act: 'proxyRefresh', disabled: busy })}
  </div>`);
}
export function networkActions(refresh, setBusy) {
  const perform = async (fn, message) => {
    setBusy(true);
    try { await fn(); toast(message); }
    catch (e) { toast(e.message, 'err'); }
    finally { setBusy(false); await refresh(); }
  };
  return {
    proxyRefresh: refresh,
    webMove: async (el) => {
      setBusy(true);
      try { const { plan } = await api.discoverWeb({ kind: el.dataset.kind, id: el.dataset.id }); await moveDialog(plan); }
      catch (e) { toast(e.message, 'err'); }
      finally { setBusy(false); await refresh(); }
    },
    webUndo: async (el) => {
      if (!await confirmDialog({ title: 'Undo move?', body: 'opsbox will release ports 80/443 and restore the old server. opsbox apps will use fallback ports.', confirm: 'Undo move' })) return;
      await perform(() => api.undoWebMove(el.dataset.id), 'Original web server restored.');
    },
    adoptedTls: async (el) => {
      if (!await confirmDialog({ title: 'Let opsbox handle TLS?', body: `The existing valid certificate for ${el.dataset.domain} is imported first, or a new certificate is issued. Traffic stays on passthrough until the certificate is ready.`, confirm: 'Let opsbox handle TLS' })) return;
      await perform(() => api.adoptedTls(el.dataset.domain), 'opsbox now handles TLS for this domain.');
    },
    proxyTakeover: async (el) => {
      const { port, kind, id, name } = el.dataset;
      if (!await confirmDialog({ title: kind === 'container' ? `Stop container ${name}?` : `Stop and disable ${name}?`, body: kind === 'container' ? `Sites in “${name}” will stop. Its restart policy will become “no”. Its data and volumes stay in place. You can Undo this change here.` : `Sites served by ${name} will stop and the service will be disabled. You can Undo this change here.`, confirm: 'Let opsbox take the port', danger: true })) return;
      await perform(() => api.takeProxyPort(Number(port), { kind, id }, true), 'Previous server stopped. opsbox will take the port within 30 seconds.');
    },
    proxyUndo: async (el) => {
      if (!await confirmDialog({ title: 'Undo port takeover?', body: `opsbox will move to other ports, then restore and start ${el.dataset.name}.`, confirm: 'Undo takeover' })) return;
      await perform(() => api.undoProxyTakeover(el.dataset.id, true), 'Previous server restored. opsbox is using other ports.');
    },
    proxyKeep: async () => {
      if (!await confirmDialog({ title: 'Keep using these other ports?', body: 'opsbox will save the ports shown above and stop retrying the usual ports. Automatic HTTPS for domains needs ports 80 and 443.', confirm: 'Keep these ports' })) return;
      await perform(() => api.keepProxyPorts(true), 'Ports saved. opsbox will keep using them.');
    },
  };
}

export function adoptedDomains(domains) {
  if (!domains.length) return '';
  return html`<div class="col gap12"><h3>Adopted domains</h3>${domains.map((d) => html`<div class="list-row"><span><b>${d.domain}</b> <span class="tag">${d.terminate_tls ? 'opsbox TLS' : 'passthrough'} · served by ${d.served_by}</span></span>${!d.terminate_tls ? btn('Let opsbox handle TLS', { act: 'adoptedTls', data: { domain: d.domain } }) : ''}</div>`)}</div>`;
}
function moveDialog(plan) {
  return new Promise((resolve) => {
    const d = document.createElement('dialog');
    d.innerHTML = html`<form method="dialog"><div class="d-b"><h2>Move behind opsbox</h2><p>${plan.explanation}</p>
      <p>Keep ${plan.holder} serving your sites on 127.0.0.1:${plan.routes.http_port} and 127.0.0.1:${plan.routes.https_port}. TLS stays with your server. Failed checks automatically restore its public ports.</p>
      ${plan.image ? html`<p>Image: ${plan.image}</p>` : ''}${plan.compose_project ? html`<p>Compose project: ${plan.compose_project} · ${plan.working_dir}</p>` : ''}
      <ul>${plan.routes.domains.map((s) => html`<li>${s.domain} · ${s.served_by} · ${s.certificate_expiry ? `certificate expires ${new Date(Number(s.certificate_expiry) * 1000).toLocaleDateString()}` : 'certificate expiry unknown'} · ${s.reachable_now ? 'answers now' : 'local check unavailable'}</li>`)}</ul>
      ${plan.changes.map((c) => html`<details open><summary>${c.path}</summary><pre class="sm" style="white-space:pre-wrap;overflow-wrap:anywhere">${c.before.split('\n').map((l) => `- ${l}`).join('\n')}
${c.after.split('\n').map((l) => `+ ${l}`).join('\n')}</pre></details>`)}
      <div class="field"><label for="move-domains">Additional domains (one per line)</label><textarea class="input" id="move-domains" name="domains" rows="3"></textarea></div>
      <label class="opt"><input type="checkbox" name="everything_else"${plan.routes.everything_else ? ' checked' : ''}> Forward everything else to the old server</label>
      <div class="field"><label for="move-confirm">Type ${plan.confirmation} to continue</label><input class="input" id="move-confirm" name="confirmation" autocomplete="off"></div>
      <ol data-steps aria-live="polite"><li>Pre-check</li><li>Load adopted routes</li><li>Rebind old server</li><li>Bind opsbox on 80/443</li><li>Verify websites; roll back on failure</li></ol><p class="form-err" role="alert"></p></div>
      <div class="d-f"><button type="button" class="btn" data-close>Close</button><button type="submit" class="btn primary"${plan.supported ? '' : ' disabled'}>${plan.confirmation.startsWith('recreate ') ? 'Recreate with new ports' : 'Move behind opsbox'}</button></div></form>`.toString();
    let running = false;
    const close = d.querySelector('[data-close]'), submit = d.querySelector('[type=submit]');
    close.onclick = () => { if (!running) d.close(); };
    d.addEventListener('cancel', (e) => { if (running) e.preventDefault(); });
    const update = (e) => {
      if (!running || e.detail.kind !== 'web_move') return;
      d.querySelector('[data-steps]').innerHTML = html`${(e.detail.log || []).map((line) => html`<li>${line}</li>`)}`.toString();
    };
    addEventListener('opsbox:task', update);
    d.querySelector('form').onsubmit = async (e) => {
      e.preventDefault(); if (running || !plan.supported) return;
      const form = new FormData(e.target), err = d.querySelector('[role=alert]'); err.textContent = '';
      if (form.get('confirmation') !== plan.confirmation) { err.textContent = `Type ${plan.confirmation} to continue.`; return; }
      running = true; close.disabled = true; submit.disabled = true;
      try {
        await api.moveWeb({ target: plan.target, domains: String(form.get('domains')).split(/[\s,]+/).filter(Boolean), everything_else: form.has('everything_else'), confirmation: form.get('confirmation') });
        toast('Your sites are now behind opsbox.'); d.close();
      } catch (error) { err.textContent = error.message; }
      finally { running = false; close.disabled = false; submit.disabled = false; }
    };
    d.addEventListener('close', () => { removeEventListener('opsbox:task', update); d.remove(); resolve(); });
    openModal(d, d.querySelector('[name=confirmation]'));
  });
}
