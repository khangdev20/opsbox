import { html, esc, ago, st, btn, tabs, table, section, header, bindActions, confirmDialog, toast, gb, num, patch, empty, refreshing, announce, openModal } from '../ui.js';
import * as api from '../api.js';
import { listeners } from '../store.js';

const KINDS = ['containers', 'images', 'volumes', 'networks'];
const MAX_LOG_LINES = 800;

function formatLogTime(ts) {
  if (!ts) return '';
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return String(ts).slice(0, 19);
  return d.toTimeString().slice(0, 8);
}

function renderLogLines(lines) {
  if (!lines.length) return html`<span class="d">No log lines yet.</span>`;
  return lines.map((line) => {
    const t = typeof line === 'string' ? '' : formatLogTime(line.ts);
    const txt = typeof line === 'string' ? line : line.text;
    const isErr = typeof line !== 'string' && line.stream === 'stderr';
    return html`${t ? html`<span class="d">${t}</span> ` : ''}<span class="${isErr ? 'e' : ''}">${txt}</span>\n`;
  });
}

function showInspectDialog(info) {
  const d = document.createElement('dialog');
  d.style.maxWidth = '760px';
  const row = (k, v) => `<tr><th style="width:160px">${esc(k)}</th><td style="height:36px">${v}</td></tr>`;
  const ports = (info.ports || []).map((p) => `${p.ip ? `${p.ip}:` : ''}${p.public_port ?? ''}→${p.private_port}/${p.protocol}`).join(', ') || '–';
  const mounts = (info.mounts || []).map((m) => `<div class="mono sm">${esc(m.source)} → ${esc(m.destination)} (${esc(m.mode || (m.rw ? 'rw' : 'ro'))})</div>`).join('') || '<span class="muted">None</span>';
  const nets = (info.networks || []).map((n) => `<div class="mono sm">${esc(n.name)}: ${esc(n.ip_address || '–')}</div>`).join('') || '<span class="muted">None</span>';
  const env = (info.env || []).map((e) => `<tr><td class="mono sm" style="height:32px">${esc(e.key)}</td><td class="mono sm" style="height:32px;word-break:break-all">${e.masked ? '<span class="muted">[masked]</span>' : esc(e.value)}</td></tr>`).join('') || '<tr><td class="muted" style="height:32px">None</td><td></td></tr>';
  const labels = Object.entries(info.labels || {}).map(([k, v]) => `<div class="mono sm">${esc(k)}=${esc(v)}</div>`).join('') || '<span class="muted">None</span>';

  d.innerHTML = `<form method="dialog"><div class="d-b" style="max-height:78vh;overflow:auto">
    <h2>Container ${esc(info.name)}</h2>
    <div class="tw"><table><tbody>
      ${row('ID', `<span class="mono sm">${esc(info.fullId || info.id)}</span>`)}
      ${row('Image', `<span class="mono sm">${esc(info.image)}</span>`)}
      ${row('Ownership', info.managed ? `Managed by opsbox${info.service ? ` (${esc(info.service)})` : ''}` : 'Unmanaged (external container)')}
      ${row('State', `${esc(info.state)} · exit ${esc(info.exitCode)} · restarts ${esc(info.restartCount)}${info.pid ? ` · PID ${esc(info.pid)}` : ''}${info.oomKilled ? ' · OOM killed' : ''}`)}
      ${row('Command', `<span class="mono sm">${esc((info.cmd || []).join(' ') || '–')}</span>`)}
      ${row('Working dir', `<span class="mono sm">${esc(info.workingDir || '/')}</span>`)}
      ${row('User', `<span class="mono sm">${esc(info.user || 'default')}</span>`)}
      ${row('Ports', `<span class="mono sm">${esc(ports)}</span>`)}
      ${row('Mounts', mounts)}
      ${row('Networks', nets)}
      ${row('Labels', labels)}
    </tbody></table></div>
    <div><h3 class="sm" style="margin:8px 0">Environment</h3><div class="tw"><table><tbody>${env}</tbody></table></div></div>
  </div><div class="d-f"><button class="btn primary" value="close" autofocus>Close</button></div></form>`;
  d.addEventListener('close', () => d.remove());
  openModal(d);
}

