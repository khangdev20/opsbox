def switch(label,on=True):
    return f'<button type="button" role="switch" aria-checked="{"true" if on else "false"}" aria-label="{label}" style="min-height: 44px; min-width: 64px; border: 1px solid {"var(--accent)" if on else "var(--ctl)"}; border-radius: 8px; background: {"var(--accent)" if on else "transparent"}; color: {"var(--onaccent)" if on else "var(--text)"}; {MO}; font-size: 13px; font-weight: 600; cursor: pointer">{"ON" if on else "OFF"}</button>'
def sel2(i,label,opts):
    return f'<select id="{i}" aria-label="{label}" style="min-height: 44px; padding: 0 12px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px">'+''.join(f'<option>{o}</option>' for o in opts)+'</select>'
GREY='#8A8578'
def ln_(t): return f'<span style="color: {GREY}">{t}</span>'
# ---------- System ----------
upd=table('Available updates',[('Package','left'),('Installed','left'),('Available','left'),('Type','left')],[
 [mono('openssl','var(--text)',13),mono('3.0.13-0ubuntu3.4'),mono('3.0.13-0ubuntu3.5'),stat('warn','security')],
 [mono('libc6','var(--text)',13),mono('2.39-0ubuntu8.2'),mono('2.39-0ubuntu8.3'),stat('warn','security')],
 [mono('linux-image-generic','var(--text)',13),mono('6.8.0-48'),mono('6.8.0-52'),stat('warn','security')],
 [mono('curl','var(--text)',13),mono('8.5.0-2ubuntu10.4'),mono('8.5.0-2ubuntu10.5'),mono('regular')],
 [mono('tzdata','var(--text)',13),mono('2024a-3ubuntu1.1'),mono('2024b-0ubuntu0.24.04'),mono('regular')]],minw=640)
upd_body=(
 '<div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px 24px"><span style="font-size: 15px">14 updates available · <span style="color: var(--warn)">3 security</span> · last checked 03:30</span><div style="display: flex; flex-wrap: wrap; gap: 10px">'+btn("Install security updates (3)","primary")+btn("Install all (14)")+btn("Check now")+'</div></div>'
 +'<div style="display: flex; flex-wrap: wrap; align-items: center; gap: 12px 20px; padding: 12px 14px; border: 1px solid var(--warn); border-radius: 8px"><span style="color: var(--warn); '+MO+'; font-size: 13px; font-weight: 500">▲ REBOOT REQUIRED</span><span style="flex: 1 1 280px; font-size: 15px">Kernel 6.8.0-52 is installed but 6.8.0-48 is running. Containers restart automatically after reboot.</span>'+btn("Reboot now","danger")+btn("Schedule…")+'</div>'
 +upd
 +'<div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px"><div style="display: flex; flex-direction: column; gap: 3px"><span style="font-size: 15px; font-weight: 500">Install security updates automatically</span><span style="font-size: 14px; color: var(--muted)">Nightly 04:00 · reboot allowed Sundays 04:30 · apps are restarted with a health check</span></div>'+switch("Automatic security updates")+'</div>')
def U(n): return mono(n,'var(--text)',13)
svcs=table('systemd services',[('Unit','left'),('State','left'),('Boot','left'),('Memory','right'),('','right')],[
 [U('opsbox.service'),stat('run','active'),mono('enabled'),mono('14 MB','var(--text)'),btn('restart','link',aria='Restart opsbox')+btn('logs','link',aria='Logs opsbox')],
 [U('docker.service'),stat('run','active'),mono('enabled'),mono('142 MB','var(--text)'),btn('restart','link',aria='Restart docker')+btn('logs','link',aria='Logs docker')],
 [U('chrony.service'),stat('run','active'),mono('enabled'),mono('3 MB','var(--text)'),btn('restart','link',aria='Restart chrony')+btn('logs','link',aria='Logs chrony')],
 [U('snapd.service'),stat('fail','failed'),mono('enabled'),mono('–'),btn('restart','link',aria='Restart snapd')+btn('logs','link',aria='Logs snapd')],
 [U('ssh.service'),stat('stop','inactive'),mono('disabled'),mono('–'),btn('start for 1 hour','link',aria='Start ssh for one hour')]],minw=720)
