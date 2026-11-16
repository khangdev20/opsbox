import { html, st, btn, table, section, header, sw, bindActions, confirmDialog, askForm, toast, download } from '../ui.js';
import * as api from '../api.js';

const TZ = ['Asia/Ho_Chi_Minh', 'UTC', 'Australia/Brisbane'];
export async function view() {
  let d = await api.getSettings(), root;
  const body = () => html`
    ${section('General', html`<div style="max-width:520px;display:flex;flex-direction:column;gap:16px">
      <div class="field"><label for="sh">Hostname</label><input class="input" id="sh" value="${d.hostname}"></div>
      <div class="field"><label for="sz">Time zone</label><select class="select" id="sz">${TZ.map((z) => html`<option${z === d.tz ? ' selected' : ''}>${z}</option>`)}</select><span class="hint">Used for cron jobs, backups and alert quiet hours.</span></div>
      <div class="field"><label for="sp">Panel address</label><input class="input" id="sp" value="${d.panel}"><span class="hint">Certificate issued automatically. The admin port stays bound to localhost.</span></div>
      <p id="err" role="alert" style="color:var(--danger);font-size:13px"></p><div>${btn('Save', { act: 'save', kind: 'primary' })}</div></div>`)}
    ${section('API tokens', html`${table('API tokens', [['Name'], ['Can do'], ['Last used'], ['', 'r']], d.tokens.map((t) => [html`<span class="mono">${t.name}</span>`, t.can, t.last, btn('Revoke', { act: 'revoke', data: { id: t.id }, kind: 'link' })]))}<div>${btn('Create token', { act: 'token' })}</div>`)}
    ${section('Memory and disk', html`<div style="max-width:520px;display:flex;flex-direction:column;gap:16px"><p class="muted sm">Swap protects small instances from running out of memory. Disk growth needs the volume enlarged at your cloud provider first.</p>
      <div class="field"><label for="sw">Swap file</label><select class="select" id="sw" data-act="swap">${['None', '1 GB', '2 GB', '4 GB'].map((v) => html`<option${v === d.swap ? ' selected' : ''}>${v}</option>`)}</select></div>
      <div class="row">${btn('Grow filesystem to fill disk', { act: 'grow', disabled: d.fsGrown })}<span class="mono sm muted">disk 120 GB · filesystem ${d.fsGrown ? '120' : '100'} GB</span></div></div>`)}
    ${section('GitHub', html`<div class="row spread"><div><b>${d.github ? 'github.com/acme-co' : 'Not connected'}</b><div class="sm muted mono">${d.github ? 'GitHub App · read-only code access · 14 of 31 repositories' : 'Connect to deploy from your repositories'}</div></div>${d.github ? btn('Disconnect', { act: 'gh', kind: 'danger' }) : btn('Connect', { act: 'gh', kind: 'primary' })}</div>`)}
    ${section('opsbox updates', html`<div class="list-row"><span>opsbox 0.1.0 · ${d.channel} channel · up to date</span>${btn('Check now', { act: 'check' })}</div><div class="list-row"><span>Update opsbox automatically</span>${sw('Update opsbox automatically', d.autoUpdate, 'autoup')}</div><p class="muted sm">If a new version fails its health check, the watchdog rolls back to the previous one by itself.</p>`)}
    ${section('Export and uninstall', html`<p class="muted sm" style="max-width:620px">Export saves services and settings as a file you can import on a new server. Uninstall removes opsbox but leaves your apps and data running.</p><div class="row">${btn('Export configuration', { act: 'export' })}${btn('Uninstall opsbox', { act: 'uninstall', kind: 'danger' })}</div>`)}`;
  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  const reload = async () => { d = await api.getSettings(); paint(); };
  return {
    nav: null, title: 'Settings',
    html: html`<main class="narrow">${header('Settings', `${d.hostname} · opsbox 0.1.0`)}<div id="b"></div></main>`,
    mount(r) {
      root = r; paint();
      root.addEventListener('change', async (e) => { if (e.target.id === 'sw') { await api.setSwap(e.target.value); toast('Swap updated.'); } });
      bindActions(root, {
        save: async () => { try { await api.saveSettings({ hostname: root.querySelector('#sh').value, tz: root.querySelector('#sz').value, panel: root.querySelector('#sp').value }); toast('Saved.'); await reload(); } catch (e) { root.querySelector('#err').textContent = e.message; } },
        token: async () => { const out = await askForm({ title: 'Create API token', fields: [{ name: 'n', label: 'Name', placeholder: 'ci-deploy' }, { name: 'c', label: 'Can do', options: ['deploy only', 'read-only metrics', 'full access'], value: 'deploy only' }], submit: 'Create' }, (o) => api.createToken(o.n.trim(), o.c)); if (out) { await confirmDialog({ title: 'Copy your token now', body: `${out.secret} — it is shown only once.`, confirm: 'I have copied it' }); await reload(); } },
        revoke: async (el) => { if (await confirmDialog({ title: 'Revoke this token?', body: 'Anything using it stops working.', confirm: 'Revoke', danger: true })) { await api.revokeToken(el.dataset.id); await reload(); } },
        grow: async () => { await api.growFs(); toast('Filesystem grown.'); await reload(); },
        gh: async () => { if (d.github) { if (await confirmDialog({ title: 'Disconnect GitHub?', body: 'Existing services keep running but auto-deploy stops.', confirm: 'Disconnect', danger: true })) { await api.disconnectGithub(); await reload(); } } else toast('GitHub connection opens in the real build.'); },
        check: () => toast('opsbox is up to date.'),
        autoup: async (el) => { await api.setOpsboxAutoUpdate(el.getAttribute('aria-checked') !== 'true'); await reload(); },
        export: async () => { download('opsbox-config.json', await api.exportConfig(), 'application/json'); toast('Exported.'); },
        uninstall: async () => { if (await confirmDialog({ title: 'Uninstall opsbox?', body: 'The panel is removed. Your apps and data keep running.', confirm: 'Uninstall', danger: true, typed: 'uninstall' })) toast('The demo does not uninstall anything.'); },
      });
    },
  };
}
