# ---------- Apps › web › Domains & SSL ----------
CODEF="font-family: 'Geist Mono', ui-monospace, monospace"
def dom_row(name,tags,dns,cert,actions):
    t=''.join(f'<span style="padding: 2px 8px; border-radius: 99px; background: var(--fill2); font-size: 12px; color: var(--muted)">{x}</span>' for x in tags)
    return [f'<span style="display: flex; flex-direction: column; gap: 4px; padding: 8px 0"><span style="display: flex; align-items: center; gap: 8px; flex-wrap: wrap"><a href="#" style="font-weight: 600; color: var(--text)">{name}</a>{t}</span></span>',dns,cert,actions]
acts=lambda *b: '<span style="display: flex; gap: 6px; justify-content: flex-end; flex-wrap: wrap">'+''.join(b)+'</span>'
dtable=table('Domains for web',[('Domain','left'),('DNS','left'),('HTTPS','left'),('','right')],[
 dom_row('shop.example.com',['primary'],stat('run','points here'),stat('run','valid · renews in 61 d'),acts(btn('Edit','link'),btn('Remove','link'))),
 dom_row('www.shop.example.com',['redirects → shop.example.com'],stat('run','points here'),stat('run','valid · renews in 61 d'),acts(btn('Make primary','link'),btn('Remove','link'))),
 dom_row('shop.acme.co',['custom'],stat('warn','waiting for DNS'),stat('stop','not issued yet'),acts(btn('Check DNS','link'),btn('Remove','link'))),
 dom_row('web.srv-sgn-01.opsbox.internal',['private network'],stat('run','built in'),stat('run','internal CA'),acts(btn('Copy','link'))),
],minw=760)
def dns_line(typ,host,val,ok):
    s=stat('run','found') if ok else stat('warn','not found')
    return [f'<span style="{CODEF}; font-size: 13px">{typ}</span>',f'<span style="{CODEF}; font-size: 13px">{host}</span>',f'<span style="display: flex; align-items: center; gap: 8px"><span style="{CODEF}; font-size: 13px">{val}</span>{btn("Copy","link")}</span>',s]
dnsguide=f'''<div style="display: flex; flex-direction: column; gap: 14px">
<p style="margin: 0; font-size: 14px; line-height: 1.6; color: var(--muted)">Add this record at the company that manages <b style="color: var(--text)">acme.co</b> (Cloudflare, Route 53, Namecheap…). Changes usually show up within a few minutes, sometimes up to an hour.</p>
{table('DNS records to add',[('Type','left'),('Name','left'),('Value','left'),('Status','left')],[dns_line('A','shop','203.0.113.10',False),dns_line('AAAA','shop','2001:db8::10',True)],minw=620)}
<div style="display: flex; flex-wrap: wrap; align-items: center; gap: 12px">{btn('Check DNS now','primary')}<span style="font-size: 13px; color: var(--muted)">Checked 40 s ago · rechecks every minute · certificate is issued as soon as DNS points here</span></div>
<div role="note" style="display: flex; gap: 12px; padding: 12px 14px; border-radius: 12px; background: var(--fill); font-size: 13.5px; line-height: 1.55"><span style="color: var(--warn); font-weight: 700">!</span><span><b>Using Cloudflare?</b> Set the record to <b>DNS only</b> (grey cloud) until the certificate is issued, then you can turn the proxy back on.</span></div></div>'''
addform=f'''<div style="display: flex; flex-direction: column; gap: 16px">
{fld('nd','Domain',inp2('nd','api.acme.co'),'A whole domain (acme.co) or a subdomain (app.acme.co). Wildcards (*.acme.co) need a DNS provider token.')}
{fld('np','Send traffic to',sel('np',['web · port 3000','api · port 8080'],'web · port 3000'))}
{fld('nr','www',sel('nr',['Redirect www to the bare domain','Redirect the bare domain to www','Serve both','No www'],'Redirect www to the bare domain'))}
<div style="display: flex; flex-direction: column; gap: 10px">{chk('nh',True,'Force HTTPS (redirect http:// to https://)')}{chk('ns',True,'HSTS, so browsers always use HTTPS')}</div>
<div>{btn('Add domain','primary')}</div></div>'''
certs=f'''<div style="display: flex; flex-direction: column; gap: 10px; font-size: 14px">
<div style="display: flex; justify-content: space-between; gap: 12px; padding: 10px 0; border-bottom: 1px solid var(--line)"><span style="color: var(--muted)">Issuer</span><span>Let's Encrypt (automatic)</span></div>
<div style="display: flex; justify-content: space-between; gap: 12px; padding: 10px 0; border-bottom: 1px solid var(--line)"><span style="color: var(--muted)">Renewal</span><span>30 days before expiry</span></div>
<div style="display: flex; justify-content: space-between; gap: 12px; padding: 10px 0; border-bottom: 1px solid var(--line)"><span style="color: var(--muted)">Alert if renewal fails</span><span>Slack + email</span></div>
<div style="display: flex; flex-wrap: wrap; gap: 8px; padding-top: 6px">{btn('Upload own certificate')}{btn('Renew now','link')}</div></div>'''
svc_d=tabs(['Deployments','Variables','Domains & SSL','Metrics','Settings'],2)+f'''<div style="display: flex; flex-direction: column; gap: 32px">
<div style="display: flex; flex-direction: column; gap: 12px">{dtable}</div>
<div style="display: flex; flex-wrap: wrap; gap: 40px; align-items: flex-start">
<div style="flex: 3 1 520px; min-width: 0; display: flex; flex-direction: column; gap: 12px"><h3 style="margin: 0; font-size: 15px; font-weight: 600">Finish setting up shop.acme.co</h3>{dnsguide}</div>
<div style="flex: 2 1 320px; min-width: 0; display: flex; flex-direction: column; gap: 32px">
<div style="display: flex; flex-direction: column; gap: 14px"><h3 style="margin: 0; font-size: 15px; font-weight: 600">Add a domain</h3>{addform}</div>
<div style="display: flex; flex-direction: column; gap: 6px"><h3 style="margin: 0; font-size: 15px; font-weight: 600">Certificates</h3>{certs}</div>
</div></div></div>'''
main=hdr('shop-platform','5 services · srv-sgn-01 · environment production ▾',btn('+ Database')+btn('Deploy from GitHub','primary',href='NewService.dc.html'))+section('web',svc_d,btn('Redeploy')+btn('Open site','link'))
W['AppDomains']=page('Apps · web · Domains and SSL','apps',main,h=1400)
