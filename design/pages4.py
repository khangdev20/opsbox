# ---------- Landing page (design only) ----------
FM="font-family: 'Geist Mono', ui-monospace, monospace"
def ic(path,size=20,color='currentColor'):
    return f'<svg width="{size}" height="{size}" viewBox="0 0 24 24" fill="none" stroke="{color}" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{path}</svg>'
I={
 'deploy':'<path d="M12 3l9 5-9 5-9-5 9-5z"/><path d="M3 13l9 5 9-5"/>',
 'term':'<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3M12 15h5"/>',
 'shield':'<path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6l8-3z"/><path d="M9 12l2 2 4-4"/>',
 'db':'<ellipse cx="12" cy="6" rx="8" ry="3"/><path d="M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6"/>',
 'bell':'<path d="M6 9a6 6 0 0112 0c0 6 2 7 2 7H4s2-1 2-7z"/><path d="M10 20a2 2 0 004 0"/>',
 'box':'<path d="M21 8l-9-5-9 5v8l9 5 9-5V8z"/><path d="M3 8l9 5 9-5M12 13v8"/>',
 'chart':'<path d="M3 3v18h18"/><path d="M7 15l4-4 3 3 5-6"/>',
 'folder':'<path d="M3 6a2 2 0 012-2h4l2 2h8a2 2 0 012 2v10a2 2 0 01-2 2H5a2 2 0 01-2-2V6z"/>',
 'net':'<circle cx="5" cy="12" r="2"/><circle cx="19" cy="5" r="2"/><circle cx="19" cy="19" r="2"/><path d="M7 11l10-5M7 13l10 5"/>',
 'gh':'<path d="M9 19c-4 1.5-4-2-6-2.5M15 21v-3.5a3 3 0 00-.9-2.4c3-.3 6-1.5 6-6.6a5 5 0 00-1.4-3.6 4.6 4.6 0 00-.1-3.5s-1.1-.3-3.6 1.4a12.4 12.4 0 00-6.5 0C6 1.1 4.9 1.4 4.9 1.4a4.6 4.6 0 00-.1 3.5A5 5 0 003.4 8.5c0 5.1 3 6.3 6 6.6a3 3 0 00-.9 2.3V21"/>',
 'copy':'<rect x="9" y="9" width="12" height="12" rx="2"/><path d="M5 15V5a2 2 0 012-2h10"/>',
 'laptop':'<rect x="4" y="5" width="16" height="11" rx="2"/><path d="M2 19h20"/>',
 'globe':'<circle cx="12" cy="12" r="9"/><path d="M3 12h18M12 3a14 14 0 010 18M12 3a14 14 0 000 18"/>',
}
def L_btn(label,kind='default',href='#',icon=None):
    i=ic(I[icon],18) if icon else ''
    st={'primary':'background: var(--accent); color: var(--onaccent); font-weight: 600; box-shadow: 0 1px 0 rgba(255,255,255,.25) inset, 0 10px 24px -10px var(--accent);',
        'default':'background: var(--fill); color: var(--text); font-weight: 500;'}[kind]
    return f'<a href="{href}" style="display: inline-flex; align-items: center; gap: 10px; min-height: 48px; padding: 0 22px; border-radius: 12px; {st} font-size: 15px">{i}{label}</a>'
def L_sec(eyebrow,title,sub,body,id_=''):
    return f'''<section {f'id="{id_}"' if id_ else ''} style="width: 100%; max-width: 1160px; margin: 0 auto; padding: 96px 32px 0; box-sizing: border-box; display: flex; flex-direction: column; gap: 40px">
<div style="display: flex; flex-direction: column; gap: 14px; max-width: 680px">
<span style="{FM}; font-size: 13px; color: var(--accent); letter-spacing: .04em">{eyebrow}</span>
<h2 style="margin: 0; font-size: 40px; line-height: 1.1; font-weight: 600; letter-spacing: -0.035em">{title}</h2>
<p style="margin: 0; font-size: 17px; line-height: 1.6; color: var(--muted)">{sub}</p></div>
{body}</section>'''
def card(icon,title,text):
    return f'''<div style="background: var(--hl); border: 1px solid var(--line); border-radius: 18px; box-shadow: var(--shadow); padding: 26px; display: flex; flex-direction: column; gap: 12px">
<span style="width: 40px; height: 40px; border-radius: 11px; background: color-mix(in srgb, var(--accent) 14%, transparent); color: var(--accent); display: flex; align-items: center; justify-content: center">{ic(I[icon])}</span>
<h3 style="margin: 4px 0 0; font-size: 17px; font-weight: 600; letter-spacing: -0.02em">{title}</h3>
<p style="margin: 0; font-size: 14.5px; line-height: 1.6; color: var(--muted)">{text}</p></div>'''