jl='\n'.join([ln_('03:51:12')+' opsbox[412]: deploy web a3f9c21 queued by minh.dev',
 ln_('03:51:44')+' dockerd[887]: Container web-app health_status: healthy',
 ln_('03:52:03')+' systemd[1]: snapd.service: Main process exited, code=exited, status=1',
 ln_('03:52:03')+" systemd[1]: snapd.service: Failed with result 'exit-code'.",
 ln_('03:55:20')+' kernel: [UFW BLOCK] IN=eth0 SRC=203.0.113.9 DST=10.0.1.4 DPT=22',
 ln_('03:55:21')+' kernel: [UFW BLOCK] IN=eth0 SRC=203.0.113.9 DST=10.0.1.4 DPT=22'])
logs=('<div style="display: flex; flex-wrap: wrap; gap: 10px; align-items: center">'+sel2("lu","Unit",["All units","opsbox.service","docker.service","snapd.service"])+sel2("lp","Priority",["info and above","warning and above","errors only"])+sel2("lt","Time range",["last hour","last 24 hours","last 7 days"])
 +'<input type="search" aria-label="Search logs" placeholder="Search logs" style="flex: 1 1 220px; min-height: 44px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); '+MO+'; font-size: 14px; box-sizing: border-box">'+btn("Follow")+btn("Download")+'</div>'+code(jl,170))
jobs=table('Scheduled jobs',[('Schedule','left'),('Job','left'),('Last run','left'),('Result','left'),('','right')],[
 [U('0 2 * * *'),'Back up postgres to /var/backups',mono('02:00 today'),stat('run','ok'),btn('run now','link',aria='Run backup now')+btn('edit','link',aria='Edit backup job')],
 [U('0 3 * * 0'),'Safe cleanup (journal, apt, tmp)',mono('Sun 03:00'),stat('run','ok · 2.1 GB'),btn('run now','link',aria='Run cleanup now')+btn('edit','link',aria='Edit cleanup job')],
 [U('17 */12 * * *'),'Renew TLS certificates',mono('03:17 today'),stat('run','ok'),btn('run now','link',aria='Run renew now')+btn('edit','link',aria='Edit renew job')]],minw=760)
fw=table('Firewall rules',[('Port','left'),('Protocol','left'),('Source','left'),('Action','left'),('Note','left'),('','right')],[
 [U('443'),mono('tcp'),mono('anywhere'),stat('run','allow'),'HTTPS for apps and this panel',btn('remove','link',aria='Remove rule 443')],
 [U('80'),mono('tcp'),mono('anywhere'),stat('run','allow'),'Redirect to HTTPS, certificate checks',btn('remove','link',aria='Remove rule 80')],
 [U('8443'),mono('tcp'),mono('203.0.113.0/24'),stat('run','allow'),'Admin port, office network only',btn('remove','link',aria='Remove rule 8443')],
 [U('22'),mono('tcp'),mono('anywhere'),stat('stop','closed'),'SSH is off. Everything is done in the browser.',btn('open for 1 hour','link',aria='Open port 22 for one hour')]],minw=820)
fwnote='<p style="margin: 0; font-size: 14px; line-height: 1.6; color: var(--muted); max-width: 760px">Default policy: deny all inbound. These rules apply to this server. If you use AWS security groups or another cloud firewall, they are separate and must allow the same ports.</p>'
ssh='<div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px 24px"><div style="display: flex; flex-direction: column; gap: 4px; max-width: 640px"><span style="font-size: 15px; font-weight: 500">Emergency SSH</span><span style="font-size: 14px; color: var(--muted); line-height: 1.5">Off by default. Turning it on opens port 22 to your current IP for one hour, then closes it again and logs the session.</span></div>'+btn("Enable for 1 hour")+'</div>'
main=hdr('System','srv-sgn-01 · Ubuntu 24.04 LTS · kernel 6.8.0-48 · up 41 days · 10.0.1.4',btn('Reboot instance'))+section('Updates',upd_body)+section('Services',svcs,btn('Show all units','link'))+section('Logs',logs)+section('Scheduled jobs',jobs,btn('+ New job'))+section('Firewall',fw+fwnote,btn('+ Add rule'))+section('SSH access',ssh)
W['System']=page('System','system',main,h=2900)
# ---------- Recovery ----------
rchk=table('Health checks',[('Check','left'),('Result','left'),('Detail','left')],[
 ['opsbox.service',stat('fail','failed'),mono('exit code 101 at 03:51:47 · restarted 3 times')],
 ['Health endpoint /healthz',stat('fail','no answer'),mono('timeout after 5 s')],
 ['Disk /',stat('fail','100% full'),mono('0 B free · likely cause: opsbox cannot write its database')],
 ['Docker',stat('run','running'),mono('4 containers still serving traffic')],
 ['Network and DNS',stat('run','ok'),mono('443 reachable from outside')]],minw=640)
