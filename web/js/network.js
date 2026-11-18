import { html, btn, section, st, confirmDialog, toast } from './ui.js';
import * as api from './api.js';

export const holderName = (h) => h.container ? `${h.process} (container “${h.container.name}”, image ${h.container.image})` : `${h.process} (PID ${h.pid}${h.unit ? `, ${h.unit}` : ''})`;
export function networkBanner(data) {
  const n = data?.network;
  if (!n?.port_conflict) return '';
  return section('Web ports are busy', html`<div role="status"><p>opsbox is running on ${n.listen_http} and ${n.listen_https}. ${n.holders.map((h) => `Port ${h.port}: ${holderName(h)}.`).join(' ')}</p><a class="btn" href="#/settings?section=network">Fix in Settings → Network</a></div>`);
}
export function networkCard(data, busy = false) {
  const n = data?.network;
  return section('Network', html`<div id="network" class="col gap12">
    ${!n ? html`<p role="status">${data?.error || 'Proxy status is unavailable.'}</p>` : html`
      ${st(n.port_conflict ? 'warn' : 'ok', n.port_conflict ? 'Web ports are busy; apps are running on other ports' : 'Web proxy is running')}
      <p class="sm">Serving HTTP at ${n.listen_http} and HTTPS at ${n.listen_https}.</p>
      ${n.port_conflict ? html`<p class="sm muted">opsbox checks the usual ports every 30 seconds and switches back when they are free.</p>
        ${n.conflict_ports.map((port) => html`<div class="col gap12"><p>Port ${port}: ${n.holders.filter((h) => h.port === port).map(holderName).join('; ') || 'The owner could not be identified, or access is restricted.'}</p>
          ${n.holders.filter((h, i, all) => h.port === port && (h.container || h.unit) && all.findIndex((x) => (x.container?.id || x.unit) === (h.container?.id || h.unit)) === i).map((h) => btn(h.container ? `Let opsbox take ports ${n.holders.filter((x) => x.container?.id === h.container.id).map((x) => x.port).filter((p, i, all) => all.indexOf(p) === i).join('/')} — stop ${h.container.name}` : `Stop and disable ${h.unit}`, { act: 'proxyTakeover', data: { port, kind: h.container ? 'container' : 'unit', id: h.container?.id || h.unit, name: h.container?.name || h.unit }, disabled: busy }))}
        </div>`)}
        <div>${btn('Keep it and use other ports', { act: 'proxyKeep', disabled: busy })}</div>` : ''}
      <p class="sm muted">Automatic HTTPS for domains needs ports 80 and 443. Containers and their volumes are always kept.</p>`}
    ${(data?.undo || []).map((r) => html`<div class="list-row"><span>Previous ${r.target.kind === 'container' ? 'container restart policy' : 'service state'}: ${r.previous_policy}</span>${btn('Undo takeover', { act: 'proxyUndo', data: { id: r.id, name: r.target.id }, disabled: busy })}</div>`)}
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
