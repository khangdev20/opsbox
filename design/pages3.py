# ---------- Boards added after review: Backups, Alerts, Templates, Access, Settings ----------
def sel(i,opts,val,w=''):
    o=''.join(f'<option{" selected" if x==val else ""}>{x}</option>' for x in opts)
    return f'<select id="{i}" style="min-height: 44px; padding: 0 12px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px; box-sizing: border-box; {w}">{o}</select>'
def onoff(on,label):
    c='var(--accent)' if on else 'var(--ctl)'
    return f'<button type="button" role="switch" aria-checked="{"true" if on else "false"}" aria-label="{label}" style="min-height: 44px; min-width: 64px; border: 1px solid {c}; border-radius: 8px; background: {"var(--accent)" if on else "transparent"}; color: {"var(--onaccent)" if on else "var(--muted)"}; {MO}; font-size: 13px; font-weight: 600; cursor: pointer">{"ON" if on else "OFF"}</button>'
def cell(t,sub=''):
    s=f'<span style="font-size: 13px; color: var(--muted)">{sub}</span>' if sub else ''
    return f'<span style="display: flex; flex-direction: column; gap: 2px; padding: 6px 0"><span style="font-size: 15px; font-weight: 500">{t}</span>{s}</span>'
def mc(t,c='var(--text)'): return mono(t,c,13)
def strip3(items):
    out=''
    for l,v,s in items:
        out+=f'<div style="flex: 1 1 220px; border-top: 1px solid var(--ls); padding-top: 14px; display: flex; flex-direction: column; gap: 8px"><span style="font-size: 14px; color: var(--muted)">{l}</span><span style="{MO}; font-size: 28px; font-weight: 500; letter-spacing: -0.03em">{v}</span><span style="{MO}; font-size: 13px; color: var(--muted)">{s}</span></div>'
    return f'<div style="display: flex; flex-wrap: wrap; gap: 32px">{out}</div>'
def callout(kind,text,right=''):
    c='var(--warn)' if kind=='warn' else 'var(--danger)'
    g='▲' if kind=='warn' else '✕'
    return f'<div role="note" style="display: flex; flex-wrap: wrap; align-items: center; gap: 12px 20px; padding: 12px 16px; border: 1px solid {c}; border-left-width: 4px; border-radius: 8px"><span style="{MO}; font-size: 13px; font-weight: 600; color: {c}">{g}</span><span style="flex: 1 1 320px; font-size: 15px">{text}</span>{right}</div>'
def para(t): return f'<p style="margin: 0; font-size: 14px; line-height: 1.7; color: var(--muted); max-width: 720px">{t}</p>'
def two(left,right,lw=3,rw=1):
    return f'<div style="display: flex; flex-wrap: wrap; gap: 48px; align-items: flex-start"><div style="flex: {lw} 1 560px; min-width: 0; display: flex; flex-direction: column; gap: 32px">{left}</div><div style="flex: {rw} 1 300px; max-width: 400px; min-width: 0; display: flex; flex-direction: column; gap: 32px">{right}</div></div>'

# ---------- Backups ----------
bk_strip=strip3([('Last backup','02:00 today','all 3 jobs succeeded · took 4 min 12 s'),('Next backup','02:00 tomorrow','in 21 h 40 min'),('Stored off-server','18.4 GB','s3://acme-backups · 14 daily + 8 weekly')])
bk_warn=callout('warn','Backups of <b>/var/backups</b> are stored on the same disk as your data. If the instance is lost, they are lost too. Add an off-server destination below.',btn('Add destination','primary',href='BackupDest.dc.html'))
bk_sched=table('Backup schedules',[('What','left'),('Schedule','left'),('Keep','left'),('Destination','left'),('Last run','left'),('','right')],[
 [cell('postgres-15','database · pg_dump, consistent snapshot'),mc('daily 02:00'),mc('14 daily, 8 weekly'),mc('S3 · acme-backups'),stat('run','ok · 1.2 GB'),btn('Run now','link')],
 [cell('Volume pgdata, redisdata','Docker volumes · stopped briefly if needed'),mc('daily 02:20'),mc('7 daily'),mc('S3 · acme-backups'),stat('run','ok · 4.2 GB'),btn('Run now','link')],
 [cell('/var/www/app, /etc/opsbox','files and panel settings'),mc('daily 02:40'),mc('7 daily'),mc('/var/backups'),stat('warn','same disk'),btn('Run now','link')],
 [cell('Whole instance','provider snapshot (EBS) · needs AWS access'),mc('not set up','var(--muted)'),mc('–','var(--muted)'),mc('–','var(--muted)'),stat('stop','off'),btn('Set up','link')]],minw=900)
