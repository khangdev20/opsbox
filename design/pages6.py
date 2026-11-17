# ---------- Backups › Add destination (provider picker + credentials) ----------
CF6="font-family: 'Geist Mono', ui-monospace, monospace"
PROV=[('AWS S3','Amazon','aws','S3',False),('Cloudflare R2','Cloudflare','cf','R2',True),('Backblaze B2','Backblaze','b2','B2',False),('Wasabi','Wasabi','wa','WA',False),
      ('DigitalOcean Spaces','DigitalOcean','do','DO',False),('Hetzner Storage','Hetzner','hz','HZ',False),('Google Cloud Storage','Google Cloud','gcs','GC',False),('MinIO','Self-hosted','mn','MI',False),
      ('SFTP server','Any Linux box over SSH','sftp','SF',False),('Other S3-compatible','Custom endpoint','s3','S3',False)]
def ptile(name,sub,key,mono,sel):
    bd='2px solid var(--accent)' if sel else '1px solid var(--line)'
    pad='13px' if sel else '14px'
    chkm=f'<span style="position: absolute; top: 10px; right: 10px; width: 20px; height: 20px; border-radius: 50%; background: var(--accent); color: var(--onaccent); display: flex; align-items: center; justify-content: center; font-size: 12px; font-weight: 700">✓</span>' if sel else ''
    return f'''<button type="button" aria-pressed="{'true' if sel else 'false'}" style="position: relative; text-align: left; padding: {pad}; border: {bd}; border-radius: 14px; background: var(--hl); color: var(--text); display: flex; flex-direction: column; gap: 10px; cursor: pointer; font: inherit; min-height: 112px">{chkm}
<span data-logo-slot="{key}" title="Provider logo goes here" style="width: 40px; height: 40px; border-radius: 10px; border: 1px dashed var(--ls); background: var(--fill); display: flex; align-items: center; justify-content: center; font: 600 13px 'Geist', sans-serif; color: var(--muted)">{mono}</span>
<span style="display: flex; flex-direction: column; gap: 2px"><b style="font-size: 14.5px; font-weight: 600">{name}</b><span style="font-size: 12.5px; color: var(--muted)">{sub}</span></span></button>'''
grid='<div style="display: grid; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); gap: 12px">'+''.join(ptile(*p) for p in PROV)+'</div>'
def secret(i,label,val,hint=''):
    inner=f'<div style="display: flex; gap: 8px"><input id="{i}" type="password" value="{val}" style="flex: 1; min-height: 40px; padding: 0 12px; border: 1px solid var(--line); border-radius: 10px; background: var(--input); color: var(--text); {CF6}; font-size: 13px; box-sizing: border-box">{btn("Show","link")}</div>'
    return fld(i,label,inner,hint)
def ro(i,label,val,hint=''):
    return fld(i,label,f'<input id="{i}" type="text" value="{val}" readonly style="min-height: 40px; padding: 0 12px; border: 1px solid var(--line); border-radius: 10px; background: var(--fill); color: var(--muted); {CF6}; font-size: 13px; box-sizing: border-box">',hint)