nav_l=f'''<header style="position: sticky; top: 0; z-index: 30; background: color-mix(in srgb, var(--bg) 80%, transparent); backdrop-filter: saturate(1.4) blur(12px); -webkit-backdrop-filter: saturate(1.4) blur(12px); border-bottom: 1px solid var(--line)">
<div style="max-width: 1160px; margin: 0 auto; padding: 0 32px; min-height: 64px; box-sizing: border-box; display: flex; flex-wrap: wrap; align-items: center; gap: 8px 28px">
<a href="#top" style="font: 700 19px 'Geist', sans-serif; letter-spacing: -0.04em; color: var(--text)">opsbox<span style="color: var(--accent)">_</span></a>
<nav aria-label="Landing" style="display: flex; flex-wrap: wrap; gap: 4px 24px; font-size: 14.5px">
<a href="#features" style="color: var(--muted)">Features</a><a href="#how" style="color: var(--muted)">How it works</a><a href="#security" style="color: var(--muted)">Security</a><a href="#faq" style="color: var(--muted)">FAQ</a><a href="#docs" style="color: var(--muted)">Docs</a></nav>
<span style="flex: 1"></span>
<button type="button" onClick="{{{{toggleTheme}}}}" aria-label="Switch theme" style="width: 40px; height: 40px; border: 0; border-radius: 10px; background: var(--fill); color: var(--text); display: flex; align-items: center; justify-content: center; cursor: pointer">{ic('<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',18)}</button>
<a href="#github" style="display: inline-flex; align-items: center; gap: 8px; min-height: 40px; padding: 0 14px; border-radius: 10px; background: var(--fill); color: var(--text); font-size: 14px; font-weight: 500">{ic(I['gh'],18)}GitHub</a>
<a href="#install" style="display: inline-flex; align-items: center; min-height: 40px; padding: 0 16px; border-radius: 10px; background: var(--text); color: var(--bg); font-size: 14px; font-weight: 600">Install</a>
</div></header>'''

# product preview window
def mini_kpi(l,v,u,p):
    return f'<div style="background: var(--hl); border: 1px solid var(--line); border-radius: 12px; padding: 14px; display: flex; flex-direction: column; gap: 8px"><span style="font-size: 12px; color: var(--muted)">{l}</span><span style="font-size: 26px; font-weight: 600; letter-spacing: -0.04em">{v}<span style="font-size: 13px; color: var(--muted); font-weight: 500">{u}</span></span><span style="height: 5px; border-radius: 99px; background: var(--fill2); overflow: hidden"><span style="display: block; width: {p}%; height: 5px; border-radius: 99px; background: var(--accent)"></span></span></div>'
def mini_node(x,y,name,s,sel=False):
    b='1px solid var(--accent); box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent)' if sel else '1px solid var(--line)'
    return f'<div class="topo-node" style="animation-delay: {(x+y)/1500:.2f}s; position: absolute; left: {x}px; top: {y}px; width: 150px; padding: 10px 12px; box-sizing: border-box; border: {b}; border-radius: 12px; background: var(--hl); display: flex; flex-direction: column; gap: 6px"><b style="font-size: 13px">{name}</b>{s}</div>'
