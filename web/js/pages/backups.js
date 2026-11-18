import { html, st, btn, table, section, header, bindActions, confirmDialog, askForm, formDialog, toast, bytes, gb } from '../ui.js';
import { server } from '../store.js';
import * as api from '../api.js';

const LIVE = api.MODE === 'http';
const sampleBadge = () => (LIVE ? html`<span class="tag-sm sample" style="align-self:flex-start">sample data</span>` : '');

function checkKind(item) {
  if (item.checkState === 'checksum_failed') return 'fail';
  if (item.checkState === 'checksum_ok' || item.checkState === 'restore_tested' || item.check === 'checksum ok' || item.check === 'restore-tested') return 'ok';
  return 'warn';
}

export async function view() {
  let d = await api.getBackups();
  let root;
  const isAdmin = !LIVE || server.role === 'admin';

  const body = () => {
    const ld = d.localDisk;
    const sameList = ld && Array.isArray(ld.sameDiskPaths) && ld.sameDiskPaths.length ? ` (${ld.sameDiskPaths.join(', ')})` : '';
    const diskMeta = ld && ld.availableBytes != null
      ? html`<p class="muted sm">Local repo: <span class="mono">${ld.path}</span> · ${bytes(ld.availableBytes)} free${ld.resticVersion ? ` · restic ${ld.resticVersion}` : ''}</p>`
      : '';
    return html`
      ${ld && ld.resticInstalled === false ? html`<div class="callout warn"><span class="tag">▲</span><span><b>restic</b> is not installed on this server yet (<span class="mono">/usr/local/bin/restic</span>). Run the installer or install restic 0.19.1 to take backups.</span></div>` : ''}
      ${d.sameDisk ? html`<div class="callout"><span class="tag">▲</span><span>Local backups at <b>${(ld && ld.path) || '/var/backups/opsbox'}</b> live on the same disk as your data${sameList}. If the instance is lost, they are lost too. Off-server destinations arrive next.</span><a class="btn primary" href="#/backups/new">Add destination</a></div>` : ''}
      ${diskMeta}
      <div class="cols"><div class="w3">
        ${section('Schedules', table(
          'Backup schedules',
          [['What'], ['Schedule'], ['Keep'], ['Destination'], ['Last run'], ['', 'r']],
          d.jobs.map((j) => [
            html`<b>${j.what}</b><div class="sm muted">${j.note}${j.enabled === false ? ' · disabled' : ''}</div>${j.lastError ? html`<div class="sm" style="color:var(--danger)">${j.lastError}</div>` : ''}`,
            html`<div>${j.cronHuman || j.cron}</div><div class="mono sm muted">${j.cron}</div>`,
            html`<span class="mono">${j.keep}</span>`,
            html`<span class="mono">${j.dest}</span>`,
            st(j.state, j.last),
            isAdmin
              ? html`<span class="row" style="justify-content:flex-end">${btn('Run now', { act: 'run', data: { id: j.id }, kind: 'link' })}${btn('Edit', { act: 'editjob', data: { id: j.id }, kind: 'link' })}${j.id !== 'opsbox-self' ? btn('Delete', { act: 'deljob', data: { id: j.id }, kind: 'link' }) : ''}</span>`
              : '',
          ]),
        ), isAdmin ? btn('+ Add schedule', { act: 'addjob' }) : '')}
        ${section('Available backups', table(
          'Backups',
          [['Taken (UTC)'], ['What'], ['Size', 'r'], ['Where'], ['Check'], ['', 'r']],
          d.items.length
            ? d.items.map((k) => [
              html`<span class="mono">${k.at}</span>`,
              html`<span class="mono">${k.what}</span>`,
              k.sizeBytes != null ? bytes(k.sizeBytes) : gb(k.size),
              html`<span class="mono">${k.where}</span>`,
              st(checkKind(k), k.check),
              html`<span class="row" style="justify-content:flex-end">${btn('Restore', { act: 'restore', data: { id: k.id }, kind: 'link' })}</span>`,
            ])
            : [[html`<span class="muted">No snapshots yet — click <b>Back up now</b> to create the first encrypted snapshot.</span>`, '', '', '', '', '']],
        ), isAdmin ? html`<span class="row">${btn('Prune old', { act: 'prune', kind: 'link' })}${btn('Verify checksums', { act: 'check', kind: 'link' })}</span>` : '')}
      </div><div>
        ${section('Destinations', table('Destinations', [['Name'], ['Location'], ['Status']], d.dests.map((x) => [x.name, html`<span class="mono sm">${x.loc}</span>`, x.ok ? st('ok', 'ready') : st('warn', x.status === 'restic_missing' ? 'restic missing' : 'local only')])), html`<span class="row" style="align-items:center">${sampleBadge()}<a class="btn link" href="#/backups/new">+ Add</a></span>`)}
        ${section('Restore, safely', html`${sampleBadge()}<p class="muted sm">Restoring never overwrites live data by default. opsbox restores into a copy next to the original; you check it, then swap it in or delete it.</p>`)}
        ${section('Encryption', html`${sampleBadge()}<p class="muted sm">Local snapshots are encrypted with a key derived from <span class="mono">/etc/opsbox/master.key</span>. Recovery key export arrives with offsite destinations.</p><div class="row">${btn('Download recovery key', { act: 'key' })}</div>`)}
      </div></div>
    `;
  };

  const paint = () => { root.querySelector('#b').innerHTML = body().toString(); };
  const reload = async () => { d = await api.getBackups(); paint(); };

  return {
    nav: 'backups',
    title: 'Backups',
    html: html`<main>${header('Backups', 'Scheduled, encrypted restic snapshots (UTC)', isAdmin ? html`<span class="row">${btn('Verify checksums', { act: 'check' })}${btn('+ Add schedule', { act: 'addjob' })}${btn('Back up now', { act: 'run', data: { id: '' }, kind: 'primary' })}</span>` : '')}<div id="b"></div></main>`,
    mount(r) {
      root = r;
      paint();
      bindActions(root, {
        run: async (el) => {
          toast('Backup started…');
          await api.runBackup(el.dataset.id);
          toast('Backup finished.');
          await reload();
        },
        prune: async () => {
          if (!await confirmDialog({ title: 'Prune old snapshots?', body: 'Applies each schedule’s daily/weekly retention limits and removes expired data blocks via restic forget --prune.', confirm: 'Prune' })) return;
          toast('Pruning expired snapshots…');
          const t = await api.pruneBackups({});
          toast(`Pruned ${(t && t.result && t.result.removed) || 0} snapshot(s).`);
          await reload();
        },
        check: async () => {
          toast('Verifying repository checksums (10% data subset)…');
          await api.checkBackups({ readDataSubset: '10%' });
          toast('Checksum check passed.');
          await reload();
        },
        addjob: async () => {
          const created = await askForm({
            title: 'Add backup schedule',
            body: 'Back up a host folder or Docker volume into the encrypted local repository on a UTC cron schedule.',
            fields: [
              { name: 'name', label: 'Name', placeholder: 'uploads', hint: 'Short label for this backup job' },
              { name: 'kind', label: 'Target type', options: ['path', 'volume'], value: 'path' },
              { name: 'targets', label: 'Paths or Docker volumes', placeholder: '/var/www/uploads', hint: 'Absolute paths (or volume names) separated by commas' },
              { name: 'cron', label: 'Cron schedule (UTC)', value: '0 3 * * *', hint: '5 fields (min hour day month weekday) or @daily / @hourly' },
              { name: 'retainDaily', label: 'Keep daily snapshots', type: 'number', value: '14' },
              { name: 'retainWeekly', label: 'Keep weekly snapshots', type: 'number', value: '8' },
              { name: 'pauseContainers', label: 'Pause containers during volume snapshot', options: ['no', 'yes'], value: 'no', hint: 'Only applies to volume jobs' },
            ],
            submit: 'Create schedule',
          }, async (o) => {
            const list = String(o.targets || '').split(',').map((s) => s.trim()).filter(Boolean);
            const payload = {
              name: o.name,
              kind: o.kind,
              cron: o.cron,
              retainDaily: parseInt(o.retainDaily, 10) || 14,
              retainWeekly: parseInt(o.retainWeekly, 10) || 8,
              ...(o.kind === 'volume'
                ? { volumes: list, pauseContainers: o.pauseContainers === 'yes' }
                : { paths: list }),
            };
            await api.createBackupJob(payload);
            return true;
          });
          if (created) { toast('Backup schedule created.'); await reload(); }
        },
        editjob: async (el) => {
          const j = d.jobs.find((x) => x.id === el.dataset.id);
          if (!j) return;
          const fields = [
            { name: 'name', label: 'Name', value: j.name },
            { name: 'cron', label: 'Cron schedule (UTC)', value: j.cron },
            { name: 'retainDaily', label: 'Keep daily snapshots', type: 'number', value: String(j.retainDaily ?? 14) },
            { name: 'retainWeekly', label: 'Keep weekly snapshots', type: 'number', value: String(j.retainWeekly ?? 8) },
            { name: 'enabled', label: 'Enabled', options: ['yes', 'no'], value: j.enabled === false ? 'no' : 'yes' },
          ];
          if (j.kind === 'path' && j.whatDetail && Array.isArray(j.whatDetail.paths)) {
            fields.push({ name: 'targets', label: 'Paths', value: j.whatDetail.paths.join(', ') });
          } else if (j.kind === 'volume' && j.whatDetail && Array.isArray(j.whatDetail.volumes)) {
            fields.push({ name: 'targets', label: 'Volumes', value: j.whatDetail.volumes.join(', ') });
          }
          const ok = await askForm({ title: `Edit "${j.name}"`, fields, submit: 'Save' }, async (o) => {
            const patch = {
              name: o.name,
              cron: o.cron,
              retainDaily: parseInt(o.retainDaily, 10),
              retainWeekly: parseInt(o.retainWeekly, 10),
              enabled: o.enabled === 'yes',
            };
            if (o.targets !== undefined) {
              const list = String(o.targets || '').split(',').map((s) => s.trim()).filter(Boolean);
              if (j.kind === 'path') patch.paths = list;
              if (j.kind === 'volume') patch.volumes = list;
            }
            await api.updateBackupJob(j.id, patch);
            return true;
          });
          if (ok) { toast('Schedule updated.'); await reload(); }
        },
        deljob: async (el) => {
          const j = d.jobs.find((x) => x.id === el.dataset.id);
          if (!j) return;
          if (!await confirmDialog({ title: `Delete "${j.name}"?`, body: 'Existing encrypted snapshots stay in the repository until pruned.', confirm: 'Delete', danger: true })) return;
          await api.deleteBackupJob(j.id);
          toast('Backup schedule removed.');
          await reload();
        },
        restore: async (el) => {
          if (LIVE && !d.restoreReady) {
            toast('Restore copy/replace and snapshot download arrive in the next update.');
            return;
          }
          const o = await formDialog({ title: 'Restore this backup', body: 'Restoring to a copy is safe: nothing live changes.', fields: [{ name: 'mode', label: 'Restore as', options: ['copy', 'replace'], value: 'copy', hint: 'replace overwrites the live data' }], submit: 'Restore' });
          if (!o) return;
          if (o.mode === 'replace' && !await confirmDialog({ title: 'Replace live data?', body: 'The current data is overwritten. This cannot be undone.', confirm: 'Replace', danger: true, typed: 'replace' })) return;
          const r = await api.restoreBackup(el.dataset.id, o.mode);
          toast(r.msg);
        },
        key: () => toast('Recovery key export arrives with offsite destinations.'),
      });
    },
  };
}
