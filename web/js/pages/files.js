import { html, raw, esc, bytes, table, btn, header, bindActions, confirmDialog, askForm, toast, patch, dropzone, empty } from '../ui.js';
import * as api from '../api.js';
import { server } from '../store.js';
import { loadAmd } from '../vendor-loader.js';

// Monaco (the VS Code editor core), self-hosted and loaded on first use.
let monacoP;
function loadMonaco() {
  if (monacoP) return monacoP;
  monacoP = loadAmd().then((require) => new Promise((resolve, reject) => {
    const base = new URL('../../vendor/monaco/vs', import.meta.url).href;
    // Same-origin worker file (not data:) so CSP script-src 'self' allows it.
    const worker = new URL('../../vendor/monaco/opsbox-editor.worker.js', import.meta.url).href;
    window.MonacoEnvironment = { getWorkerUrl: () => worker };
    require.config({ paths: { vs: base } });
    require(['vs/editor/editor.main'], () => resolve(window.monaco), reject);
  })).catch((e) => { monacoP = null; throw e; });
  return monacoP;
}
const LANG = { js: 'javascript', mjs: 'javascript', ts: 'typescript', json: 'json', yml: 'yaml', yaml: 'yaml', md: 'markdown', py: 'python', sh: 'shell', css: 'css', html: 'html', xml: 'xml', toml: 'ini', conf: 'ini', ini: 'ini', env: 'ini', sql: 'sql', go: 'go', rs: 'rust', php: 'php', rb: 'ruby', log: 'plaintext' };
const langOf = (path) => { const n = path.split('/').pop().toLowerCase(); if (n === 'dockerfile') return 'dockerfile'; return LANG[n.split('.').pop()] || 'plaintext'; };