def mini_ln(x,y,w,h,d=0): return f'<div class="topo-edge {"v" if h>w else "h"}" style="position: absolute; left: {x}px; top: {y}px; width: {w}px; height: {max(h,2)}px; animation-delay: -{d}s"></div>'
side_items=''.join(f'<span style="display: flex; align-items: center; gap: 10px; padding: 7px 10px; border-radius: 8px; font-size: 13px; {"background: var(--fill2); color: var(--text); font-weight: 600" if n=="Apps" else "color: var(--muted)"}">{ic(I[k],16)}{n}</span>' for n,k in [('Overview','chart'),('Apps','deploy'),('Containers','box'),('Backups','db'),('Alerts','bell'),('Terminal','term'),('Files','folder'),('Security','shield')])
preview=f'''<figure role="img" aria-label="opsbox panel showing services and server metrics" style="margin: 0; width: 100%; max-width: 1080px; border-radius: 20px; border: 1px solid var(--line); background: var(--bg); box-shadow: 0 40px 120px -40px rgba(0,0,0,.55), 0 0 0 1px rgba(255,255,255,.03); overflow: hidden; text-align: left">
<div style="display: flex; align-items: center; gap: 8px; padding: 12px 16px; border-bottom: 1px solid var(--line); background: var(--hl)"><span style="width: 11px; height: 11px; border-radius: 50%; background: var(--ls)"></span><span style="width: 11px; height: 11px; border-radius: 50%; background: var(--ls)"></span><span style="width: 11px; height: 11px; border-radius: 50%; background: var(--ls)"></span><span style="flex: 1; text-align: center; font-size: 12.5px; color: var(--muted)">srv-sgn-01 / shop-platform / production</span></div>
<div style="display: flex; min-height: 420px">
<div style="width: 180px; flex: none; border-right: 1px solid var(--line); padding: 14px 10px; display: flex; flex-direction: column; gap: 2px">{side_items}</div>
<div style="flex: 1; min-width: 0; padding: 22px; display: flex; flex-direction: column; gap: 18px; overflow: hidden">
<div style="display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 10px">{mini_kpi('CPU','25','%',25)}{mini_kpi('Memory','3.1',' / 8 GB',39)}{mini_kpi('Disk','98',' / 120 GB',82)}{mini_kpi('Network','4.2',' MB/s',30)}</div>
<div aria-label="Service topology" style="position: relative; height: 210px; min-width: 640px">
{mini_ln(150,104,40,1)}{mini_ln(190,40,1,64,.3)}{mini_ln(190,40,60,1,.6)}{mini_ln(190,104,1,64,.4)}{mini_ln(190,168,60,1,.8)}{mini_ln(400,40,90,1,.2)}{mini_ln(400,168,90,1,.9)}
{mini_node(0,76,'web',stat('run','live'),True)}{mini_node(250,12,'api',stat('build','building'))}{mini_node(250,140,'worker',stat('run','running'))}{mini_node(490,12,'postgres',stat('run','running'))}{mini_node(490,140,'redis',stat('run','running'))}
</div></div></div></figure>'''

install_box=f'''<div id="install" style="display: flex; flex-wrap: wrap; align-items: center; gap: 12px; padding: 8px 8px 8px 18px; border-radius: 14px; background: #111113; border: 1px solid #2E2E33; color: #DADADF; max-width: 640px; width: 100%; box-sizing: border-box; text-align: left">
<span style="color: #8A8A92; {FM}; font-size: 14px">$</span><code style="flex: 1 1 360px; {FM}; font-size: 14px; white-space: nowrap; overflow-x: auto">curl -fsSL [INSTALL URL]/install.sh | sh</code>
<button type="button" aria-label="Copy install command" style="display: inline-flex; align-items: center; gap: 8px; min-height: 40px; padding: 0 14px; border: 0; border-radius: 10px; background: rgba(255,255,255,.08); color: #EEEEF0; font: 500 13.5px 'Geist', sans-serif; cursor: pointer">{ic(I['copy'],16)}Copy</button></div>'''

