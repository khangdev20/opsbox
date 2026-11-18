import { html, st, btn, table, section, header, bindActions, confirmDialog, askForm, formDialog, download, toast, bytes, gb, iconAct } from '../ui.js';
import { server } from '../store.js';
import * as api from '../api.js';

const LIVE = api.MODE === 'http';

function checkKind(item) {
  if (item.checkState === 'checksum_failed') return 'fail';
  if (item.checkState === 'checksum_ok' || item.checkState === 'restore_tested' || item.check === 'checksum ok' || item.check === 'restore-tested') return 'ok';
  return 'warn';
}

export async function view() {
  let d = await api.getBackups();
  let root;
  const isAdmin = !LIVE || server.role === 'admin';

  const readyDests = () => d.dests.filter((x) => x.id === 'local' || x.ok);

  const body = () => {
    const ld = d.localDisk;
    const sameList = ld && Array.isArray(ld.sameDiskPaths) && ld.sameDiskPaths.length ? ` (${ld.sameDiskPaths.join(', ')})` : '';
    const diskMeta = ld && ld.availableBytes != null
      ? html`<p class="muted sm">Local repo: <span class="mono">${ld.path}</span> · ${bytes(ld.availableBytes)} free${ld.resticVersion ? ` · restic ${ld.resticVersion}` : ''}</p>`
      : '';
    const hasLocalJob = d.jobs.some((j) => j.destinationId === 'local' || j.dest === 'local' || j.dest === 'Local disk');
    const rt = d.restoreTest;
    const rtNote = rt && rt.lastRunAt
      ? html`<p class="sm ${rt.lastOk === false ? '' : 'muted'}" style="${rt.lastOk === false ? 'color:var(--danger)' : ''}">${rt.lastSummary || (rt.lastOk ? 'Last monthly restore test passed.' : 'Last monthly restore test failed.')}</p>`
      : '';
    return html`
      ${ld && ld.resticInstalled === false ? html`<div class="callout warn"><span class="tag">▲</span><span><b>restic</b> is not installed on this server yet (<span class="mono">/usr/local/bin/restic</span>). Run the installer or install restic 0.19.1 to take backups.</span></div>` : ''}
      ${d.sameDisk && hasLocalJob ? html`<div class="callout"><span class="tag">▲</span><span>Local backups at <b>${(ld && ld.path) || '/var/backups/opsbox'}</b> live on the same disk as your data${sameList}. If the instance is lost, they are lost too.</span><a class="btn primary" href="#/backups/new">Add offsite destination</a></div>` : ''}
      ${diskMeta}
      <div class="cols"><div class="w3">
        ${section('Schedules', table(
          'Backup schedules',
          [['What'], ['Schedule'], ['Keep'], ['Destination'], ['Last run'], ['', 'r']],
          d.jobs.map((j) => [
            html`<b>${j.what}</b><div class="sm muted">${j.note}${j.enabled === false ? ' · disabled' : ''}</div>${j.lastError ? html`<div class="sm err-t">${j.lastError}</div>` : ''}`,
            html`<div>${j.cronHuman || j.cron}</div><div class="mono sm muted">${j.cron}</div>`,
            html`<span class="mono">${j.keep}</span>`,
            html`<span class="mono">${j.dest}</span>`,
            st(j.state, j.last),
            isAdmin
              ? html`<span class="acts">${iconAct('run', `Run ${j.what} now`, 'start', { id: j.id })}${iconAct('editjob', `Edit ${j.what}`, 'edit', { id: j.id })}${j.id !== 'opsbox-self' ? iconAct('deljob', `Delete ${j.what}`, 'del', { id: j.id }, { danger: true }) : ''}</span>`
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
              isAdmin
                ? html`<span class="acts">${iconAct('testrestore', 'Test restore', 'check', { id: k.id })}${iconAct('restore', 'Restore', 'restore', { id: k.id })}</span>`
                : '',
            ])
            : [[html`<span class="muted">No snapshots yet — click <b>Back up now</b> to create the first encrypted snapshot.</span>`, '', '', '', '', '']],
        ), isAdmin ? html`<span class="row">${btn('Prune old', { act: 'prune', kind: 'link' })}${btn('Verify checksums', { act: 'check', kind: 'link' })}${btn('Run restore test', { act: 'testrestoreall', kind: 'link' })}</span>` : '')}
      </div><div>
        ${section('Destinations', table(
          'Destinations',
          [['Name'], ['Location'], ['Status'], ['', 'r']],
          d.dests.map((x) => [
            html`<b>${x.name}</b>${x.isDefault ? html` <span class="tag-sm">default</span>` : ''}`,
            html`<span class="mono sm">${x.loc}</span>`,
            x.ok
              ? st('ok', 'ready')
              : x.status === 'pending'
                ? st('warn', 'needs key / verify')
                : st('warn', x.status === 'restic_missing' ? 'restic missing' : 'local only'),
            isAdmin
              ? html`<span class="acts">${x.publicKey ? iconAct('showkey', 'Show SSH key', 'key', { id: x.id }) : ''}${x.id !== 'local' ? iconAct('testdest', `Test ${x.name}`, 'check', { id: x.id }) : ''}${!x.isDefault && (x.id === 'local' || x.ok) ? iconAct('defaultdest', `Make ${x.name} the default`, 'star', { id: x.id }) : ''}${x.id !== 'local' ? iconAct('deldest', `Delete ${x.name}`, 'del', { id: x.id }, { danger: true }) : ''}</span>`
              : '',
          ]),
        ), isAdmin ? html`<a class="btn link" href="#/backups/new">+ Add destination</a>` : '')}
        ${section('Restore, safely', html`<p class="muted sm">Restoring never overwrites live data by default. Databases start as a separate copy you can inspect and swap in; files restore to <span class="mono">.restored-&lt;date&gt;</span>. Replacing in place requires typing <span class="mono">replace</span> and takes a live safety snapshot first.</p>${rtNote}`)}
        ${section('Encryption', html`<p class="muted sm">Every repository is encrypted with a key derived from <span class="mono">/etc/opsbox/master.key</span> before leaving this server. Keep the recovery key offline.</p><div class="row">${isAdmin ? btn('Download recovery key', { act: 'key' }) : ''}</div>`)}
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
          const c=await api.checkCapacity({feature:'backup'});
          if(c.fit.status==='blocked') {toast([...(c.fit.reasons||[]),...(c.fit.suggestions||[])].join(' '),'err');return;}
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
          const dests = readyDests();
          const def = dests.find((x) => x.isDefault) || dests[0];
          const fields = [
            { name: 'name', label: 'Name', placeholder: 'uploads', hint: 'Short label for this backup job' },
            { name: 'kind', label: 'Target type', options: ['path', 'volume'], value: 'path' },
            { name: 'targets', label: 'Paths or Docker volumes', placeholder: '/var/www/uploads', hint: 'Absolute paths (or volume names) separated by commas' },
            { name: 'cron', label: 'Cron schedule (UTC)', value: '0 3 * * *', hint: '5 fields (min hour day month weekday) or @daily / @hourly' },
            { name: 'retainDaily', label: 'Keep daily snapshots', type: 'number', value: '14' },
            { name: 'retainWeekly', label: 'Keep weekly snapshots', type: 'number', value: '8' },
            { name: 'pauseContainers', label: 'Pause containers during volume snapshot', options: ['no', 'yes'], value: 'no', hint: 'Only applies to volume jobs' },
          ];
          if (dests.length > 1) {
            fields.push({ name: 'destination', label: 'Destination', options: dests.map((x) => x.name), value: def ? def.name : dests[0].name });
          }
          const created = await askForm({
            title: 'Add backup schedule',
            body: 'Back up a host folder or Docker volume into an encrypted repository on a UTC cron schedule.',
            fields,
            submit: 'Create schedule',
          }, async (o) => {
            const list = String(o.targets || '').split(',').map((s) => s.trim()).filter(Boolean);
            const chosenDest = o.destination ? dests.find((x) => x.name === o.destination) : def;
            const payload = {
              name: o.name,
              kind: o.kind,
              cron: o.cron,
              retainDaily: parseInt(o.retainDaily, 10) || 14,
              retainWeekly: parseInt(o.retainWeekly, 10) || 8,
              destinationId: chosenDest ? chosenDest.id : undefined,
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
          const dests = readyDests();
          const curDest = d.dests.find((x) => x.id === j.destinationId) || dests[0];
          const fields = [
            { name: 'name', label: 'Name', value: j.name },
            { name: 'cron', label: 'Cron schedule (UTC)', value: j.cron },
            { name: 'retainDaily', label: 'Keep daily snapshots', type: 'number', value: String(j.retainDaily ?? 14) },
            { name: 'retainWeekly', label: 'Keep weekly snapshots', type: 'number', value: String(j.retainWeekly ?? 8) },
            { name: 'enabled', label: 'Enabled', options: ['yes', 'no'], value: j.enabled === false ? 'no' : 'yes' },
          ];
          if (dests.length > 1) {
            fields.push({ name: 'destination', label: 'Destination', options: dests.map((x) => x.name), value: curDest ? curDest.name : dests[0].name });
          }
          if (j.kind === 'path' && j.whatDetail && Array.isArray(j.whatDetail.paths)) {
            fields.push({ name: 'targets', label: 'Paths', value: j.whatDetail.paths.join(', ') });
          } else if (j.kind === 'volume' && j.whatDetail && Array.isArray(j.whatDetail.volumes)) {
            fields.push({ name: 'targets', label: 'Volumes', value: j.whatDetail.volumes.join(', ') });
          }
          const ok = await askForm({ title: `Edit "${j.name}"`, fields, submit: 'Save' }, async (o) => {
            const chosenDest = o.destination ? dests.find((x) => x.name === o.destination) : null;
            const patch = {
              name: o.name,
              cron: o.cron,
              retainDaily: parseInt(o.retainDaily, 10),
              retainWeekly: parseInt(o.retainWeekly, 10),
              enabled: o.enabled === 'yes',
              ...(chosenDest ? { destinationId: chosenDest.id } : {}),
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
          const k = d.items.find((x) => x.id === el.dataset.id);
          if (!k) return;
          const isSelf = k.kind === 'opsbox' || k.what === 'opsbox (SQLite + /etc/opsbox)';
          const fields = isSelf
            ? [{ name: 'copyTarget', label: 'Copy folder (optional)', placeholder: '/var/lib/opsbox/opsbox-restored-...', hint: 'opsbox itself always restores to a side-by-side copy' }]
            : [
                { name: 'mode', label: 'Restore as', options: ['copy', 'replace'], value: 'copy', hint: 'copy creates a new database/folder; replace overwrites live data after taking a safety snapshot' },
                { name: 'copyTarget', label: 'Copy name / path (optional)', placeholder: 'Leave blank for default copy name', hint: 'Applies only when restoring to a copy' },
              ];
          const o = await formDialog({
            title: `Restore "${k.what}"`,
            body: `Snapshot ${k.shortId || k.id} from ${k.at}. Restoring to a copy leaves everything live untouched.`,
            fields,
            submit: 'Restore',
          });
          if (!o) return;
          const mode = isSelf ? 'copy' : (o.mode || 'copy');
          if (mode === 'replace') {
            if (!await confirmDialog({
              title: 'Replace live data in place?',
              body: 'opsbox will take a live safety snapshot first, then overwrite the current data. This cannot be skipped.',
              confirm: 'Replace',
              danger: true,
              typed: 'replace',
            })) return;
          }
          toast(mode === 'replace' ? 'Taking safety snapshot and restoring…' : 'Restoring into copy…');
          const r = await api.restoreBackup(k.id, mode, {
            confirm: mode === 'replace' ? 'replace' : undefined,
            copyTarget: o.copyTarget ? o.copyTarget.trim() : undefined,
          });
          toast(r.msg || 'Restore completed.');
          await reload();
        },
        testrestore: async (el) => {
          toast('Running automated restore test into temporary environment…');
          await api.testBackupRestore(el.dataset.id);
          toast('Restore test passed.');
          await reload();
        },
        testrestoreall: async () => {
          toast('Running restore test on latest snapshot…');
          await api.testBackupRestore(null);
          toast('Restore test passed.');
          await reload();
        },
        showkey: async (el) => {
          const x = d.dests.find((item) => item.id === el.dataset.id);
          if (!x || !x.publicKey) return;
          await confirmDialog({
            title: `SSH public key for ${x.name}`,
            body: html`<p class="sm">Add this Ed25519 key to <span class="mono">~/.ssh/authorized_keys</span> on your backup server, then click <b>Test</b>.</p><pre class="mono sm" style="white-space:pre-wrap;word-break:break-all;background:var(--fill);padding:10px;border-radius:6px">${x.publicKey}</pre>${x.fingerprint ? html`<p class="mono sm muted">${x.fingerprint}</p>` : ''}`,
            confirm: 'Close',
          });
        },
        testdest: async (el) => {
          toast('Testing destination connection…');
          const res = await api.testSavedDestination(el.dataset.id);
          if (res.ok) {
            toast('Destination verified and ready.');
          } else {
            const firstFail = (res.steps || []).find(([s]) => s === 'fail');
            toast((firstFail && firstFail[1]) || 'Destination test failed.', 'err');
          }
          await reload();
        },
        defaultdest: async (el) => {
          await api.setDefaultDestination(el.dataset.id, { reassignExisting: true });
          toast('Updated default backup destination.');
          await reload();
        },
        deldest: async (el) => {
          const x = d.dests.find((item) => item.id === el.dataset.id);
          if (!x) return;
          if (!await confirmDialog({ title: `Remove "${x.name}"?`, body: 'Schedules using it revert to Local disk. Existing snapshots on remote storage are not deleted.', confirm: 'Remove', danger: true })) return;
          await api.deleteDestination(x.id);
          toast('Destination removed.');
          await reload();
        },
        key: async () => {
          const o = await formDialog({
            title: 'Download recovery key',
            body: 'Re-enter your account password to export the restic encryption recovery password.',
            fields: [{ name: 'password', label: 'Account password', type: 'password' }],
            submit: 'Download',
          });
          if (!o) return;
          const res = await api.downloadRecoveryKey(o.password);
          const host = res.hostname || server.name || 'server';
          const text = `# opsbox backup recovery key (${host})\n# ${res.instructions || ''}\nRESTIC_PASSWORD=${res.password}\n`;
          download(`opsbox-${host.replace(/[^a-z0-9.-]/gi, '-')}-recovery-key.txt`, text);
          toast('Recovery key downloaded.');
        },
      });
    },
  };
}