form=f'''<div style="display: flex; flex-direction: column; gap: 16px">
{fld('ra','Account ID',inp2('ra','4f1c0e9a7b2d4c8e9f3a1b2c3d4e5f60'),'Cloudflare dashboard › R2 › Overview, right-hand side.')}
{ro('re','Endpoint','https://4f1c0e9a…r2.cloudflarestorage.com','Filled in from the Account ID.')}
{fld('rb','Bucket',inp2('rb','acme-backups'),'Create it first in R2. Use one bucket per server or a folder below.')}
{fld('rp','Folder in bucket',inp2('rp','srv-sgn-01/'))}
{fld('rk','Access Key ID',inp2('rk','a8c3f2…'))}
{secret('rs','Secret Access Key','••••••••••••••••••••','Stored encrypted on this server. Never shown again after saving.')}
{fld('rn','Name in opsbox',inp2('rn','Offsite (R2)'))}
</div>'''
howto=f'''<ol style="margin: 0; padding-left: 20px; display: flex; flex-direction: column; gap: 10px; font-size: 14px; line-height: 1.55">
<li>In the Cloudflare dashboard open <b>R2 Object Storage</b> and create a bucket.</li>
<li>Go to <b>Manage R2 API Tokens</b> › <b>Create API token</b>.</li>
<li>Choose <b>Object Read &amp; Write</b> and limit it to that one bucket.</li>
<li>Copy the Access Key ID and Secret here. Cloudflare shows the secret only once.</li></ol>
<a href="#" style="font-size: 14px; color: var(--accent)">Open Cloudflare R2 docs ↗</a>
<div role="note" style="display: flex; gap: 10px; padding: 12px 14px; border-radius: 12px; background: var(--fill); font-size: 13.5px; line-height: 1.55"><span style="color: var(--ok); font-weight: 700">✓</span><span>Good choice for backups: R2 charges nothing to download, so restoring costs only storage.</span></div>'''
test=f'''<div style="display: flex; flex-direction: column; gap: 10px">
<div style="display: flex; flex-direction: column; gap: 8px; font-size: 14px">
<span style="display: flex; align-items: center; gap: 10px">{stat('run','ok')} Connected to endpoint</span>
<span style="display: flex; align-items: center; gap: 10px">{stat('run','ok')} Bucket acme-backups found</span>
<span style="display: flex; align-items: center; gap: 10px">{stat('run','ok')} Wrote and read back a 1 KB test file</span>
<span style="display: flex; align-items: center; gap: 10px">{stat('warn','check')} Key can also delete objects. Fine for pruning old backups; use a separate key if you want append-only.</span></div></div>'''
enc=f'''<div style="display: flex; flex-direction: column; gap: 12px; font-size: 14px; line-height: 1.55">
<p style="margin: 0; color: var(--muted)">Everything is encrypted on this server before upload. Cloudflare only ever sees encrypted data.</p>
<div style="display: flex; flex-wrap: wrap; gap: 8px">{btn('Download recovery key')}</div>
<span style="font-size: 13px; color: var(--warn)">▲ Keep the recovery key off this server. Without it, these backups cannot be opened.</span></div>'''
stepper6=lambda n,t,on: f'<span style="display: flex; align-items: center; gap: 8px; font-size: 14px; {"color: var(--text); font-weight: 600" if on else "color: var(--muted)"}"><span style="width: 24px; height: 24px; border-radius: 50%; display: flex; align-items: center; justify-content: center; font-size: 12px; font-weight: 600; {"background: var(--accent); color: var(--onaccent)" if on else "background: var(--fill2)"}">{n}</span>{t}</span>'
steps6='<div style="display: flex; flex-wrap: wrap; gap: 10px 24px">'+stepper6('✓','Choose provider',False)+stepper6('2','Credentials',True)+stepper6('3','Test and save',False)+'</div>'
main=hdr('Add backup destination','Where should opsbox send backups for srv-sgn-01?',btn('Cancel',href='Backups.dc.html'))+steps6+section('Provider',grid+'<p style="margin: 0; font-size: 13px; color: var(--muted)">Any S3-compatible storage works. Pick yours and opsbox fills in the endpoint and shows where to find the keys.</p>')+f'''<div style="display: flex; flex-wrap: wrap; gap: 24px; align-items: flex-start">
<div style="flex: 3 1 520px; min-width: 0; display: flex; flex-direction: column; gap: 24px">{section('Cloudflare R2 credentials',form)}{section('Test connection',test,btn('Run test again','link'))}</div>
<div style="flex: 2 1 320px; min-width: 0; display: flex; flex-direction: column; gap: 24px">{section('Where to find these keys',howto)}{section('Encryption',enc)}</div></div>
<div style="display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 10px">{btn('Back')}{btn('Save destination','primary',href='Backups.dc.html')}</div>'''
W['BackupDest']=page('Backups · Add destination','backups',main,h=1900)