hero=f'''<section id="top" style="position: relative; overflow: hidden; padding: 88px 32px 0">
<div aria-hidden="true" style="position: absolute; inset: -200px 0 auto; height: 700px; background: radial-gradient(600px 300px at 50% 30%, color-mix(in srgb, var(--accent) 16%, transparent), transparent 70%); pointer-events: none"></div>
<div style="position: relative; max-width: 1160px; margin: 0 auto; display: flex; flex-direction: column; align-items: center; text-align: center; gap: 26px">
<a href="#github" style="display: inline-flex; align-items: center; gap: 10px; padding: 6px 14px 6px 6px; border-radius: 99px; background: var(--fill); font-size: 13.5px; color: var(--muted)"><span style="padding: 3px 10px; border-radius: 99px; background: var(--accent); color: var(--onaccent); font-weight: 600; font-size: 12px">Alpha</span>Open source · Apache-2.0 · free forever</a>
<h1 style="margin: 0; max-width: 900px; font-size: 68px; line-height: 1.02; font-weight: 600; letter-spacing: -0.05em">Your own Railway,<br>on any Linux server.</h1>
<p style="margin: 0; max-width: 640px; font-size: 19px; line-height: 1.6; color: var(--muted)">Install once. Deploy from GitHub, watch the machine, open a terminal, back up your data. After that, you never need to SSH into it again.</p>
{install_box}
<div style="display: flex; flex-wrap: wrap; justify-content: center; gap: 12px">{L_btn('Download the desktop app','primary',icon='laptop')}{L_btn('Star on GitHub',icon='gh')}</div>
<div style="display: flex; flex-wrap: wrap; justify-content: center; gap: 8px 10px; font-size: 13.5px; color: var(--muted)"><span style="display: inline-flex; align-items: center; gap: 6px; padding: 5px 12px; border-radius: 99px; background: color-mix(in srgb, var(--accent) 12%, transparent); color: var(--text); font-weight: 500"><span style="color: var(--accent); display: flex"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2"/></svg></span>Written in Rust</span><span style="padding: 5px 12px; border-radius: 99px; background: var(--fill)">One binary, no runtime</span><span style="padding: 5px 12px; border-radius: 99px; background: var(--fill)">About [RAM] MB of RAM idle</span><span style="padding: 5px 12px; border-radius: 99px; background: var(--fill)">x86_64 and arm64</span></div><p style="margin: 0; font-size: 13px; color: var(--muted)">Ubuntu, Debian, Amazon Linux, Fedora and any Linux with systemd</p>
<div style="width: 100%; display: flex; justify-content: center; padding-top: 28px; overflow-x: auto">{preview}</div>
</div></section>'''

features=L_sec('FEATURES','Everything you used to SSH in for.','One panel for the apps you ship and the machine they run on.',
 '<div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 16px">'+
 card('deploy','Deploy from GitHub','Pick a repo, opsbox detects how to build it. Auto-deploy on push, custom domains with HTTPS, one-click rollback.')+
 card('db','Databases in one click','PostgreSQL, MySQL, Redis, MongoDB and more, with daily backups on by default. Or paste your own Compose file.')+
 card('box','Many apps, one server','Run as many services as the machine can hold, each isolated, each with its own domain, logs and variables.')+
 card('chart','Monitoring that tells you what to do','CPU, memory, disk and network live, plus a short list of what needs your attention and the button to fix it.')+
 card('bell','Alerts before it breaks','Email, Slack, Telegram or webhook when the disk fills, a service stops or a backup fails. Quiet hours included.')+
 card('term','Terminal and files in the browser','A real shell, a file manager and a code editor built on the same engine as VS Code.')+
 '</div>','features')

def step(n,t,txt,codeline=''):
    c=f'<code style="display: block; margin-top: 4px; padding: 12px 14px; border-radius: 10px; background: #111113; border: 1px solid #2E2E33; color: #DADADF; {FM}; font-size: 13px; overflow-x: auto; white-space: nowrap">{codeline}</code>' if codeline else ''
    return f'<div style="display: flex; flex-direction: column; gap: 10px; padding: 26px; border-radius: 18px; background: var(--hl); border: 1px solid var(--line); box-shadow: var(--shadow)"><span style="width: 32px; height: 32px; border-radius: 50%; background: var(--fill2); display: flex; align-items: center; justify-content: center; {FM}; font-size: 14px; font-weight: 600">{n}</span><h3 style="margin: 6px 0 0; font-size: 18px; font-weight: 600; letter-spacing: -0.02em">{t}</h3><p style="margin: 0; font-size: 14.5px; line-height: 1.6; color: var(--muted)">{txt}</p>{c}</div>'