bk_dest=table('Destinations',[('Name','left'),('Type','left'),('Location','left'),('Status','left'),('','right')],[
 [cell('Offsite'),mc('S3-compatible'),mc('s3://acme-backups/srv-sgn-01'),stat('run','verified 02:41'),btn('Edit','link')],
 [cell('Local disk'),mc('folder'),mc('/var/backups'),stat('warn','not off-server'),btn('Edit','link')]],minw=760)
bk_list=table('Backups',[('Taken','left'),('What','left'),('Size','right'),('Where','left'),('Check','left'),('','right')],[
 {'cells':[mc('today 02:00'),mc('postgres-15'),mc('1.2 GB'),mc('S3'),stat('run','restore-tested'),f'<span style="display: flex; gap: 4px; justify-content: flex-end">{btn("Restore","primary")}{btn("Download","link")}</span>'],'hl':True},
 [mc('yesterday 02:00'),mc('postgres-15'),mc('1.2 GB'),mc('S3'),stat('run','checksum ok'),f'<span style="display: flex; gap: 4px; justify-content: flex-end">{btn("Restore")}{btn("Download","link")}</span>'],
 [mc('today 02:20'),mc('volumes'),mc('4.2 GB'),mc('S3'),stat('run','checksum ok'),f'<span style="display: flex; gap: 4px; justify-content: flex-end">{btn("Restore")}{btn("Download","link")}</span>'],
 [mc('Sun 02:00'),mc('postgres-15 (weekly)'),mc('1.1 GB'),mc('S3'),stat('run','checksum ok'),f'<span style="display: flex; gap: 4px; justify-content: flex-end">{btn("Restore")}{btn("Download","link")}</span>']],minw=860)
bk_side=section('Restore, safely',f'{para("Restoring never overwrites live data by default. opsbox restores into a new database or folder next to the original, then you switch over when you are happy.")}<ol style="margin: 0; padding-left: 20px; font-size: 15px; line-height: 1.9"><li>Pick a backup and choose <span style="{MO}; font-size: 14px">Restore to a copy</span></li><li>Check the copy in Containers or Files</li><li>Swap it in, or delete the copy</li></ol>')+section('Encryption',f'{para("Backups are encrypted before they leave this server. Keep the recovery key somewhere safe: without it a backup cannot be opened.")}<div style="display: flex; flex-wrap: wrap; gap: 10px">{btn("Download recovery key")}{btn("Test restore now")}</div>')
main=hdr('Backups','7 of 7 runs succeeded this week · restore tested monthly',btn('Back up now','primary'))+bk_strip+bk_warn+two(section('Schedules',bk_sched)+section('Available backups',bk_list),section('Destinations',bk_dest)+bk_side,3,1)
W['Backups']=page('Backups','backups',main,h=1900)

# ---------- Alerts ----------
al_fire=callout('warn','<b>Disk / is at 82%</b> since 03:12, rule threshold is 75%. 14.6 GB can be reclaimed.',f'<span style="display: flex; gap: 10px">{btn("Clean up","primary",href="Cleanup.dc.html")}{btn("Silence 1 h")}</span>')
al_ch=table('Notification channels',[('Channel','left'),('Sends to','left'),('For','left'),('Status','left'),('','right')],[
 [cell('Email'),mc('admin@acme.co'),mc('all alerts'),stat('run','verified'),btn('Send test','link')],
 [cell('Slack'),mc('#ops-alerts'),mc('critical only'),stat('run','verified'),btn('Send test','link')],
 [cell('Telegram'),mc('not connected','var(--muted)'),mc('–','var(--muted)'),stat('stop','off'),btn('Connect','link')],
 [cell('Webhook'),mc('not set','var(--muted)'),mc('–','var(--muted)'),stat('stop','off'),btn('Add','link')]],minw=760)