acts=[('Free disk space','Remove unused Docker data, old logs and apt cache (about 14.6 GB). Running apps are not touched.',btn('Free space','primary')),
 ('Restart opsbox','Restart the service and run the health check again.',btn('Restart')),
 ('Roll back opsbox','Go back to 0.1.3, the previous version. Your settings and data are kept.',btn('Roll back to 0.1.3')),
 ('Open emergency SSH','Open port 22 to your IP for one hour. Last resort.',btn('Enable for 1 hour')),
 ('Reboot instance','Restarts the whole server. Apps come back automatically.',btn('Reboot','danger'))]
ractions='<div style="display: flex; flex-direction: column">'+''.join('<div style="display: flex; flex-wrap: wrap; align-items: center; gap: 10px 24px; min-height: 64px; border-bottom: 1px solid var(--line)"><div style="flex: 1 1 320px; display: flex; flex-direction: column; gap: 2px"><span style="font-size: 15px; font-weight: 500">'+t+'</span><span style="font-size: 14px; color: var(--muted)">'+d+'</span></div>'+b+'</div>' for t,d,b in acts)+'</div>'
rlog='\n'.join([ln_('03:51:44')+' INFO  opsbox 0.1.4 starting',ln_('03:51:46')+' ERROR sqlite: database or disk is full (code 13)',ln_('03:51:47')+' FATAL cannot persist session store, exiting',ln_('03:51:47')+' watchdog: opsbox exited with code 101, restart 1/3',ln_('03:52:19')+' watchdog: restart 3/3 failed, entering safe mode'])
safe_head='<header style="border-bottom: 1px solid var(--line)"><div style="max-width: 1100px; margin: 0 auto; padding: 8px 32px; box-sizing: border-box; display: flex; flex-wrap: wrap; align-items: center; gap: 8px 20px; min-height: 56px">'+logo(26,17)+'<span style="'+MO+'; font-size: 13px; font-weight: 600; letter-spacing: 0.06em; color: var(--bg); background: var(--warn); padding: 3px 8px; border-radius: 8px">SAFE MODE</span><span style="flex: 1"></span><span style="'+MO+'; font-size: 14px; color: var(--muted)">srv-sgn-01 · :8444 · signed in as admin</span></div></header>'
rmain=('<main style="flex: 1; width: 100%; max-width: 1100px; margin: 0 auto; box-sizing: border-box; padding: 40px 32px 64px; display: flex; flex-direction: column; gap: 36px">'
 +hdr("opsbox is not responding","your apps are still running · this page is served by a separate watchdog process")
 +section("What the watchdog found",rchk)+section("What you can do",ractions)+section("Last log lines",code(rlog,140))
 +'<p style="margin: 0; font-size: 14px; line-height: 1.7; color: var(--muted); max-width: 720px">If this page cannot be reached either, use your cloud provider instead: AWS Systems Manager Session Manager or EC2 Serial Console, or reboot from the EC2 console. The watchdog starts again after a reboot.</p></main>')
rbody='<div data-theme="{{theme}}" style="min-height: 100vh; display: flex; flex-direction: column; background: var(--bg); color: var(--text); '+UI+'">'+safe_head+rmain+'</div>'
W['Recovery']=head('opsbox — Safe mode',rbody,'',h=1300)
# ---------- Setup ----------
def sstep(n,t,on=False):
    c='var(--text)' if on else 'var(--muted)'
    bd='2px solid var(--accent)' if on else '2px solid var(--line)'
    return '<div style="padding: 10px 0 10px 14px; border-left: '+bd+'; display: flex; gap: 12px; align-items: baseline"><span style="'+MO+'; font-size: 13px; color: '+('var(--accent)' if on else 'var(--muted)')+'">'+n+'</span><span style="font-size: 15px; font-weight: 500; color: '+c+'">'+t+'</span></div>'