how=L_sec('HOW IT WORKS','From a fresh instance to a live app in minutes.','No Kubernetes, no YAML to learn, no separate control server to pay for.',
 '<div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(300px, 1fr)); gap: 16px">'+
 step('1','Install the agent','Run one command, or paste it into your cloud provider\'s user data when you launch the instance.','curl -fsSL [INSTALL URL]/install.sh | sh')+
 step('2','Pair the desktop app','The agent prints a pairing code. Enter it in the app and the server joins your private network.','pairing code  K7QX-29MD')+
 step('3','Ship your first app','Connect GitHub, pick a repo, add a domain. Every push to main deploys by itself.')+'</div>','how')

def two_col(icon,t,txt,points):
    li=''.join(f'<li style="display: flex; gap: 10px; align-items: flex-start"><span style="color: var(--ok); font-weight: 700">✓</span><span>{p}</span></li>' for p in points)
    return f'<div style="flex: 1 1 420px; padding: 30px; border-radius: 20px; background: var(--hl); border: 1px solid var(--line); box-shadow: var(--shadow); display: flex; flex-direction: column; gap: 14px"><span style="width: 44px; height: 44px; border-radius: 12px; background: var(--fill2); display: flex; align-items: center; justify-content: center">{ic(I[icon],22)}</span><h3 style="margin: 0; font-size: 22px; font-weight: 600; letter-spacing: -0.03em">{t}</h3><p style="margin: 0; font-size: 15px; line-height: 1.6; color: var(--muted)">{txt}</p><ul style="margin: 4px 0 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 10px; font-size: 14.5px">{li}</ul></div>'
ways=L_sec('TWO WAYS IN','A desktop app for your fleet. A web panel when you want one.','Both use the same interface and the same agent on each server.',
 '<div style="display: flex; flex-wrap: wrap; gap: 16px">'+
 two_col('laptop','Desktop app','The main way in. Manage every server from one window, over a private encrypted network.',['All your servers, one switcher','Reaches servers over WireGuard, nothing exposed to the internet','Network rules like a cloud security group','macOS, Windows and Linux'])+
 two_col('globe','Web panel (optional)','Turn it on for a single server when you want to reach it from any browser.',['Manages only the server it runs on','Your own domain with automatic HTTPS','Two-factor sign-in and IP allowlist','Off by default'])+'</div>','ways')

def sec_item(t,txt):
    return f'<div style="display: flex; gap: 14px; padding: 20px 0; border-bottom: 1px solid var(--line)"><span style="flex: none; width: 24px; height: 24px; border-radius: 50%; background: color-mix(in srgb, var(--ok) 16%, transparent); color: var(--ok); display: flex; align-items: center; justify-content: center; font-size: 12px; font-weight: 700">✓</span><div style="display: flex; flex-direction: column; gap: 4px"><b style="font-size: 15.5px; font-weight: 600">{t}</b><span style="font-size: 14.5px; line-height: 1.6; color: var(--muted)">{txt}</span></div></div>'
security=L_sec('SECURITY','A panel with root access should be hard to reach.','So by default, it is not reachable from the internet at all.',
 '<div style="display: flex; flex-wrap: wrap; gap: 48px"><div style="flex: 1 1 460px">'+
 sec_item('Private network by default','The agent only listens on an encrypted WireGuard network between your devices and servers.')+
 sec_item('Two-factor and lockout','TOTP on every account, sign-in lockout after repeated failures, every action in an audit log.')+
 sec_item('Signed releases','The installer verifies checksums and signatures before anything runs.')+
 sec_item('Memory-safe by design','The agent is written in Rust: no garbage-collected runtime to install, and a whole class of memory bugs ruled out by the compiler.')+
 '</div><div style="flex: 1 1 460px">'+
 sec_item('Safe mode','A separate watchdog restarts opsbox, rolls back a bad update and can open SSH for an hour if you need it.')+
 sec_item('No telemetry','opsbox sends nothing home. Your servers talk only to you.')+
 sec_item('Open source','Read every line. Report issues privately through SECURITY.md.')+
 '</div></div>','security')

def faq(q,a):
    return f'<details style="padding: 20px 0; border-bottom: 1px solid var(--line)"><summary style="cursor: pointer; font-size: 16px; font-weight: 600; letter-spacing: -0.01em">{q}</summary><p style="margin: 12px 0 0; font-size: 15px; line-height: 1.65; color: var(--muted); max-width: 760px">{a}</p></details>'