export async function view({ query }) {
  let dir = query.p || (api.MODE === 'http' ? (server.role === 'deployer' ? '/var/lib/opsbox/apps' : '/') : '/var/www/app'), listing, open = null, dirty = false, picked = new Set(), root, ed = null;
  const disposeEd = () => { if (ed) { ed.dispose(); ed = null; } };
  const load = async () => { listing = await api.listDir(dir); picked.clear(); };
  await load();
  const crumbs = () => { const parts = dir.split('/').filter(Boolean); let acc = ''; return html`<nav class="path" aria-label="Path"><a href="#" data-act="go" data-p="/">/</a>${parts.map((p, i) => { acc += '/' + p; return html`<a href="#" data-act="go" data-p="${acc}">${p}</a>${i < parts.length - 1 ? '/' : ''}`; })}</nav>`; };
  const list = () => !listing.entries.length ? empty('This folder is empty. Upload a file or create a folder.', btn('Upload a file', { act: 'upload', kind: 'primary' })) : table('Files', [['', { label: false }], ['Name'], ['Mode'], ['Size', { align: 'r', label: 'Size' }], ['Modified']], [
    ...(dir !== '/' ? [[ '', html`<a href="#" data-act="go" data-p="${dir.split('/').slice(0, -1).join('/') || '/'}" class="mono">../</a>`, '', '', '']] : []),
    ...listing.entries.map((e) => Object.assign([html`<input type="checkbox" data-act="pick" data-p="${e.path}" aria-label="Select ${e.name}" ${raw(picked.has(e.path) ? 'checked' : '')}>`,
      e.type === 'd' ? html`<a href="#" class="mono" data-act="go" data-p="${e.path}">${e.name}/</a>` : html`<a href="#" class="mono" data-act="open" data-p="${e.path}">${e.name}</a>${api.downloadUrl(e.path) ? html` <a href="${api.downloadUrl(e.path)}" class="muted" title="Download" aria-label="Download ${e.name}" download>↓</a>` : ''}${e.warn ? html` <span class="st warn" title="Readable by everyone">▲ world-readable secrets</span>` : ''}`,
      html`<span class="mono">${e.mode}</span>`, e.size == null ? '' : bytes(e.size), e.mtime], { meta: { key: e.path, sig: `${e.mtime}|${e.size}` } }))], { sticky: true, cards: false });
  const editor = () => open ? html`<section class="sec"><div class="sec-h"><h2 class="mono">${open.path}</h2><span class="row">${btn('Save', { act: 'save', kind: 'primary' })}${api.downloadUrl(open.path) ? html`<a class="btn" href="${api.downloadUrl(open.path)}" download>Download</a>` : ''}${btn('Permissions', { act: 'chmod' })}${btn('Close', { act: 'close' })}</span></div>
    <div id="mon" role="group" aria-label="Editor for ${open.path}" style="height:420px;border:1px solid var(--ls);border-radius:8px;background:#1e1e1e"><p class="muted sm" style="padding:16px">Loading editor…</p></div><p class="sm muted mono" id="es" style="margin-top:6px">${open.mode} · ${langOf(open.path)} · Ctrl S to save</p></section>` : '';
  let fresh = false; // opening another folder is a new view: nothing enters or flashes
  const paint = () => {
    const opts = fresh ? { animate: false } : undefined; fresh = false;
    root.classList.toggle('editing', !!open);
    patch(root.querySelector('#c'), crumbs().toString(), opts); patch(root.querySelector('#l'), list().toString(), opts); root.querySelector('#e').innerHTML = editor().toString();
    root.querySelector('#del').disabled = !picked.size;
    disposeEd();
    const host = root.querySelector('#mon'); if (host) {
      const path = open.path;
      loadMonaco().then((monaco) => {
        if (!open || open.path !== path || !host.isConnected) return;
        host.innerHTML = '';
        ed = monaco.editor.create(host, { value: open.content, language: langOf(path), theme: 'vs-dark', automaticLayout: true, minimap: { enabled: true }, fontFamily: 'Geist Mono, monospace', fontSize: 13, scrollBeyondLastLine: false, tabSize: 2, renderWhitespace: 'selection', bracketPairColorization: { enabled: true } });
        ed.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => root.querySelector('[data-act=save]')?.click());
        ed.onDidChangeModelContent(() => { dirty = ed.getValue() !== open.content; const es = root.querySelector('#es'); if (es) es.textContent = `${open.mode} · ${langOf(path)} · ${dirty ? '● unsaved changes · ' : ''}Ctrl S to save`; });
        ed.focus();
      }).catch((e) => { host.innerHTML = `<p class="st fail" style="padding:16px">✕ ${esc(e.message)}</p>`; });
    }
  };
  return {
    nav: 'files', title: 'Files',
    html: html`<main class="files-page">${header('Files', 'Browse and edit files on this server.', html`<span class="row">${btn('New folder', { act: 'mkdir' })}${raw(btn('Delete', { act: 'delete', kind: 'danger', disabled: true }).toString().replace('<button', '<button id="del"'))}</span>`)}<div id="c"></div>${dropzone({ id: 'up', label: 'Drop a file here to upload it to this folder, or', browse: 'choose a file' })}<div id="l"></div><div id="e" style="margin-top:24px"></div></main>`,
    mount(r) {
      root = r; paint();
      const rl = async () => { await load(); paint(); };
      root.querySelector('#up').addEventListener('change', async (e) => {
        const f = e.target.files[0]; const z = e.target.closest('.drop'); if (!f || z.hasAttribute('data-busy')) return;
        z.setAttribute('data-busy', '1'); z.querySelector('.drop-t').textContent = `Uploading ${f.name}…`;
        try { await api.upload(dir, f); toast(`Uploaded ${f.name}.`); await rl(); }
        catch (err) { toast(err.message || 'Upload failed.', 'err'); }
        finally { e.target.value = ''; z.removeAttribute('data-busy'); z.classList.remove('has'); z.querySelector('.drop-t').innerHTML = 'Drop a file here to upload it to this folder, or <u>choose a file</u>'; z.querySelector('[data-file]').textContent = 'No file chosen'; }
      });
      bindActions(root, {
        upload: () => root.querySelector('#up').click(),
        go: async (el) => { fresh = true; dir = el.dataset.p; open = null; await rl(); },
        open: async (el) => { open = await api.readFile(el.dataset.p); paint(); },
        close: async () => { if (dirty && !await confirmDialog({ title: 'Discard unsaved changes?', body: open.path, confirm: 'Discard', danger: true })) return; open = null; paint(); },
        save: async () => { if (!ed) return; const v = ed.getValue(); await api.writeFile(open.path, v); open.content = v; dirty = false; toast('Saved.'); listing = await api.listDir(dir); patch(root.querySelector('#l'), list().toString()); const es = root.querySelector('#es'); if (es) es.textContent = `${open.mode} · ${langOf(open.path)} · Ctrl S to save`; },
        pick: (el) => { el.checked ? picked.add(el.dataset.p) : picked.delete(el.dataset.p); root.querySelector('#del').disabled = !picked.size; },
        mkdir: async () => { if (await askForm({ title: 'New folder', fields: [{ name: 'n', label: 'Name' }], submit: 'Create' }, (o) => { if (!o.n.trim() || o.n.includes('/')) throw new Error('Enter a folder name without slashes.'); return api.mkdir(`${dir}/${o.n.trim()}`); })) await rl(); },
        chmod: async () => { if (await askForm({ title: 'Permissions', body: open.path, fields: [{ name: 'm', label: 'Mode (octal)', value: '600', hint: '600 = owner only, 644 = everyone can read' }], submit: 'Apply' }, (o) => api.chmod(open.path, o.m))) { toast('Permissions updated.'); await rl(); } },
        delete: async () => { const ps = [...picked]; if (await confirmDialog({ title: `Delete ${ps.length} item${ps.length > 1 ? 's' : ''}?`, body: ps.join(', ') + (api.MODE === 'http' ? ' will be moved to the trash and kept for 7 days.' : ' will be removed permanently.'), confirm: 'Delete', danger: true })) { await api.removePaths(ps); if (open && ps.includes(open.path)) open = null; await rl(); } },
      });
    },
    unmount() { disposeEd(); },
  };
}