def rule(name,cond,sev,ch,on):
    s=stat('fail','critical') if sev=='critical' else stat('warn','warning')
    return [cell(name),mc(cond),s,mc(ch),onoff(on,'Enable rule '+name),btn('Edit','link')]
al_rules=table('Alert rules',[('Rule','left'),('When','left'),('Level','left'),('Notify','left'),('On','left'),('','right')],[
 rule('Disk full','disk / above 85% for 5 min','critical','Slack + email',True),
 rule('Disk filling','disk / above 75% for 5 min','warning','email',True),
 rule('High CPU','above 90% for 10 min','warning','email',True),
 rule('High memory','above 90% for 5 min','warning','email',True),
 rule('Service or container down','any running one stops','critical','Slack + email',True),
 rule('Deploy failed','any service','warning','Slack',True),
 rule('Backup failed or missed','any job','critical','Slack + email',True),
 rule('TLS certificate expiring','under 14 days','warning','email',True),
 rule('Security updates waiting','for more than 3 days','warning','email',True),
 rule('Repeated failed sign-ins','5 or more from one IP','warning','email',False)],minw=980)
def hist(t,name,msg,stt):
    return [mc(t),cell(name,msg),stt]
al_hist=table('Recent alerts',[('Time','left'),('Alert','left'),('State','left')],[
 hist('03:12 today','Disk filling','disk / reached 82%',stat('warn','firing')),
 hist('yesterday 14:08','Deploy failed','shop-api 1c80ae7 · npm ci exited with code 1',stat('run','resolved')),
 hist('Mon 09:41','TLS certificate expiring','shop.example.com renewed automatically',stat('run','resolved')),
 hist('Sun 02:03','Backup failed or missed','S3 timeout, retried at 02:11 and passed',stat('run','resolved'))],minw=620)
al_side=section('Quiet hours',f'{para("Warnings wait until morning. Critical alerts always send.")}'+fld('qh1','From',sel('qh1',['22:00','23:00','00:00'],'23:00'))+fld('qh2','To',sel('qh2',['06:00','07:00','08:00'],'07:00'))+fld('qh3','Time zone',sel('qh3',['Asia/Ho_Chi_Minh','UTC','Australia/Brisbane'],'Asia/Ho_Chi_Minh'))+btn('Send a test alert'))+section('What to expect',para('Each alert says what happened, what it affects and the one place to fix it. The same problem is sent once, then again only when it clears or gets worse.'))
main=hdr('Alerts','2 channels connected · 10 rules · 1 firing',btn('Send a test alert'))+al_fire+two(section('Channels',al_ch)+section('Rules',al_rules)+section('Recent alerts',al_hist),al_side,3,1)
W['Alerts']=page('Alerts','alerts',main,h=1700)

# ---------- Templates ----------
def tpl(name,desc,res,sel_=False):
    b=btn('Add →','primary') if sel_ else btn('Add →')
    return {'cells':[cell(name,desc),mc(res),b],'hl':sel_}
tp_db=table('Databases',[('Database','left'),('Typical use','left'),('','right')],[
 tpl('PostgreSQL 16','Relational database · daily backup on by default','≈ 100 MB RAM · 1 GB disk',True),
 tpl('MySQL 8','Relational database','≈ 400 MB RAM · 1 GB disk'),
 tpl('MariaDB 11','MySQL-compatible database','≈ 150 MB RAM · 1 GB disk'),
 tpl('Redis 7','Cache and queues · memory only or persisted','≈ 20 MB RAM'),
 tpl('MongoDB 7','Document database','≈ 300 MB RAM · 1 GB disk')],minw=640)
tp_tools=table('Tools',[('Tool','left'),('What it does','left'),('','right')],[
 tpl('MinIO','S3-compatible file storage','≈ 150 MB RAM'),
 tpl('Uptime Kuma','Watch your public URLs from outside','≈ 100 MB RAM'),
 tpl('Umami','Privacy-friendly website analytics (needs PostgreSQL)','≈ 150 MB RAM'),
 tpl('n8n','Workflow automation','≈ 250 MB RAM'),
 tpl('Grafana','Dashboards for your own metrics','≈ 200 MB RAM')],minw=640)
