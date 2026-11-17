s=open('gen.py',encoding='utf-8').read()
old=s[s.index("nav_items=["):s.index("\n",s.index("nav_items=["))]
s=s.replace(old,"nav_items=[('01','Overview','Main.dc.html','overview'),('02','Apps','Apps.dc.html','apps'),('03','Containers','Containers.dc.html','containers'),('04','System','System.dc.html','system'),('05','Cleanup','Cleanup.dc.html','cleanup'),('06','Terminal','Terminal.dc.html','terminal'),('07','Files','Files.dc.html','files'),('08','Security','Security.dc.html','security')]")
s=s.replace('"options":["overview","apps","containers","cleanup","terminal","files","security"]','"options":["overview","apps","containers","system","cleanup","terminal","files","security"]')
s=s.replace('hint-placeholder-count="7"','hint-placeholder-count="8"')
att='''def arow(st,txt,link):
    return '<div style="display: flex; flex-wrap: wrap; align-items: center; gap: 8px 20px; min-height: 48px; border-bottom: 1px solid var(--line)">'+st+'<span style="flex: 1 1 280px; font-size: 14px">'+txt+'</span>'+link+'</div>'
attention=section('Needs attention · 3','<div style="display: flex; flex-direction: column">'+arow(stat('warn','security'),'3 security updates are waiting (openssl, libc6, linux-image)',btn('Review and install →','link',href='System.dc.html'))+arow(stat('warn','reboot'),'A new kernel is installed. Reboot to use it.',btn('Schedule reboot →','link',href='System.dc.html'))+arow(stat('warn','disk'),'Disk / is at 82%. 14.6 GB can be reclaimed.',btn('Clean up →','link',href='Cleanup.dc.html'))+'</div>')
'''
assert "main=hdr('Overview'," in s
s=s.replace("main=hdr('Overview',",att+"main=hdr('Overview',",1)
a="</span>')+strip+f'''"
assert a in s
s=s.replace(a,"</span>')+attention+strip+f'''",1)
b=" ['Private SSH keys encrypted at rest',stat('run','on'),'']],minw=520)"
assert b in s
s=s.replace(b," ['Private SSH keys encrypted at rest',stat('run','on'),''],\n ['SSH daemon on this server (port 22 closed, managed through opsbox)',stat('run','off'),btn('Enable for 1 hour','link')],\n ['Watchdog restarts opsbox if it stops answering',stat('run','on'),''],\n ['Automatic rollback if an opsbox update fails its health check',stat('run','on'),btn('Safe mode page →','link',href='Recovery.dc.html')]],minw=520)")
s=s.replace("hdr('Security','5 of 6 checks passing'","hdr('Security','8 of 9 checks passing'")
s=s.replace("\nfor k,v in W.items(): open","\nexec(open('pages2.py',encoding='utf-8').read())\nfor k,v in W.items(): open",1)
open('gen.py','w',encoding='utf-8').write(s)