ec2='\n'.join([ln_('[   18.402]')+' cloud-init: installing opsbox 0.1.4',ln_('[   24.911]')+' opsbox: listening on https://0.0.0.0:8443',ln_('[   24.912]')+' opsbox setup ready',ln_('[   24.912]')+'   url    https://203.0.113.20:8443/setup',ln_('[   24.912]')+'   token  7QK2-9XMD-4TFA   (single use, expires in 60 min)'])
udata='#!/bin/bash\ncurl -fsSL https://get.opsbox.example/install.sh | sh'
sbody=('<div data-theme="{{theme}}" style="min-height: 100vh; display: flex; flex-wrap: wrap; background: var(--bg); color: var(--text); '+UI+'">'
 +'<div style="flex: 1 1 360px; max-width: 460px; box-sizing: border-box; padding: 56px 48px; display: flex; flex-direction: column; gap: 48px; border-right: 1px solid var(--ls)">'
 +logo(36,23)
 +'<div style="display: flex; flex-direction: column; gap: 4px"><span style="font-size: 14px; color: var(--muted); margin-bottom: 8px">First-time setup · about 3 minutes</span>'+sstep("01","Claim this server",True)+sstep("02","Create the admin account")+sstep("03","Domain and HTTPS")+sstep("04","Connect GitHub")+sstep("05","Lock down SSH")+'</div>'
 +'<p style="margin: 0; font-size: 14px; line-height: 1.6; color: var(--muted)">After setup you manage this server entirely from the browser. SSH can be closed for good.</p></div>'
 +'<div style="flex: 1 1 520px; box-sizing: border-box; padding: 56px 64px; display: flex; flex-direction: column; gap: 28px; max-width: 820px">'
 +'<div style="display: flex; flex-direction: column; gap: 8px"><h1 style="margin: 0; font-size: 32px; font-weight: 600; letter-spacing: -0.03em">Claim this server</h1><p style="margin: 0; font-size: 15px; line-height: 1.6; color: var(--muted); max-width: 560px">Anyone who can reach this address could set it up, so opsbox asks for a one-time token. It is printed in the instance boot log, which only you can read in your cloud console.</p></div>'
 +tabs(["AWS EC2","Google Cloud","Azure","Other"],0)
 +'<ol style="margin: 0; padding-left: 20px; font-size: 15px; line-height: 1.9"><li>Open the EC2 console and select this instance.</li><li>Choose Actions, then Monitor and troubleshoot, then Get system log.</li><li>Find the line that says <span style="'+MO+'; font-size: 14px">opsbox setup ready</span> and copy the token.</li></ol>'
 +code(ec2,0)
 +'<form style="display: flex; flex-direction: column; gap: 16px; max-width: 420px"><div style="display: flex; flex-direction: column; gap: 6px"><label for="tk" style="font-size: 14px; font-weight: 500">Setup token</label><input id="tk" type="text" placeholder="XXXX-XXXX-XXXX" autocomplete="off" style="min-height: 48px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); '+MO+'; font-size: 16px; letter-spacing: 0.1em; box-sizing: border-box"></div>'+btn("Continue","primary",href="Login.dc.html")+'</form>'
 +'<div style="display: flex; flex-direction: column; gap: 8px; border-top: 1px solid var(--line); padding-top: 20px"><span style="font-size: 14px; font-weight: 500">Launching a new instance?</span><span style="font-size: 14px; color: var(--muted)">Paste this into the User data field when you create it, and allow ports 443 and 8443 in the security group.</span>'+code(udata,0)+'</div>'
 +'<div><button type="button" onClick="{{toggleTheme}}" style="min-height: 44px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: transparent; color: var(--text); font: 500 13px \'Geist\', sans-serif; cursor: pointer">{{themeLabel}} theme</button></div>'
 +'</div></div>')
W['Setup']=head('opsbox — First-time setup',sbody,'',h=1100)