yml='\n'.join([ln_('# paste or upload a docker-compose.yml'),'services:','  web:','    image: ghcr.io/acme/shop:latest','    ports: ["3000:3000"]','    env_file: .env','  cache:','    image: redis:7'])
tp_comp=f'{para("Already have a Compose file? Paste it and opsbox shows each service on the Apps graph, with logs, variables and rollback like any other service.")}'+code(yml,170)+f'<div style="display: flex; flex-wrap: wrap; gap: 10px">{btn("Deploy compose file","primary")}{btn("Upload a file")}{btn("From a GitHub repo →",href="NewService.dc.html")}</div>'
tp_form=section('Add PostgreSQL 16',fld('t1','Name',inp2('t1','postgres-main'),'Services in this project reach it at postgres-main:5432.')+fld('t2','Version',sel('t2',['16.4','16.3','15.8'],'16.4'))+fld('t3','Disk for data',sel('t3',['1 GB','5 GB','20 GB'],'5 GB'),'Can be grown later without downtime.')+chk('t4',True,'Back up daily at 02:00 to S3')+chk('t5',True,'Only reachable from other services (not the internet)')+f'<div style="display: flex; flex-direction: column; gap: 4px"><span style="font-size: 14px; font-weight: 500">Password</span>{mono("generated · stored encrypted · shown once as DATABASE_URL","var(--muted)",12)}</div>'+btn('Add PostgreSQL','primary'))
main=hdr('New service','shop-platform · production',btn('Cancel',href='Apps.dc.html'))+stepper(0)+tabs(['GitHub','Public repo URL','Docker image','Templates'],3)+two(section('Databases',tp_db)+section('Tools',tp_tools)+section('Your own Compose file',tp_comp),tp_form,3,1)
W['Templates']=page('Templates','apps',main,h=1750)

# ---------- Access (SSH keys and Linux users) ----------
ak=table('Authorized keys',[('Name','left'),('Fingerprint','left'),('Added','left'),('Last used','left'),('','right')],[
 [cell('Khang · MacBook','ed25519'),mc('SHA256:3Fq…n8Yk'),mc('Mar 02'),mc('today 03:40'),btn('Remove','link')],
 [cell('CI deploy key','ed25519 · restricted to git pull'),mc('SHA256:b71…Qd2w'),mc('May 19'),mc('2 min ago'),btn('Remove','link')],
 [cell('Old laptop','rsa 2048'),mc('SHA256:xK0…7mA1'),mc('2023'),stat('warn','unused 8 months'),btn('Remove','link')]],minw=820)
addkey=f'<div style="display: flex; flex-direction: column; gap: 14px; max-width: 640px">'+fld('k0','For user',sel('k0',['deploy','ubuntu','root'],'deploy'))+fld('k1','Public key',f'<textarea id="k1" rows="3" placeholder="ssh-ed25519 AAAA… name" style="padding: 12px 14px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px; box-sizing: border-box; resize: vertical"></textarea>','Paste the contents of the .pub file. Never paste a private key here.')+f'<div>{btn("Add key","primary")}</div></div>'
lu=table('Linux users',[('User','left'),('Groups','left'),('Sudo','left'),('Last login','left'),('Status','left'),('','right')],[
 [cell('ubuntu','default cloud user'),mc('sudo, docker'),mc('yes'),mc('Mar 02'),stat('run','active'),btn('Lock','link')],
 [cell('deploy','runs your apps'),mc('docker'),mc('no'),mc('today 03:40'),stat('run','active'),btn('Lock','link')],
 [cell('root','direct login'),mc('–','var(--muted)'),mc('–','var(--muted)'),mc('never'),stat('run','locked'),btn('Unlock','link')]],minw=820)
out=table('Keys opsbox uses to reach other servers',[('Name','left'),('Used for','left'),('Storage','left'),('','right')],[
 [cell('db-primary','deploy@10.0.1.12'),mc('Terminal tab'),stat('run','encrypted'),btn('Rotate','link')],
 [cell('edge-hn-02','ubuntu@203.0.113.40:2222'),mc('Terminal tab'),stat('run','encrypted'),btn('Rotate','link')]],minw=640)