faqs=L_sec('FAQ','Questions, answered.','',
 '<div style="max-width: 860px">'+
 faq('Is it really free?','Yes. opsbox is open source under Apache-2.0. There is no paid tier and no account to create. You pay only for your own servers.')+
 faq('Is it ready for production?','Not yet. opsbox is in alpha. Use it for side projects and staging first, and keep backups somewhere off the server.')+
 faq('Which servers does it run on?','Any recent Linux with systemd: Ubuntu, Debian, Amazon Linux, Fedora and similar, on x86_64 or arm64. Any cloud or a box under your desk.')+
 faq('How is it different from Coolify or Dokploy?','opsbox manages the server itself as well as your apps (updates, firewall, users, disk, files) and keeps the panel off the public internet by default.')+
 faq('Can I manage more than one server?','Yes, with the desktop app. Each server runs a small agent and joins your private network.')+
 faq('What happens if opsbox itself breaks?','Your apps keep running. A watchdog restarts or rolls back opsbox, and the command line on the server can repair it.')+
 '</div>','faq')

cta=f'''<section style="max-width: 1160px; margin: 96px auto 0; padding: 0 32px; box-sizing: border-box"><div style="position: relative; overflow: hidden; border-radius: 28px; border: 1px solid var(--line); background: var(--hl); padding: 64px 32px; display: flex; flex-direction: column; align-items: center; text-align: center; gap: 20px">
<div aria-hidden="true" style="position: absolute; inset: 0; background: radial-gradient(500px 220px at 50% 0%, color-mix(in srgb, var(--accent) 18%, transparent), transparent 70%); pointer-events: none"></div>
<h2 style="position: relative; margin: 0; font-size: 44px; line-height: 1.08; font-weight: 600; letter-spacing: -0.04em">Stop SSHing into your servers.</h2>
<p style="position: relative; margin: 0; font-size: 17px; color: var(--muted)">One command, then everything else from a window.</p>
<div style="position: relative; width: 100%; display: flex; justify-content: center">{install_box.replace(' id="install"','')}</div>
<div style="position: relative; display: flex; flex-wrap: wrap; justify-content: center; gap: 12px">{L_btn('Download the desktop app','primary',icon='laptop')}{L_btn('Read the docs')}</div></div></section>'''

foot=f'''<footer style="max-width: 1160px; margin: 96px auto 0; padding: 32px 32px 48px; box-sizing: border-box; border-top: 1px solid var(--line); display: flex; flex-wrap: wrap; gap: 24px 48px; align-items: flex-start; font-size: 14px">
<div style="flex: 1 1 260px; display: flex; flex-direction: column; gap: 10px"><span style="font: 700 18px 'Geist', sans-serif; letter-spacing: -0.04em">opsbox<span style="color: var(--accent)">_</span></span><span style="color: var(--muted)">Self-hosted server management. Apache-2.0.</span></div>
<div style="display: flex; flex-direction: column; gap: 10px"><b style="font-weight: 600">Product</b><a href="#features" style="color: var(--muted)">Features</a><a href="#security" style="color: var(--muted)">Security</a><a href="#install" style="color: var(--muted)">Install</a></div>
<div style="display: flex; flex-direction: column; gap: 10px"><b style="font-weight: 600">Project</b><a href="#github" style="color: var(--muted)">GitHub</a><a href="#docs" style="color: var(--muted)">Docs</a><a href="#roadmap" style="color: var(--muted)">Roadmap</a><a href="#security" style="color: var(--muted)">Report a vulnerability</a></div>
<div style="display: flex; flex-direction: column; gap: 10px"><b style="font-weight: 600">Community</b><a href="#discussions" style="color: var(--muted)">Discussions</a><a href="#changelog" style="color: var(--muted)">Changelog</a></div>
</footer>'''

body=f'<div data-theme="{{{{theme}}}}" style="min-height: 100vh; background: var(--bg); color: var(--text); {UI}">'+nav_l+'<main>'+hero+features+how+ways+security+faqs+cta+'</main>'+foot+'</div>'
W['Landing']=head('opsbox — Landing page',body,'',w=1440,h=5600)