function renderStateBadge(c) {
  if (c.state === 'running') {
    const label = c.since > 0 ? `running ${ago(c.since).replace(' ago', '')}` : (c.status || 'running');
    return st('ok', label);
  }
  if (c.state === 'restarting') return st('warn', c.status || 'restarting');
  if (c.state === 'paused') return st('warn', 'paused');
  return st('off', c.state || 'exited');
}

export async function view() {
  let status = await api.dockerStatus();
  let tab = 0;
  let data = status.running ? await api.listContainers() : [];
  let logTarget = null;
  let logLines = [];
  let logFollow = true;
  let logEnded = false;
  let logError = '';
  let stopStream = null;
  let root;
  let noAnim = false; // a tab switch is a different view: nothing enters or flashes
  const seen = new Map(data.map((c) => [c.id, c.state]));
  const put = (el, h) => patch(el, h.toString(), noAnim ? { animate: false } : undefined);

  const stopLogs = () => {
    if (stopStream) {
      stopStream();
      stopStream = null;
    }
  };

  const load = async () => {
    status = await api.dockerStatus();
    if (!status.running) {
      data = [];
      return;
    }
    data = tab === 0 ? await api.listContainers() : await api.listDocker(KINDS[tab]);
  };

  const paintHeader = () => {
    const metaEl = root.querySelector('#hdr-meta');
    if (!metaEl) return;
    if (!status.installed) {
      metaEl.textContent = 'Docker Engine is not installed on this server';
    } else if (!status.running) {
      metaEl.textContent = 'Docker daemon is not running';
    } else {
      const n = tab === 0 ? data.filter((c) => c.state === 'running').length : null;
      const ver = status.version ? `Docker ${status.version}` : 'Docker running';
      metaEl.textContent = n !== null ? `${ver} · ${n} running` : ver;
    }
  };

  const paintTable = () => {
    const el = root.querySelector('#tbl');
    if (!el) return;
    paintHeader();
    if (!status.installed) {
      put(el, section(
        'Docker is not installed',
        html`<p class="muted">Install Docker Engine from ${status.installMethod ? `signed ${status.installMethod} packages` : 'signed system repositories'} to view and manage containers, images, volumes, and networks.</p>
          <div class="row">${btn('Install Docker', { act: 'install', kind: 'primary', disabled: !status.canInstall })}</div>
          ${!status.canInstall ? html`<p class="sm muted">Automatic Docker installation is not supported on this OS distribution; install Docker Engine manually.</p>` : ''}`,
      ));
      return;
    }
    if (!status.running) {
      put(el, section(
        'Docker daemon is not running',
        html`<p class="muted">${status.error || 'Start the Docker service to manage containers.'}</p>
          <div class="row">${btn('Start Docker', { act: 'startdaemon', kind: 'primary' })}<a class="btn" href="#/system">Open System services</a></div>`,
      ));
      return;
    }

    if (tab === 0) {
      if (!data.length) {
        put(el, section('Containers', empty('No containers on this server yet.', html`<a class="btn primary" href="#/apps/new">Deploy a service</a>`)));
        return;
      }
      put(el, table(
        'Containers',
        [['Name'], ['Image'], ['State'], ['CPU', 'r'], ['Mem', 'r'], ['Ports'], ['', 'r']],
        data.map((c) => Object.assign([
          html`<span class="mono">${c.name}</span>${c.managed ? (c.service ? html`<div class="sm muted">opsbox · ${c.service}</div>` : html`<div class="sm muted">opsbox</div>`) : html`<div class="sm muted">unmanaged</div>`}`,
          html`<span class="mono muted">${c.image}</span>`,
          renderStateBadge(c),
          c.state === 'running' ? num(c.cpu ?? 0, 'pct1') : '–',
          c.state === 'running' ? num(c.mem ?? 0, 'mb') : '–',
          c.ports || '–',
          html`<span class="row" style="justify-content:flex-end">
            ${btn('Logs', { act: 'logs', data: { id: c.id, name: c.name }, kind: 'link' })}
            ${btn('Inspect', { act: 'inspect', data: { id: c.id }, kind: 'link' })}
            ${c.state === 'running' || c.state === 'restarting' || c.state === 'paused'
              ? html`${btn('Restart', { act: 'do', data: { id: c.id, name: c.name, a: 'restart' }, kind: 'link' })}${btn('Stop', { act: 'do', data: { id: c.id, name: c.name, a: 'stop' }, kind: 'link' })}`
              : html`${btn('Start', { act: 'do', data: { id: c.id, name: c.name, a: 'start' }, kind: 'link' })}${btn('Delete', { act: 'del', data: { id: c.id, name: c.name }, kind: 'link' })}`}
          </span>`,
        ], { meta: { key: c.id, sig: c.state } })),
      ));
      return;
    }
    if (tab === 1) {
      if (!data.length) { put(el, section('Images', empty('No images present.'))); return; }
      put(el, table('Images', [['Image'], ['Tag'], ['Size', 'r'], ['Used by']], data.map((i) => [
        html`<span class="mono">${i.name}</span>`,
        i.tag,
        i.size != null ? gb(i.size, 2) : '–',
        i.used ? st('ok', 'in use') : st('warn', 'unused'),
      ])));
      return;
    }
    if (tab === 2) {
      if (!data.length) { put(el, section('Volumes', empty('No volumes present.'))); return; }
      put(el, table('Volumes', [['Volume'], ['Size', 'r'], ['Attached']], data.map((v) => [
        html`<span class="mono">${v.name}</span>`,
        v.size != null ? gb(v.size, 2) : '–',
        v.used ? st('ok', 'attached') : st('warn', 'not attached'),
      ])));
      return;
    }
    if (!data.length) { put(el, section('Networks', empty('No networks present.'))); return; }
    put(el, table('Networks', [['Network'], ['Driver']], data.map((n) => [
      html`<span class="mono">${n.name}</span>`,
      n.driver,
    ])));
  };

  const paintLogs = () => {
    const el = root.querySelector('#logpanel');
    if (!el) return;
    if (!logTarget || tab !== 0 || !status.running) {
      put(el, '');
      return;
    }
    const statusText = logError ? `Error: ${logError}` : logEnded ? 'Stream ended' : logFollow ? 'Following live' : 'Paused';
    put(el, `<div style="margin-top:20px">${section(
      `Logs: ${logTarget.name}`,
      html`${logError ? html`<p class="sm" style="color:var(--danger)">${logError}</p>` : ''}
        <pre class="code" id="logpre" aria-label="Container logs">${renderLogLines(logLines)}</pre>`,
      html`<div class="row">
        <span class="sm muted">${statusText}</span>
        ${btn(logFollow ? 'Pause follow' : 'Follow', { act: 'togglefollow', kind: 'link' })}
        ${btn('Close logs', { act: 'closelogs', kind: 'link' })}
      </div>`,
    )}</div>`);
    const pre = el.querySelector('#logpre');
    if (pre && logFollow) pre.scrollTop = pre.scrollHeight;
  };

  const openLogs = async (id, name) => {
    stopLogs();
    logTarget = { id, name };
    logLines = [];
    logFollow = true;
    logEnded = false;
    logError = '';
    paintLogs();

    if (api.MODE === 'http') {
      stopStream = api.streamContainerLogs(
        id,
        { tail: 200 },
        (entries) => {
          logLines.push(...entries);
          if (logLines.length > MAX_LOG_LINES) logLines.splice(0, logLines.length - MAX_LOG_LINES);
          const pre = root.querySelector('#logpre');
          if (pre) {
            pre.innerHTML = renderLogLines(logLines).toString();
            if (logFollow) pre.scrollTop = pre.scrollHeight;
          } else {
            paintLogs();
          }
        },
        () => { logEnded = true; paintLogs(); },
        (msg) => { logError = msg; paintLogs(); },
      );
    } else {
      logLines = await api.containerLogs(id, 200);
      paintLogs();
    }
  };

  const paintAll = () => {
    const tEl = root.querySelector('#t');
    if (tEl) patch(tEl, status.running ? tabs(['Containers', 'Images', 'Volumes', 'Networks'], tab).toString() : '');
    paintTable();
    paintLogs();
  };

  return {
    nav: 'containers',
    title: 'Containers',
    html: html`<main>
      <div class="hdr">
        <div><h1>Containers</h1><div class="meta" id="hdr-meta"></div></div>
        <div class="row"><a class="btn" href="#/cleanup">Clean up unused</a></div>
      </div>
      <div id="t"></div>
      <div id="b">
        <div id="tbl"></div>
        <div id="logpanel"></div>
      </div>
    </main>`,
    mount(r) {
      root = r;
      paintAll();
      bindActions(root, {
        tab: async (el) => {
          tab = +el.dataset.i;
          stopLogs();
          logTarget = null;
          await refreshing(root.querySelector('#b'), () => load());
          noAnim = true;
          try { paintAll(); } finally { noAnim = false; }
        },
        install: async (el) => {
          if (!await confirmDialog({
            title: 'Install Docker Engine?',
            body: 'Docker Engine will be installed from signed official repositories and started on this server.',
            confirm: 'Install Docker',
          })) return;
          toast('Installing Docker Engine… this may take a few minutes.');
          await api.installDocker();
          toast('Docker Engine installed.');
          await load();
          paintAll();
        },
        startdaemon: async () => {
          await api.unitAction('docker.service', 'start');
          toast('Docker service started.');
          await load();
          paintAll();
        },
        logs: async (el) => {
          await openLogs(el.dataset.id, el.dataset.name || el.dataset.id);
        },
        togglefollow: async () => {
          logFollow = !logFollow;
          if (logFollow && logTarget && !stopStream && api.MODE === 'http') {
            await openLogs(logTarget.id, logTarget.name);
            return;
          }
          if (!logFollow) stopLogs();
          paintLogs();
        },
        closelogs: () => {
          stopLogs();
          logTarget = null;
          logLines = [];
          paintLogs();
        },
        inspect: async (el) => {
          const info = await api.inspectContainer(el.dataset.id);
          showInspectDialog(info);
        },
        do: async (el) => {
          const a = el.dataset.a;
          const name = el.dataset.name || el.dataset.id;
          if (a === 'stop' || a === 'restart') {
            const ok = await confirmDialog({
              title: `${a === 'stop' ? 'Stop' : 'Restart'} ${name}?`,
              body: a === 'stop'
                ? 'Active connections to this container will drop.'
                : 'The container will stop briefly and start again.',
              confirm: a === 'stop' ? 'Stop' : 'Restart',
              danger: a === 'stop',
            });
            if (!ok) return;
          }
          await api.containerAction(el.dataset.id, a);
          toast(`${name}: ${a} done.`);
          await refreshing(root.querySelector('#tbl'), () => load());
          paintTable();
        },
        del: async (el) => {
          const name = el.dataset.name || el.dataset.id;
          if (await confirmDialog({
            title: `Delete ${name}?`,
            body: 'The container is removed. Volumes are kept.',
            confirm: 'Delete',
            danger: true,
          })) {
            await api.containerAction(el.dataset.id, 'delete');
            if (logTarget && logTarget.id === el.dataset.id) {
              stopLogs();
              logTarget = null;
              paintLogs();
            }
            toast('Container deleted.');
            await refreshing(root.querySelector('#tbl'), () => load());
            paintTable();
          }
        },
      });
      this._t = async () => {
        if (!status.running) {
          const next = await api.dockerStatus().catch(() => null);
          if (next && next.running) {
            status = next;
            await load();
            paintAll();
          }
          return;
        }
        if (tab === 0) {
          try {
            data = await api.listContainers();
            for (const c of data) {
              const was = seen.get(c.id);
              if (was && was !== c.state) announce(c.state === 'running' ? `${c.name} is running.` : `${c.name} is ${c.state}.`);
              seen.set(c.id, c.state);
            }
            paintTable();
          } catch { /* ignore transient poll failure */ }
        }
      };
      listeners.add(this._t);
    },
    unmount() {
      stopLogs();
      listeners.delete(this._t);
    },
  };
}