sshs=f'<div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px 24px"><div style="display: flex; flex-direction: column; gap: 4px"><span style="font-size: 15px; font-weight: 500">SSH daemon is off</span>{mono("port 22 closed in the firewall · password login disabled")}</div>{btn("Open for 1 hour","primary")}</div>'
main=hdr('Access','who can log in to this server, and with what',btn('Open emergency SSH'))+tabs(['SSH keys','Linux users','Outgoing keys'],0)+section('SSH access',sshs)+section('Authorized keys',ak+addkey)+section('Linux users',lu+para('Locking a user keeps their files but stops every login and key. Panel accounts are separate, see Security.'))+section('Outgoing keys',out)
W['Access']=page('Access','security',main,h=1650)

# ---------- Settings ----------
st_gen=f'<div style="display: flex; flex-direction: column; gap: 16px; max-width: 520px">'+fld('s1','Hostname',inp2('s1','srv-sgn-01'))+fld('s2','Time zone',sel('s2',['Asia/Ho_Chi_Minh','UTC','Australia/Brisbane'],'Asia/Ho_Chi_Minh'),'Used for cron jobs, backups and alert quiet hours.')+fld('s3','Panel address',inp2('s3','https://panel.example.com'),'Certificate issued automatically. Port 8443 stays bound to localhost.')+f'<div>{btn("Save","primary")}</div></div>'
st_tok=table('API tokens',[('Name','left'),('Can do','left'),('Last used','left'),('','right')],[
 [cell('github-actions','deploy only, service web'),mc('2 min ago'),mc('Revoke','var(--danger)')],
 [cell('uptime-check','read-only metrics'),mc('1 min ago'),mc('Revoke','var(--danger)')]],minw=520)
st_tok=st_tok.replace('<th style="padding: 10px 16px 10px 0; text-align: left;','<th style="padding: 10px 16px 10px 0; text-align: left;')
st_disk=f'<div style="display: flex; flex-direction: column; gap: 16px; max-width: 520px">{para("Swap protects small instances from running out of memory. Disk growth needs the volume enlarged at your cloud provider first.")}'+fld('d1','Swap file',sel('d1',['None','1 GB','2 GB','4 GB'],'2 GB'))+f'<div style="display: flex; flex-wrap: wrap; align-items: center; gap: 12px">{btn("Grow filesystem to fill disk")}{mono("disk 120 GB · filesystem 100 GB","var(--muted)",12)}</div></div>'
st_up=f'<div style="display: flex; flex-direction: column; gap: 4px"><div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px 24px; min-height: 52px; border-bottom: 1px solid var(--line)"><span style="font-size: 15px">opsbox 0.1.4 · stable channel · up to date</span>{btn("Check now")}</div><div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px 24px; min-height: 52px; border-bottom: 1px solid var(--line)"><span style="font-size: 15px">Update opsbox automatically</span>{onoff(True,"Update opsbox automatically")}</div>{para("If a new version fails its health check, the watchdog rolls back to the previous one by itself.")}</div>'
st_gh=f'<div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px 24px"><div style="display: flex; flex-direction: column; gap: 4px"><span style="font-size: 15px; font-weight: 500">github.com/acme-co</span>{mono("GitHub App · read-only code access · 14 of 31 repositories")}</div><div style="display: flex; flex-wrap: wrap; gap: 10px">{btn("Manage access ↗")}{btn("Disconnect","danger")}</div></div>'
st_dz=f'<div style="display: flex; flex-direction: column; gap: 14px; max-width: 620px">{para("Export saves every setting, service and variable as one encrypted file you can import on a new server. Uninstall removes opsbox but leaves your apps and data running.")}<div style="display: flex; flex-wrap: wrap; gap: 10px">{btn("Export configuration")}{btn("Import configuration")}{btn("Uninstall opsbox","danger")}</div></div>'
main=hdr('Settings','srv-sgn-01 · opsbox 0.1.4')+section('General',st_gen)+section('API tokens',st_tok+f'<div>{btn("Create token")}</div>')+section('Memory and disk',st_disk)+section('GitHub',st_gh)+section('opsbox updates',st_up)+section('Export and uninstall',st_dz)
W['Settings']=page('Settings','settings',main,h=1950)
