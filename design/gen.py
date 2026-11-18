import json, re

LOGO_URI='assets/logo-mark-128.png'
BRAND='#FC5801'
def logo(px=28,fs=19,href=None,extra=''):
    inner=(f'<img src="{LOGO_URI}" alt="" width="{px}" height="{px}" style="display: block; width: {px}px; height: {px}px; object-fit: contain">'
           f'<span style="font: 700 {fs}px \'Geist\', sans-serif; letter-spacing: -0.035em; color: var(--text)">Ops<span style="color: {BRAND}">Box</span></span>')
    st=f'display: inline-flex; align-items: center; gap: {max(6,px//4)}px; color: var(--text); text-decoration: none; {extra}'
    return f'<a href="{href}" aria-label="OpsBox home" style="{st}">{inner}</a>' if href else f'<span aria-label="OpsBox" style="{st}">{inner}</span>'
UI="font-family: 'Geist', system-ui, sans-serif"
MO="font-family: 'Geist', system-ui, sans-serif; font-variant-numeric: tabular-nums"
FONTS='<link href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600;700&family=Geist+Mono:wght@400;500&display=swap" rel="stylesheet">'
CSS='''body{margin:0;background:#17171A}
a{text-decoration:none}
[data-theme="dark"]{--fill:rgba(255,255,255,.06);--fill2:rgba(255,255,255,.10);--shadow:0 1px 0 rgba(255,255,255,.04) inset,0 8px 24px -12px rgba(0,0,0,.6);--bg:#17171A;--hl:#1F1F23;--input:#1A1A1D;--line:#2E2E33;--ls:#3E3E44;--ctl:#74747C;--text:#EEEEF0;--muted:#A3A3AB;--accent:#FF6A3D;--onaccent:#17171A;--ok:#5FD08A;--warn:#E8B64A;--danger:#FF8FB8;--code:#111113;--codetext:#DADADF}
[data-theme="light"]{--fill:rgba(16,16,20,.045);--fill2:rgba(16,16,20,.08);--shadow:0 1px 2px rgba(16,16,20,.04),0 8px 24px -16px rgba(16,16,20,.18);--bg:#F6F6F3;--hl:#FFFFFF;--input:#FFFFFF;--line:#E6E5E0;--ls:#D5D4CE;--ctl:#84837C;--text:#1A1A18;--muted:#5E5D58;--accent:#C9300F;--onaccent:#FFFFFF;--ok:#1F7A3D;--warn:#8A5A00;--danger:#9B1B5A;--code:#0A0A09;--codetext:#D9D9D3}'''
LOGIC='''class Component extends DCLogic {
  renderVals() {
    const theme = (this.state && this.state.theme) || this.props.theme || 'dark';
    return {
      theme,
      themeLabel: theme === 'light' ? 'Dark' : 'Light',
      toggleTheme: () => this.setState({ theme: theme === 'light' ? 'dark' : 'light' })
    };
  }
}'''
def head(title,body,props_extra='',w=1440,h=1100,theme='dark',logic=LOGIC):
    return f'''<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>{title}</title>
<script src="./support.js"></script>
</head>
<body>
<x-dc>
<helmet>
{FONTS}
<style>
{CSS}

select{{-webkit-appearance:none!important;appearance:none!important;background-image:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 24 24' fill='none' stroke='%23A3A3AB' stroke-width='2' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E")!important;background-repeat:no-repeat!important;background-position:right 12px center!important;background-size:16px 16px!important;padding-right:38px!important;cursor:pointer;transition:border-color .12s}}
select:hover{{border-color:var(--muted)!important}}
select:focus-visible{{outline:none;border-color:var(--accent)!important;box-shadow:0 0 0 3px color-mix(in srgb,var(--accent) 22%,transparent)}}
select option{{background:var(--hl);color:var(--text)}}
input[type=checkbox]{{-webkit-appearance:none!important;appearance:none!important;margin:0;flex:none;border-radius:7px!important;border:1.5px solid var(--ctl)!important;background:var(--input) no-repeat center/72% 72%!important;cursor:pointer;transition:background-color .18s cubic-bezier(.2,.8,.2,1),border-color .18s,box-shadow .18s,transform .12s}}
input[type=checkbox]:hover{{border-color:var(--muted)!important;box-shadow:0 0 0 4px var(--fill)}}
input[type=checkbox]:active{{transform:scale(.92)}}
input[type=checkbox]:checked{{background-color:var(--accent)!important;border-color:var(--accent)!important;background-image:url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='white' stroke-width='3' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M5 12.5l4.5 4.5L19 7.5'/%3E%3C/svg%3E")!important}}
input[type=checkbox]:checked:hover{{box-shadow:0 0 0 4px color-mix(in srgb,var(--accent) 18%,transparent)}}
input[type=checkbox]:focus-visible{{outline:none;box-shadow:0 0 0 3px color-mix(in srgb,var(--accent) 35%,transparent)}}
input[type=radio]{{-webkit-appearance:none!important;appearance:none!important;margin:0;flex:none;width:20px;height:20px;border-radius:999px!important;border:1.5px solid var(--ctl)!important;background:var(--input)!important;cursor:pointer;transition:border-color .18s,box-shadow .18s}}
input[type=radio]:checked{{border:6px solid var(--accent)!important}}
@media (prefers-reduced-motion:reduce){{input[type=checkbox],input[type=radio],select{{transition:none!important}}}}
@keyframes topo-in{{from{{opacity:0;transform:translateY(10px) scale(.98)}}to{{opacity:1;transform:none}}}}
@keyframes topo-flow-h{{to{{background-position:24px 0}}}}
@keyframes topo-flow-v{{to{{background-position:0 24px}}}}
@keyframes topo-pulse{{0%,100%{{opacity:1;transform:scale(1)}}50%{{opacity:.35;transform:scale(.7)}}}}
@keyframes topo-spin{{to{{transform:rotate(360deg)}}}}
@keyframes topo-glow{{0%,100%{{box-shadow:0 0 0 0 color-mix(in srgb,var(--accent) 35%,transparent)}}50%{{box-shadow:0 0 0 8px color-mix(in srgb,var(--accent) 0%,transparent)}}}}
.topo-node{{animation:topo-in .5s cubic-bezier(.2,.7,.2,1) both;transition:transform .2s,box-shadow .2s}}
.topo-node:hover{{transform:translateY(-2px);box-shadow:0 10px 24px -12px rgba(0,0,0,.35)}}
.topo-node.sel{{animation:topo-in .5s cubic-bezier(.2,.7,.2,1) both,topo-glow 2.4s ease-in-out .6s infinite}}
.topo-edge.h{{background:repeating-linear-gradient(90deg,var(--accent) 0 6px,transparent 6px 12px);opacity:.55;animation:topo-flow-h 1.2s linear infinite}}
.topo-edge.v{{background:repeating-linear-gradient(180deg,var(--accent) 0 6px,transparent 6px 12px);opacity:.55;animation:topo-flow-v 1.2s linear infinite}}
[aria-label="Service topology"] .st-run{{animation:topo-pulse 1.8s ease-in-out infinite}}
[aria-label="Service topology"] .st-build{{animation:topo-spin 1.2s linear infinite}}
@media (prefers-reduced-motion:reduce){{.topo-node,.topo-edge,.st-g{{animation:none!important}}}}
</style>
</helmet>
{body}
</x-dc>
<script type="text/x-dc" data-dc-script data-props='{{"theme":{{"editor":"enum","default":"{theme}","options":["dark","light"]}},{props_extra}"$preview":{{"width":{w},"height":{h}}}}}'>
{logic}
</script>
</body>
</html>
'''
def mono(t,c='var(--muted)',s=12,w=400,extra=''):
    return f'<span style="{MO}; font-size: {s}px; font-weight: {w}; color: {c}; {extra}">{t}</span>'
DANGER_RE=re.compile(r'^(Stop|Delete|Remove|Revoke|Lock|Disconnect|Uninstall|Reboot|Sign out all|\u2715)')
def btn(label,kind='default',href=None,aria=None):
    base="display: inline-flex; align-items: center; justify-content: center; min-height: 36px; padding: 0 14px; border-radius: 10px; font: 500 14px 'Geist', sans-serif; cursor: pointer; box-sizing: border-box; white-space: nowrap; "
    plain=re.sub('<[^>]+>','',label)
    if kind=='link':
        if DANGER_RE.match(plain): k="min-height: 30px; padding: 0 10px; font-size: 13px; border-radius: 8px; background: color-mix(in srgb, var(--danger) 10%, transparent); color: var(--danger); border: 1px solid transparent;"
        else: k="min-height: 30px; padding: 0 10px; font-size: 13px; border-radius: 8px; background: var(--fill); color: var(--text); border: 1px solid transparent;"
    else:
        k={'primary':"background: var(--accent); color: var(--onaccent); border: 1px solid var(--accent); font-weight: 600; box-shadow: 0 1px 0 rgba(255,255,255,.25) inset, 0 6px 16px -8px var(--accent);",
       'default':"background: var(--fill); color: var(--text); border: 1px solid transparent;",
       'danger':"background: color-mix(in srgb, var(--danger) 12%, transparent); color: var(--danger); border: 1px solid transparent;"}[kind]
    if href: return f'<a href="{href}" style="{base}{k}">{label}</a>'
    al=f' aria-label="{aria}"' if aria else ''
    return f'<button type="button"{al} style="{base}{k}">{label}</button>'
def stat(kind,text):
    g,c={'run':('\u25cf','var(--ok)'),'warn':('\u25b2','var(--warn)'),'fail':('\u2715','var(--danger)'),'stop':('\u25cb','var(--muted)'),'build':('\u25d0','var(--warn)')}[kind]
    return f'<span style="display: inline-flex; align-items: center; gap: 6px; padding: 3px 10px; border-radius: 999px; background: color-mix(in srgb, {c} 14%, transparent); color: {c}; font-size: 13px; font-weight: 600; white-space: nowrap"><span class="st-g st-{kind}" style="font-size: 10px; display: inline-block">{g}</span>{text}</span>'
def table(caption,heads,rows,minw=560):
    h=''.join(f'<th style="padding: 10px 16px 10px 0; text-align: {a}; font: 500 12px \'Geist\', sans-serif; color: var(--muted); text-transform: uppercase; letter-spacing: .05em; border-bottom: 1px solid var(--ls)">{t}</th>' for t,a in heads)
    r=''
    for row in rows:
        hl=False
        if isinstance(row,dict): hl=row.get('hl'); row=row['cells']
        bg=' background: var(--hl);' if hl else ''
        cells=''
        for i,c in enumerate(row):
            a=heads[i][1]
            cells+=f'<td style="height: 56px; padding: 0 20px 0 0; text-align: {a}; border-bottom: 1px solid var(--line); font-size: 15px;{bg}{' font-weight: 500;' if i==0 else ''}">{c}</td>'
        r+=f'<tr>{cells}</tr>'
    return f'<div style="overflow-x: auto"><table style="width: 100%; border-collapse: collapse; min-width: {minw}px"><caption style="position: absolute; width: 1px; height: 1px; overflow: hidden">{caption}</caption><thead><tr>{h}</tr></thead><tbody>{r}</tbody></table></div>'
def section(title,body,right='',grow=''):
    return f'<section style="{grow}min-width: 0; background: var(--hl); border: 1px solid var(--line); border-radius: 16px; box-shadow: var(--shadow); padding: 24px 28px 28px; display: flex; flex-direction: column; gap: 20px"><div style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 8px; min-height: 36px"><h2 style="margin: 0; font-size: 16px; font-weight: 600; letter-spacing: -0.02em">{title}</h2>{right}</div>{body}</section>'
def hdr(title,meta,right=''):
    return f'<div style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: flex-end; gap: 16px"><div style="display: flex; flex-direction: column; gap: 6px"><h1 style="margin: 0; font-size: 28px; font-weight: 600; letter-spacing: -0.03em; line-height: 1.15">{title}</h1><span style="{MO}; font-size: 14px; color: var(--muted)">{meta}</span></div><div style="display: flex; flex-wrap: wrap; gap: 10px; align-items: center">{right}</div></div>'
def sw(color,label):
    return f'<span style="display: inline-flex; align-items: center; gap: 6px; {MO}; font-size: 13px; color: var(--muted)"><span style="width: 10px; height: 10px; background: {color}; display: inline-block"></span>{label}</span>'
def tabs(items,active):
    out=''
    for i,t in enumerate(items):
        on=(i==active)
        st_=("background: var(--hl); color: var(--text); box-shadow: 0 1px 2px rgba(0,0,0,.18);" if on else "background: transparent; color: var(--muted);")
        out+=f'<button type="button" role="tab" aria-selected="{"true" if on else "false"}" style="min-height: 32px; padding: 0 14px; border: 0; border-radius: 9px; {st_} font: 500 14px \'Geist\', sans-serif; cursor: pointer">{t}</button>'
    return f'<div role="tablist" style="display: inline-flex; flex-wrap: wrap; gap: 2px; padding: 4px; border-radius: 12px; background: var(--fill); align-self: flex-start">{out}</div>'
def code(text,minh=0):
    return f'<pre style="margin: 0; padding: 16px; background: var(--code); color: var(--codetext); font: 12px/1.75 \'Geist Mono\', monospace; overflow-x: auto; white-space: pre; border-radius: 8px; min-height: {minh}px">{text}</pre>'
def page(title,active,main,**kw):
    body=f'''<div data-theme="{{{{theme}}}}" style="min-height: 100vh; display: flex; flex-direction: column; background: var(--bg); color: var(--text); {UI}">
<div style="position: sticky; top: 0; z-index: 30"><dc-import name="Header" theme="{{{{theme}}}}" toggle-theme="{{{{toggleTheme}}}}" hint-size="100%,64px"></dc-import></div>
<div style="flex: 1; display: flex; align-items: stretch; min-height: 0">
<div style="position: sticky; top: 64px; align-self: flex-start; height: calc(100vh - 64px); overflow-y: auto; overscroll-behavior: contain; flex: none"><dc-import name="Nav" active="{active}" theme="{{{{theme}}}}" hint-size="248px,100%"></dc-import></div>
<div style="flex: 1; min-width: 0; display: flex; flex-direction: column">
<main style="flex: 1; width: 100%; max-width: 1180px; margin: 0 auto; box-sizing: border-box; padding: 40px 40px 64px; display: flex; flex-direction: column; gap: 32px">
{main}
</main>
<dc-import name="Status" theme="{{{{theme}}}}" hint-size="100%,44px"></dc-import>
</div></div></div>'''
    return head(f'opsbox — {title}',body,'' ,**kw)
def steps(vals,w=640,h=180):
    n=len(vals);dx=w/(n-1)
    ys=[round(h-v/100*h,1) for v in vals]
    d=f'M0,{ys[0]}'
    for i in range(1,n): d+=f' H{round(i*dx,1)} V{ys[i]}'
    return d
W={}
# ---------- Nav ----------
nav_items=[('01','Overview','Main.dc.html','overview'),('02','Apps','Apps.dc.html','apps'),('03','Containers','Containers.dc.html','containers'),('04','System','System.dc.html','system'),('05','Backups','Backups.dc.html','backups'),('06','Alerts','Alerts.dc.html','alerts'),('07','Cleanup','Cleanup.dc.html','cleanup'),('08','Terminal','Terminal.dc.html','terminal'),('09','Files','Files.dc.html','files'),('10','Security','Security.dc.html','security')]
navbody=f'''<div data-theme="{{{{theme}}}}" style="width: {{{{navW}}}}; transition: width .2s ease; overflow: hidden; flex: none; min-height: 100%; box-sizing: border-box; background: var(--bg); border-right: 1px solid var(--line); padding: 20px 14px; display: flex; flex-direction: column; gap: 2px; color: var(--text); {UI}">
<nav aria-label="Sections" style="display: flex; flex-direction: column; gap: 2px">
<sc-if value="{{{{on.overview}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Main.dc.html" aria-current="page" title="Overview" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Overview</span></a></sc-if>
<sc-if value="{{{{off.overview}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Main.dc.html" title="Overview" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Overview</span></a></sc-if>
<div aria-hidden="true" style="display: {{{{lbl}}}}; padding: 14px 12px 6px; font: 600 11px 'Geist', sans-serif; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); white-space: nowrap">Deploy</div>
<div aria-hidden="true" style="display: {{{{sep}}}}; height: 1px; background: var(--line); margin: 8px 10px"></div>
<sc-if value="{{{{on.apps}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Apps.dc.html" aria-current="page" title="Apps" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l9 5-9 5-9-5 9-5z"/><path d="M3 13l9 5 9-5"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Apps</span></a></sc-if>
<sc-if value="{{{{off.apps}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Apps.dc.html" title="Apps" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l9 5-9 5-9-5 9-5z"/><path d="M3 13l9 5 9-5"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Apps</span></a></sc-if>
<sc-if value="{{{{on.containers}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Containers.dc.html" aria-current="page" title="Containers" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 8l-9-5-9 5v8l9 5 9-5V8z"/><path d="M3 8l9 5 9-5M12 13v8"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Containers</span></a></sc-if>
<sc-if value="{{{{off.containers}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Containers.dc.html" title="Containers" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 8l-9-5-9 5v8l9 5 9-5V8z"/><path d="M3 8l9 5 9-5M12 13v8"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Containers</span></a></sc-if>
<div aria-hidden="true" style="display: {{{{lbl}}}}; padding: 14px 12px 6px; font: 600 11px 'Geist', sans-serif; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); white-space: nowrap">Server</div>
<div aria-hidden="true" style="display: {{{{sep}}}}; height: 1px; background: var(--line); margin: 8px 10px"></div>
<sc-if value="{{{{on.system}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="System.dc.html" aria-current="page" title="System" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">System</span></a></sc-if>
<sc-if value="{{{{off.system}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="System.dc.html" title="System" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">System</span></a></sc-if>
<sc-if value="{{{{on.terminal}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Terminal.dc.html" aria-current="page" title="Terminal" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3M12 15h5"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Terminal</span></a></sc-if>
<sc-if value="{{{{off.terminal}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Terminal.dc.html" title="Terminal" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3M12 15h5"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Terminal</span></a></sc-if>
<sc-if value="{{{{on.files}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Files.dc.html" aria-current="page" title="Files" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6a2 2 0 012-2h4l2 2h8a2 2 0 012 2v10a2 2 0 01-2 2H5a2 2 0 01-2-2V6z"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Files</span></a></sc-if>
<sc-if value="{{{{off.files}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Files.dc.html" title="Files" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M3 6a2 2 0 012-2h4l2 2h8a2 2 0 012 2v10a2 2 0 01-2 2H5a2 2 0 01-2-2V6z"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Files</span></a></sc-if>
<sc-if value="{{{{on.cleanup}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Cleanup.dc.html" aria-current="page" title="Cleanup" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/><path d="M10 11v6M14 11v6"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Cleanup</span></a></sc-if>
<sc-if value="{{{{off.cleanup}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Cleanup.dc.html" title="Cleanup" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/><path d="M10 11v6M14 11v6"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Cleanup</span></a></sc-if>
<div aria-hidden="true" style="display: {{{{lbl}}}}; padding: 14px 12px 6px; font: 600 11px 'Geist', sans-serif; letter-spacing: .08em; text-transform: uppercase; color: var(--muted); white-space: nowrap">Protect</div>
<div aria-hidden="true" style="display: {{{{sep}}}}; height: 1px; background: var(--line); margin: 8px 10px"></div>
<sc-if value="{{{{on.backups}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Backups.dc.html" aria-current="page" title="Backups" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><ellipse cx="12" cy="6" rx="8" ry="3"/><path d="M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Backups</span></a></sc-if>
<sc-if value="{{{{off.backups}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Backups.dc.html" title="Backups" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><ellipse cx="12" cy="6" rx="8" ry="3"/><path d="M4 6v6c0 1.7 3.6 3 8 3s8-1.3 8-3V6M4 12v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Backups</span></a></sc-if>
<sc-if value="{{{{on.alerts}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Alerts.dc.html" aria-current="page" title="Alerts" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 9a6 6 0 0112 0c0 6 2 7 2 7H4s2-1 2-7z"/><path d="M10 20a2 2 0 004 0"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Alerts</span></a></sc-if>
<sc-if value="{{{{off.alerts}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Alerts.dc.html" title="Alerts" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 9a6 6 0 0112 0c0 6 2 7 2 7H4s2-1 2-7z"/><path d="M10 20a2 2 0 004 0"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Alerts</span></a></sc-if>
<sc-if value="{{{{on.security}}}}" hint-placeholder-val="{{{{ false }}}}"><a href="Security.dc.html" aria-current="page" title="Security" style="position: relative; min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 10px; background: var(--fill2); color: var(--text); font-size: 15px; font-weight: 600"><span style="color: var(--accent); display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6l8-3z"/><path d="M9 12l2 2 4-4"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Security</span></a></sc-if>
<sc-if value="{{{{off.security}}}}" hint-placeholder-val="{{{{ true }}}}"><a href="Security.dc.html" title="Security" style="min-height: 42px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border-radius: 8px; color: var(--muted); font-size: 15px; font-weight: 500"><span style="display: flex"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6l8-3z"/><path d="M9 12l2 2 4-4"/></svg></span><span style="display: {{{{lbl}}}}; white-space: nowrap">Security</span></a></sc-if>
</nav>
<div style="flex: 1; min-height: 16px"></div>
<div style="border-top: 1px solid var(--line); padding-top: 10px">
<button type="button" onClick="{{{{toggleNav}}}}" aria-label="{{{{toggleLabel}}}}" title="{{{{toggleLabel}}}}" style="width: 100%; min-height: 40px; display: flex; align-items: center; gap: 12px; padding: 0 12px; border: 0; border-radius: 8px; background: transparent; color: var(--muted); font: 500 14px 'Geist', sans-serif; cursor: pointer"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" style="flex: none; transform: rotate({{{{chev}}}}); transition: transform .2s"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16M15 10l-2 2 2 2"/></svg><span style="display: {{{{lbl}}}}; white-space: nowrap">Collapse</span></button>
</div></div>'''
headerbody=f'''<div data-theme="{{{{theme}}}}" style="background: color-mix(in srgb, var(--bg) 80%, transparent); backdrop-filter: saturate(1.4) blur(12px); -webkit-backdrop-filter: saturate(1.4) blur(12px); border-bottom: 1px solid var(--line); color: var(--text); {UI}">
<div style="padding: 8px 32px; box-sizing: border-box; display: flex; flex-wrap: wrap; align-items: center; gap: 8px 20px; min-height: 64px">
{logo(30,19,'Main.dc.html','width: 216px')}
<span style="font-size: 14px; color: var(--muted)">srv-sgn-01 <span style="color: var(--ls)">/</span> shop-platform <span style="color: var(--ls)">/</span> <span style="color: var(--text)">production</span></span>
<span style="flex: 1"></span>
<button type="button" style="min-height: 40px; padding: 0 10px 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: transparent; color: var(--muted); font: 400 14px 'Geist', sans-serif; display: flex; align-items: center; gap: 28px; cursor: pointer">Search or run a command<span style="border: 1px solid var(--ctl); border-radius: 6px; padding: 1px 6px; font: 400 11px 'Geist Mono', monospace">Ctrl K</span></button>
<button type="button" onClick="{{{{toggleTheme}}}}" aria-label="{{{{themeAria}}}}" title="{{{{themeAria}}}}" style="width: 40px; height: 40px; padding: 0; border: 0; border-radius: 10px; background: var(--fill); color: var(--text); display: flex; align-items: center; justify-content: center; cursor: pointer"><sc-if value="{{{{isDark}}}}" hint-placeholder-val="{{{{ true }}}}"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/></svg></sc-if><sc-if value="{{{{isLight}}}}" hint-placeholder-val="{{{{ false }}}}"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.8A9 9 0 1111.2 3a7 7 0 009.8 9.8z"/></svg></sc-if></button>
<div style="position: relative">
<button type="button" onClick="{{{{toggleMenu}}}}" aria-haspopup="menu" aria-expanded="{{{{menuOpen}}}}" aria-label="Account menu for Minh Tran" style="min-height: 40px; display: flex; align-items: center; gap: 10px; padding: 0 8px 0 4px; border: 1px solid var(--line); border-radius: 999px; background: var(--fill); color: var(--text); font: 500 14px 'Geist', sans-serif; cursor: pointer">
<span aria-hidden="true" style="width: 30px; height: 30px; border-radius: 999px; background: color-mix(in srgb, var(--accent) 22%, transparent); color: var(--accent); display: flex; align-items: center; justify-content: center; font: 600 12px 'Geist', sans-serif">MT</span>
<span style="display: flex; flex-direction: column; align-items: flex-start; line-height: 1.15"><span>Minh Tran</span><span style="font-size: 12px; color: var(--muted)">Owner</span></span>
<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 9l6 6 6-6"/></svg>
</button>
<sc-if value="{{{{menuOpen}}}}" hint-placeholder-val="{{{{ false }}}}"><div role="menu" aria-label="Account" style="position: absolute; right: 0; top: calc(100% + 8px); z-index: 50; width: 240px; padding: 6px; border: 1px solid var(--line); border-radius: 12px; background: var(--hl); box-shadow: var(--shadow); display: flex; flex-direction: column; gap: 2px">
<div style="padding: 8px 10px 10px; border-bottom: 1px solid var(--line); margin-bottom: 4px"><div style="font-size: 14px; font-weight: 600">Minh Tran</div><div style="font-size: 13px; color: var(--muted)">minh.dev · Owner · 2FA on</div></div>
<a href="Access.dc.html" role="menuitem" style="min-height: 38px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border-radius: 8px; font-size: 14px; color: var(--text); "><span style="display: flex; color: var(--muted)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="8" r="4"/><path d="M4 21c0-4 4-6 8-6s8 2 8 6"/></svg></span>Profile and security</a>
<a href="Settings.dc.html" role="menuitem" style="min-height: 38px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border-radius: 8px; font-size: 14px; color: var(--text); "><span style="display: flex; color: var(--muted)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.6 1.6 0 00.3 1.8l.1.1a2 2 0 11-2.8 2.8l-.1-.1a1.6 1.6 0 00-1.8-.3 1.6 1.6 0 00-1 1.5V21a2 2 0 11-4 0v-.1a1.6 1.6 0 00-1-1.5 1.6 1.6 0 00-1.8.3l-.1.1a2 2 0 11-2.8-2.8l.1-.1a1.6 1.6 0 00.3-1.8 1.6 1.6 0 00-1.5-1H3a2 2 0 110-4h.1a1.6 1.6 0 001.5-1 1.6 1.6 0 00-.3-1.8l-.1-.1a2 2 0 112.8-2.8l.1.1a1.6 1.6 0 001.8.3H9a1.6 1.6 0 001-1.5V3a2 2 0 114 0v.1a1.6 1.6 0 001 1.5 1.6 1.6 0 001.8-.3l.1-.1a2 2 0 112.8 2.8l-.1.1a1.6 1.6 0 00-.3 1.8V9a1.6 1.6 0 001.5 1H21a2 2 0 110 4h-.1a1.6 1.6 0 00-1.5 1z"/></svg></span>Server settings</a>
<a href="Settings.dc.html" role="menuitem" style="min-height: 38px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border-radius: 8px; font-size: 14px; color: var(--text); "><span style="display: flex; color: var(--muted)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="3" y="11" width="18" height="10" rx="2"/><path d="M7 11V7a5 5 0 0110 0v4"/></svg></span>API tokens</a>
<div style="height: 1px; background: var(--line); margin: 4px 0"></div>
<a href="Login.dc.html" role="menuitem" style="min-height: 38px; display: flex; align-items: center; gap: 10px; padding: 0 10px; border-radius: 8px; font-size: 14px; color: var(--text); color: var(--danger)"><span style="display: flex; color: var(--muted)"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 21H5a2 2 0 01-2-2V5a2 2 0 012-2h4M16 17l5-5-5-5M21 12H9"/></svg></span>Sign out</a>
</div></sc-if>
</div>
</div></div>'''
defs=json.dumps([{'n':n,'label':l,'href':h,'id':i} for n,l,h,i in nav_items])
navlogic=f'''class Component extends DCLogic {{
  renderVals() {{
    const c = !!(this.state && this.state.collapsed);
    const a = this.props.active ?? 'overview';
    const theme = this.props.theme ?? 'dark';
    const t = this.props.toggleTheme;
    const defs = {defs};
    return {{
      theme,
      themeLabel: theme === 'light' ? 'Dark' : 'Light',
      toggleTheme: () => {{ if (typeof t === 'function') t(); }},
      items: defs.map((x) => ({{ ...x, active: x.id === a, inactive: x.id !== a }})),
      on: Object.fromEntries(defs.map((x) => [x.id, x.id === a])),
      off: Object.fromEntries(defs.map((x) => [x.id, x.id !== a])),
      navW: c ? '72px' : '248px',
      lbl: c ? 'none' : 'inline',
      sep: c ? 'block' : 'none',
      chev: c ? '180deg' : '0deg',
      toggleAlign: c ? 'center' : 'flex-end',
      toggleLabel: c ? 'Expand sidebar' : 'Collapse sidebar',
      toggleNav: () => this.setState({{ collapsed: !c }})
    }};
  }}
}}'''
hdrlogic='''class Component extends DCLogic {
  renderVals() {
    const theme = this.props.theme ?? 'dark';
    const t = this.props.toggleTheme;
    return { theme, isDark: theme !== 'light', isLight: theme === 'light', themeAria: theme === 'light' ? 'Switch to dark theme' : 'Switch to light theme', toggleTheme: () => { if (typeof t === 'function') t(); }, menuOpen: !!(this.state && this.state.menuOpen), toggleMenu: () => this.setState({ menuOpen: !(this.state && this.state.menuOpen) }) };
  }
}'''
W['Header']=head('opsbox — Top header',headerbody,'"toggleTheme":{"editor":null},',w=1440,h=64,logic=hdrlogic)
W['Nav']=head('opsbox — Sidebar navigation',navbody,'"active":{"editor":"enum","default":"overview","options":["overview","apps","containers","system","backups","alerts","cleanup","terminal","files","security","settings"]},"toggleTheme":{"editor":null},',w=248,h=700,logic=navlogic)
statusbody=f'''<footer data-theme="{{{{theme}}}}" style="background: var(--bg); border-top: 1px solid var(--ls); {MO}; font-size: 13px; color: var(--muted)">
<div style="max-width: 1280px; margin: 0 auto; padding: 8px 32px; box-sizing: border-box; display: flex; flex-wrap: wrap; align-items: center; gap: 4px 24px; min-height: 44px">
<span style="color: var(--ok)">■ srv-sgn-01</span><span>cpu <span style="color: var(--text)">23%</span></span><span>mem <span style="color: var(--text)">3.1/8.0 GB</span></span><span>disk / <span style="color: var(--warn)">82%</span></span><span>net <span style="color: var(--text)">↓4.2 ↑0.8 MB/s</span></span><span style="flex: 1"></span><span>sample data</span><span>opsbox 0.1.0</span>
</div></footer>'''
statuslogic='''class Component extends DCLogic {
  renderVals() { return { theme: this.props.theme ?? 'dark' }; }
}'''
W['Status']=head('opsbox — Status bar',statusbody,'',w=1440,h=44,logic=statuslogic)
# ---------- Overview ----------
cpu=[18,22,19,31,27,24,38,29,22,26,34,41,28,23,21,30,36,27,24,22,25,33,26,23]
mem=[38,38,38,39,39,38,39,39,39,38,39,39,39,39,39,39,38,39,39,39,39,39,39,39]
def kpi(label,val,unit,sub,pct,col='var(--text)',vc='var(--text)'):
    return f'''<div style="background: var(--hl); border: 1px solid var(--line); border-radius: 16px; box-shadow: var(--shadow); padding: 20px 22px; display: flex; flex-direction: column; gap: 12px"><span style="font-size: 14px; color: var(--muted)">{label}</span><span style="{MO}; font-size: 44px; font-weight: 500; letter-spacing: -0.04em; line-height: 1; color: {vc}">{val}<span style="font-size: 16px; color: var(--muted); letter-spacing: 0"> {unit}</span></span><div style="height: 4px; background: var(--line)"><div style="width: {pct}%; height: 4px; background: {col}"></div></div>{mono(sub)}</div>'''
strip=f'''<section aria-label="Key metrics" style="display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 24px">
{kpi('CPU · 8 cores','23','%','load 0.62 · 0.48 · 0.41',23)}
{kpi('Memory','3.1','GB','of 8.0 GB · cache 2.4 GB · swap 0',39)}
{kpi('Disk /','82','%','98 of 120 GB · 22 GB free',82,'var(--warn)','var(--warn)')}
{kpi('Network eth0','4.2','MB/s in','0.8 MB/s out · 142 connections',35)}
</section>'''
chart=f'''<div style="display: flex; gap: 12px"><div style="{MO}; font-size: 12px; color: var(--muted); display: flex; flex-direction: column; justify-content: space-between; height: 300px; text-align: right"><span>100%</span><span>50%</span><span>0</span></div><div style="flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 6px"><svg viewBox="0 0 640 180" preserveAspectRatio="none" role="img" aria-label="Stepped line chart: CPU between 18 and 41 percent, memory steady near 39 percent" style="width: 100%; height: 300px; display: block"><g style="stroke: var(--line)" stroke-width="1"><line vector-effect="non-scaling-stroke" x1="0" y1="0.5" x2="640" y2="0.5"/><line vector-effect="non-scaling-stroke" x1="0" y1="90" x2="640" y2="90"/><line vector-effect="non-scaling-stroke" x1="0" y1="179.5" x2="640" y2="179.5"/></g><path d="{steps(mem)}" fill="none" vector-effect="non-scaling-stroke" style="stroke: var(--muted)" stroke-width="1.5" stroke-dasharray="5 4"/><path d="{steps(cpu)}" fill="none" vector-effect="non-scaling-stroke" style="stroke: var(--text)" stroke-width="2"/></svg><div style="{MO}; font-size: 12px; color: var(--muted); display: flex; justify-content: space-between"><span>-5 min</span><span>-2.5</span><span>now</span></div></div></div>
<div style="{MO}; font-size: 13px; color: var(--muted); display: flex; gap: 20px"><span><span style="color: var(--text)">━━</span> CPU <span style="color: var(--text)">23%</span></span><span>╍╍ Memory <span style="color: var(--text)">39%</span></span><span style="flex: 1"></span><span>avg CPU 27% · peak 41%</span></div>'''
PROCS=[('postgres','container','1204',8.1,612,'#5B8DEF'),('node','container','2231',6.4,388,'#5FD08A'),('dockerd','system','887',3.0,142,'#A3A3AB'),('nginx','container','1102',1.2,38,'#E8B64A'),('opsbox','system','412',0.1,14,'var(--accent)')]
def prow(i,p):
    n,sub,pid,cpu,mb,c=p
    me=n=='opsbox'
    cw=min(100,cpu/10*100); mw=min(100,mb/800*100)
    badge='<span style="margin-left: 6px; padding: 1px 7px; border-radius: 999px; font-size: 11px; background: color-mix(in srgb, var(--accent) 16%, transparent); color: var(--accent)">this</span>' if me else ''
    return (f'<div style="display: grid; grid-template-columns: 26px minmax(0,1.7fr) minmax(80px,1fr) minmax(80px,1fr); align-items: center; gap: 14px; min-height: 56px; padding: 0 4px; border-bottom: 1px solid var(--line)">'
      f'<span style="{MO}; font-size: 12px; color: var(--muted)">{i+1:02d}</span>'
      f'<span style="display: flex; align-items: center; gap: 10px; min-width: 0"><span aria-hidden="true" style="flex: none; width: 30px; height: 30px; border-radius: 8px; background: color-mix(in srgb, {c} 18%, transparent); color: {c}; display: flex; align-items: center; justify-content: center; font: 600 12px \'Geist\', sans-serif">{n[0].upper()}</span>'
      f'<span style="display: flex; flex-direction: column; min-width: 0"><span style="font-size: 15px; font-weight: 500; white-space: nowrap">{n}{badge}</span><span style="font-size: 12.5px; color: var(--muted); white-space: nowrap; overflow: hidden; text-overflow: ellipsis">{sub} · pid {pid}</span></span></span>'
      f'<span style="display: flex; flex-direction: column; gap: 6px"><span style="{MO}; font-size: 14px; text-align: right">{cpu}%</span><span style="height: 4px; border-radius: 99px; background: var(--fill2); overflow: hidden"><span style="display: block; height: 100%; width: {cw:.0f}%; border-radius: 99px; background: var(--accent)"></span></span></span>'
      f'<span style="display: flex; flex-direction: column; gap: 6px"><span style="{MO}; font-size: 14px; text-align: right">{mb} MB</span><span style="height: 4px; border-radius: 99px; background: var(--fill2); overflow: hidden"><span style="display: block; height: 100%; width: {mw:.0f}%; border-radius: 99px; background: var(--text); opacity: .55"></span></span></span>'
      '</div>')
top=PROCS[0]
procs=(f'<div style="display: flex; flex-direction: column">'
  f'<div style="display: flex; align-items: center; gap: 14px; padding: 14px 16px; margin-bottom: 8px; border-radius: 12px; background: color-mix(in srgb, var(--accent) 9%, transparent); border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent)">'
  f'<span style="display: flex; flex-direction: column; gap: 2px; flex: 1"><span style="font-size: 12.5px; color: var(--muted)">Heaviest right now</span><span style="font-size: 16px; font-weight: 600">postgres</span></span><span style="display: flex; gap: 18px; {MO}; font-size: 13px"><span><span style="color: var(--muted)">CPU </span>43%</span><span><span style="color: var(--muted)">mem </span>51%</span></span></div>'
  '<div role="row" style="display: grid; grid-template-columns: 26px minmax(0,1.7fr) minmax(80px,1fr) minmax(80px,1fr); gap: 14px; padding: 0 4px 8px; font-size: 12px; color: var(--muted); border-bottom: 1px solid var(--line)"><span>#</span><span>Process</span><span style="text-align: right">CPU</span><span style="text-align: right">Memory</span></div>'
  +''.join(prow(i,p) for i,p in enumerate(PROCS))+
  '</div>')
proc_right=('<div style="display: flex; align-items: center; gap: 10px"><div role="radiogroup" aria-label="Sort by" style="display: flex; padding: 3px; border-radius: 9px; background: var(--fill)">'
  '<button type="button" role="radio" aria-checked="true" style="min-height: 28px; padding: 0 12px; border: 0; border-radius: 7px; background: var(--hl); box-shadow: var(--shadow); color: var(--text); font: 500 13px \'Geist\', sans-serif; cursor: pointer">CPU</button>'
  '<button type="button" role="radio" aria-checked="false" style="min-height: 28px; padding: 0 12px; border: 0; border-radius: 7px; background: transparent; color: var(--muted); font: 500 13px \'Geist\', sans-serif; cursor: pointer">Memory</button></div>'
  '<a href="System.dc.html" style="font-size: 13px; color: var(--accent)">All →</a></div>')
def li(a,b): return f'<div style="display: flex; justify-content: space-between; gap: 12px; min-height: 36px; align-items: center; border-bottom: 1px solid var(--line); font-size: 15px"><span>{a}</span>{b}</div>'
cont=''.join([li('web-app',stat('run','up 3d')),li('postgres-15',stat('run','up 3d')),li('redis',stat('run','up 3d')),li('nginx-proxy',stat('warn','restarted ×3')),li('worker-old',stat('stop','stopped'))])
recl=f'''<div style="display: flex; flex-direction: column; gap: 14px"><span style="{MO}; font-size: 32px; font-weight: 500; letter-spacing: -0.03em">14.6<span style="font-size: 15px; color: var(--muted)"> GB</span></span><div style="display: flex; height: 8px; gap: 2px"><div style="flex: 9.8; background: var(--accent)"></div><div style="flex: 2.9; background: var(--text)"></div><div style="flex: 1.2; background: var(--warn)"></div><div style="flex: 0.7; background: var(--muted)"></div></div><div style="display: flex; flex-wrap: wrap; gap: 6px 16px">{sw('var(--accent)','docker 9.8')}{sw('var(--text)','logs 2.9')}{sw('var(--warn)','apt 1.2')}{sw('var(--muted)','tmp 0.7')}</div><div>{btn('Review and clean up →','link',href='Cleanup.dc.html')}</div></div>'''
act=''.join(f'<div style="display: flex; gap: 14px; min-height: 32px; align-items: center; {MO}; font-size: 13px"><span style="color: var(--muted)">{t}</span><span style="color: {c}">{x}</span></div>' for t,x,c in [('03:36','minh.dev deployed web a3f9c21','var(--text)'),('03:21','admin restarted web-app','var(--text)'),('03:02','admin opened terminal','var(--text)'),('02:47','failed login ×5 · 203.0.113.9','var(--warn)'),('02:10','cron cleaned journal −1.1 GB','var(--text)')])
def arow(st,txt,link):
    return '<div style="display: flex; flex-wrap: wrap; align-items: center; gap: 8px 20px; min-height: 48px; border-bottom: 1px solid var(--line)">'+st+'<span style="flex: 1 1 280px; font-size: 15px">'+txt+'</span>'+link+'</div>'
attention=section('Needs attention · 3','<div style="display: flex; flex-direction: column">'+arow(stat('warn','security'),'3 security updates are waiting (openssl, libc6, linux-image)',btn('Review and install →','link',href='System.dc.html'))+arow(stat('warn','reboot'),'A new kernel is installed. Reboot to use it.',btn('Schedule reboot →','link',href='System.dc.html'))+arow(stat('warn','disk'),'Disk / is at 82%. 14.6 GB can be reclaimed.',btn('Clean up →','link',href='Cleanup.dc.html'))+'</div>')
main=hdr('Overview','srv-sgn-01 · Ubuntu 24.04 · 8 cores · 8 GB RAM · up 41 days','<span style="'+MO+'; font-size: 13px; color: var(--muted)">design mockup · sample data · refresh 2s</span>')+attention+strip+f'''<div style="display: grid; grid-template-columns: repeat(6, minmax(0, 1fr)); gap: 24px; align-items: stretch">
{section('CPU and memory · last 5 minutes',chart,grow='grid-column: span 3; ')}
{section('Top processes <span style="margin-left: 8px; padding: 2px 8px; border-radius: 999px; font-size: 11px; font-weight: 500; background: color-mix(in srgb, var(--ok) 16%, transparent); color: var(--ok)">● live</span>',procs,proc_right,grow='grid-column: span 3; ')}
{section('Containers',cont,btn('View all','link',href='Containers.dc.html'),grow='grid-column: span 2; ')}
{section('Reclaimable space',recl,grow='grid-column: span 2; ')}
{section('Recent activity',act,btn('Audit log','link',href='Security.dc.html'),grow='grid-column: span 2; ')}
</div>'''
W['Main']=page('Overview','overview',main,h=1570)
# ---------- Apps ----------
def node(x,y,name,st,l2,l3,sel=False):
    b='2px solid var(--accent)' if sel else '1px solid var(--ls)'
    return f'<div class="topo-node{' sel' if sel else ''}" style="animation-delay: {(x+y)/2000:.2f}s; position: absolute; left: {x}px; top: {y}px; width: 220px; height: 104px; box-sizing: border-box; border: {b}; border-radius: 8px; background: var(--bg); padding: 12px 14px; display: flex; flex-direction: column; justify-content: space-between"><div style="display: flex; justify-content: space-between; align-items: center; gap: 8px"><span style="{MO}; font-size: 15px; font-weight: 600">{name}</span>{st}</div>{mono(l2)}{l3}</div>'
def ln(x,y,w,h,d=0): return f'<div class="topo-edge {"v" if h>w else "h"}" style="position: absolute; left: {x}px; top: {y}px; width: {w}px; height: {max(h,2)}px; animation-delay: -{d}s"></div>'
graph=f'''<div style="overflow-x: auto"><div role="group" aria-label="Service topology" style="position: relative; width: 960px; height: 300px">
{ln(220,150,75,1)}{ln(295,62,1,88,.3)}{ln(295,62,75,1,.6)}{ln(590,62,150,1,.2)}{ln(590,238,150,1,.9)}
<span style="position: absolute; left: 618px; top: 40px; {MO}; font-size: 12px; color: var(--muted)">tcp :5432</span>
<span style="position: absolute; left: 618px; top: 216px; {MO}; font-size: 12px; color: var(--muted)">tcp :6379</span>
{node(0,98,'web',stat('run','active'),'acme/shop · main · Dockerfile',mono('shop.example.com','var(--accent)',12,400,'text-decoration: underline; text-underline-offset: 3px'),True)}
{node(370,10,'api',stat('build','building'),'acme/shop-api · main · Nixpacks',mono('api.shop.example.com','var(--accent)',12,400,'text-decoration: underline; text-underline-offset: 3px'))}
{node(370,186,'worker',stat('run','active'),'acme/shop-api · npm run worker',mono('no public port'))}
{node(740,10,'postgres',stat('run','active'),'PostgreSQL 16 · volume 4.2 GB',mono('backup daily 02:00'))}
{node(740,186,'redis',stat('run','active'),'Redis 7 · 24 MB',mono('internal only'))}
</div></div>
<span style="{MO}; font-size: 13px; color: var(--muted)">Private network · services reach each other by name, e.g. postgres:5432</span>'''
deps=table('Deployments for web',[('Commit','left'),('Message','left'),('Status','left'),('',"right")],[
 [mono('a3f9c21','var(--text)'),'Fix cart when out of stock '+mono('· 2 min ago · 48 s'),stat('run','live'),''],
 [mono('7be1d04','var(--text)'),'Add checkout page '+mono('· yesterday'),stat('stop','replaced'),btn('Roll back','link',aria='Roll back to 7be1d04')],
 [mono('1c80ae7','var(--text)'),'Update dependencies '+mono('· 3 days ago'),stat('fail','build failed'),'']],minw=480)
log='''<span style="color: #8A8578">01</span>  03:36:02 clone    acme/shop@a3f9c21
<span style="color: #8A8578">02</span>  03:36:03 detect   Dockerfile
<span style="color: #8A8578">03</span>  03:36:03 build    FROM node:22-alpine
<span style="color: #8A8578">04</span>  03:36:21 build    npm ci · 412 packages
<span style="color: #8A8578">05</span>  03:36:44 build    npm run build · ok
<span style="color: #8A8578">06</span>  03:36:49 push     shop-web:a3f9c21 (118 MB)
<span style="color: #8A8578">07</span>  03:36:50 deploy   start new version, wait for /healthz
<span style="color: #8A8578">08</span>  03:36:53 health   200 OK after 3 s
<span style="color: #8A8578">09</span>  03:36:53 switch   traffic moved, zero downtime
<span style="color: #8A8578">10</span>  03:36:54 done     stopped old 7be1d04'''
svc=tabs(['Deployments','Variables','Domains & SSL','Metrics','Settings'],0)+f'''<div style="display: flex; flex-wrap: wrap; gap: 40px"><div style="flex: 1 1 420px; min-width: 0; display: flex; flex-direction: column; gap: 12px"><span style="font-size: 14px; color: var(--muted)">Every push to main builds and deploys. Health check: /healthz</span>{deps}</div><div style="flex: 1 1 460px; min-width: 0; display: flex; flex-direction: column; gap: 12px"><div style="display: flex; justify-content: space-between; {MO}; font-size: 13px; color: var(--muted)"><span>build log · a3f9c21</span><span>live tail</span></div>{code(log,260)}</div></div>'''
main=hdr('shop-platform','5 services · srv-sgn-01 · environment production ▾',btn('+ Database')+btn('Deploy from GitHub','primary',href='NewService.dc.html'))+section('Services',graph)+section('web',svc,btn('Redeploy')+btn('Open site','link'))
W['Apps']=page('Apps and deployments','apps',main,h=1300)
# ---------- Containers ----------
act_=lambda n:btn('logs','link',aria='Logs '+n)+btn('shell','link',aria='Shell '+n)+btn('restart','link',aria='Restart '+n)
ct=table('Containers',[('Name','left'),('Image','left'),('Status','left'),('CPU','right'),('Memory','right'),('Ports','left'),('','right')],[
 {'cells':['web-app',mono('node:22-alpine'),stat('run','up 3d'),mono('2.1%','var(--text)'),mono('210 MB','var(--text)'),mono('3000→3000'),act_('web-app')],'hl':True},
 ['postgres-15',mono('postgres:15'),stat('run','up 3d'),mono('8.1%','var(--text)'),mono('612 MB','var(--text)'),mono('5432 internal'),act_('postgres-15')],
 ['redis',mono('redis:7'),stat('run','up 3d'),mono('0.4%','var(--text)'),mono('24 MB','var(--text)'),mono('6379 internal'),act_('redis')],
 ['nginx-proxy',mono('nginx:1.27'),stat('warn','restarted ×3'),mono('1.2%','var(--text)'),mono('38 MB','var(--text)'),mono('80, 443'),act_('nginx-proxy')],
 ['worker-old',mono('acme/worker:1.2'),stat('stop','stopped 6d'),mono('–'),mono('–'),mono('–'),btn('start','link',aria='Start worker-old')+btn('✕ delete','link',aria='Delete worker-old')]],minw=900)
clog='''03:21:07 INFO  server listening :3000
03:21:09 INFO  GET /api/health 200 3ms
03:21:14 INFO  GET /api/items 200 21ms
03:21:14 WARN  slow query 412ms items.list
03:21:22 INFO  POST /api/login 200 88ms
03:21:31 INFO  GET /api/health 200 2ms
03:21:46 ERROR upstream timeout cache:6379
03:21:46 INFO  retry 1/3 ok
03:21:52 INFO  GET /api/items 200 19ms'''
main=hdr('Containers','Docker 29.9 · /var/run/docker.sock · 4 running, 1 stopped',btn('Pull image')+btn('Run container','primary'))+tabs(['Containers (5)','Images (11)','Volumes (6)','Networks (3)'],0)+ct+section('web-app · logs',code(clog,220),mono('follow: on'))
W['Containers']=page('Containers','containers',main,h=1100)
# ---------- Cleanup ----------
def item(i,checked,title,desc,size,warn=False):
    d=f'<span style="font-size: 14px; color: {"var(--warn)" if warn else "var(--muted)"}">{desc}</span>'
    ck=' checked' if checked else ''
    return f'<div style="display: flex; align-items: center; gap: 16px; min-height: 72px; border-bottom: 1px solid var(--line); flex-wrap: wrap"><input id="c{i}" type="checkbox"{ck} style="width: 22px; height: 22px; accent-color: var(--accent); flex: none"><label for="c{i}" style="flex: 1 1 300px; min-width: 0; display: flex; flex-direction: column; gap: 3px; cursor: pointer"><span style="font-size: 15px; font-weight: 500">{title}</span>{d}</label>{mono(size,"var(--text)",14)}</div>'
bar='''<div style="display: flex; height: 14px; gap: 2px"><div style="flex: 41; background: var(--ls)"></div><div style="flex: 9.8; background: var(--accent)"></div><div style="flex: 2.9; background: var(--text)"></div><div style="flex: 1.2; background: var(--warn)"></div><div style="flex: 0.7; background: var(--muted)"></div><div style="flex: 64.4; background: var(--line)"></div></div>'''
items=''.join([item(1,True,'Docker: unused images and build cache','8 dangling images, 3 older than 30 days · running containers untouched','9.8 GB'),
 item(2,True,'systemd journal older than 7 days','/var/log/journal · keeps the last 7 days','2.1 GB'),
 item(3,True,'apt cache and unused packages','apt clean + autoremove · 4 orphaned packages','1.2 GB'),
 item(4,True,'/tmp and rotated logs (.gz, .1)','Files not accessed in 10+ days','0.7 GB'),
 item(5,False,'Docker volumes not attached to any container','▲ May contain data · unchecked by default · 2 volumes','1.4 GB',True),
 f'<div style="display: flex; align-items: center; gap: 16px; min-height: 72px; flex-wrap: wrap"><span style="width: 22px; flex: none"></span><div style="flex: 1 1 300px; display: flex; flex-direction: column; gap: 3px"><span style="font-size: 15px; font-weight: 500">Large files over 1 GB</span><span style="font-size: 14px; color: var(--muted)">/home/deploy/backup-2025.tar.gz · /var/lib/mysql-old</span></div>{btn("Open in Files →","link",href="Files.dc.html")}</div>'])
auto=f'<div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 16px"><span style="font-size: 15px; color: var(--muted); max-width: 640px">Every Sunday 03:00, safe items only (journal, apt, tmp). Alert when disk passes 85%.</span><button type="button" role="switch" aria-checked="true" aria-label="Enable auto cleanup" style="min-height: 44px; min-width: 64px; border: 1px solid var(--accent); border-radius: 8px; background: var(--accent); color: var(--onaccent); {MO}; font-size: 13px; font-weight: 600; cursor: pointer">ON</button></div>'
main=hdr('Cleanup','last scan 03:30 · nothing is deleted until you confirm',btn('Rescan'))+section('Disk /',f'<div style="display: flex; flex-wrap: wrap; align-items: flex-end; gap: 24px 48px"><div style="display: flex; flex-direction: column; gap: 6px"><span style="font-size: 14px; color: var(--muted)">Selected to reclaim</span><span style="{MO}; font-size: 52px; font-weight: 500; letter-spacing: -0.04em; line-height: 1">12.9<span style="font-size: 18px; color: var(--muted)"> GB</span></span><span style="{MO}; font-size: 13px; color: var(--muted)">disk drops from 82% to 71%</span></div><div style="flex: 1 1 360px; min-width: 0; display: flex; flex-direction: column; gap: 10px">{bar}<div style="display: flex; flex-wrap: wrap; gap: 6px 16px">{sw("var(--ls)","system 41")}{sw("var(--accent)","docker 9.8")}{sw("var(--text)","logs 2.9")}{sw("var(--warn)","apt 1.2")}{sw("var(--muted)","tmp 0.7")}{sw("var(--line)","free 22 GB")}</div></div></div>')+section('Items found',f'<div style="display: flex; flex-direction: column">{items}</div><div style="display: flex; flex-wrap: wrap; gap: 12px; align-items: center">{btn("Preview and clean 12.9 GB","primary")}{mono("dry run first: you see the exact list before anything is removed")}</div>')+section('Auto cleanup',auto)
W['Cleanup']=page('Cleanup','cleanup',main,h=1250)
# ---------- Terminal ----------
def host(name,sub,sel=False):
    b='border-left: 2px solid var(--accent); background: var(--hl);' if sel else 'border-left: 2px solid transparent;'
    return f'<button type="button" style="min-height: 56px; text-align: left; padding: 0 14px; border: 0; {b} color: var(--text); display: flex; flex-direction: column; justify-content: center; gap: 2px; cursor: pointer; background-color: transparent; font: 500 14px \'Geist\', sans-serif">{name}{mono(sub)}</button>' if not sel else f'<button type="button" style="min-height: 56px; text-align: left; padding: 0 14px; border: 0; {b} color: var(--text); display: flex; flex-direction: column; justify-content: center; gap: 2px; cursor: pointer; font: 500 14px \'Geist\', sans-serif">{name}{mono(sub)}</button>'
hosts=f'<div style="display: flex; flex-direction: column">{host("srv-sgn-01","local · root",True)}{host("db-primary","deploy@10.0.1.12 · ed25519")}{host("edge-hn-02","ubuntu@203.0.113.40:2222")}</div>{btn("+ Add SSH host")}<span style="font-size: 13px; color: var(--muted); line-height: 1.6">Private keys stay encrypted on the server and never reach the browser. The first host fingerprint is pinned; a change raises a warning.</span>'
G='<span style="color: var(--ok)">root@srv-sgn-01</span>:<span style="color: var(--accent)">~</span>#'
term=f'''<div data-theme="dark" style="flex: 1 1 560px; min-width: 0; display: flex; flex-direction: column; background: var(--code); border: 1px solid var(--ls); border-radius: 8px; color: var(--text)">
<div role="tablist" aria-label="Terminal sessions" style="display: flex; flex-wrap: wrap; align-items: center; border-bottom: 1px solid var(--line)">
<button type="button" role="tab" aria-selected="true" style="min-height: 44px; padding: 0 16px; background: var(--hl); border: 0; border-bottom: 2px solid var(--accent); color: var(--text); {MO}; font-size: 14px; cursor: pointer">srv-sgn-01</button>
<button type="button" role="tab" aria-selected="false" style="min-height: 44px; padding: 0 16px; background: transparent; border: 0; border-bottom: 2px solid transparent; color: var(--muted); {MO}; font-size: 14px; cursor: pointer">db-primary</button>
<button type="button" aria-label="New terminal tab" style="min-height: 44px; min-width: 44px; background: transparent; border: 0; color: var(--muted); font-size: 18px; cursor: pointer">+</button>
<span style="flex: 1"></span>
<button type="button" style="min-height: 44px; padding: 0 12px; background: transparent; border: 0; color: var(--muted); font: 500 13px 'Geist', sans-serif; cursor: pointer">Split</button>
<button type="button" style="min-height: 44px; padding: 0 12px; background: transparent; border: 0; color: var(--muted); font: 500 13px 'Geist', sans-serif; cursor: pointer">Full screen</button></div>
<pre style="margin: 0; padding: 18px 20px; flex: 1; min-height: 480px; color: var(--codetext); font: 14px/1.7 'Geist Mono', monospace; overflow-x: auto; white-space: pre">{G} docker ps
NAMES         STATUS
web-app       Up 3 days
postgres-15   Up 3 days
redis         Up 3 days
nginx-proxy   Up 2 hours
{G} df -h /
Filesystem      Size  Used Avail Use% Mounted on
/dev/vda1       120G   98G   22G  82% /
{G} tail -n 3 /var/log/auth.log
Nov 17 02:47:03 sshd[9121]: Failed password for root from 203.0.113.9
Nov 17 02:47:09 sshd[9123]: Failed password for root from 203.0.113.9
Nov 17 03:02:11 sshd[9210]: Accepted publickey for deploy
{G} <span style="background: var(--codetext); color: var(--code)"> </span></pre>
<div style="display: flex; flex-wrap: wrap; gap: 4px 20px; padding: 8px 16px; border-top: 1px solid var(--line); {MO}; font-size: 13px; color: var(--muted)"><span style="color: var(--ok)">■ encrypted websocket</span><span>pty 120×32</span><span>utf-8</span><span>Ctrl+Shift+C/V copy, paste</span></div></div>'''
main=hdr('Terminal and SSH','local shell and SSH to other machines · every session is written to the audit log')+f'<div style="display: flex; flex-wrap: wrap; gap: 32px; align-items: stretch"><section style="flex: 1 1 240px; max-width: 320px; min-width: 0; background: var(--hl); border: 1px solid var(--line); border-radius: 16px; box-shadow: var(--shadow); padding: 24px 28px 28px; display: flex; flex-direction: column; gap: 20px"><h2 style="margin: 0; font-size: 16px; font-weight: 600; letter-spacing: -0.02em">Hosts</h2>{hosts}</section>{term}</div>'
W['Terminal']=page('Terminal and SSH','terminal',main,h=1100)
# ---------- Files ----------
def row(ic,name,size,mode,mod,sel=False,warn=False):
    mc='var(--warn)' if warn else 'var(--muted)'
    return {'cells':[f'<input type="checkbox" aria-label="Select {name}" style="width: 20px; height: 20px; accent-color: var(--accent)"{" checked" if sel else ""}>',f'<span style="{MO}; font-size: 14px; color: var(--muted)">{ic}</span> {name}',mono(size,'var(--text)'),mono(mode+(' ▲' if warn else ''),mc),mono(mod)],'hl':sel}
ft=table('Contents of /var/www/app',[('','left'),('Name','left'),('Size','right'),('Mode','left'),('Modified','left')],[
 row('d','..','','',''),row('d','public','–','drwxr-xr-x','yesterday'),row('d','logs','–','drwxr-x---','03:21'),
 row('f','.env','1.2 KB','-rw-r--r--','3 days ago',True,True),row('f','docker-compose.yml','2.4 KB','-rw-r--r--','5 days ago'),row('f','package.json','980 B','-rw-r--r--','5 days ago')],minw=560)
env='''<span style="color: #8A8578">1</span>  NODE_ENV=production
<span style="color: #8A8578">2</span>  PORT=3000
<span style="color: #8A8578">3</span>  DATABASE_URL=postgres://app:••••••@postgres-15:5432/app
<span style="color: #8A8578">4</span>  REDIS_URL=redis://redis:6379
<span style="color: #8A8578">5</span>  SESSION_SECRET=••••••••••••••••'''
editor=f'<div style="padding: 10px 14px; border: 1px solid var(--warn); border-radius: 8px; color: var(--warn); font-size: 14px; line-height: 1.5">▲ This file holds secrets but its mode is 644. Suggested: chmod 600.</div>{code(env,200)}<div style="display: flex; gap: 10px; flex-wrap: wrap">{btn("Save","primary")}{btn("chmod 600")}</div>'
path=f'<nav aria-label="Path" style="{MO}; font-size: 14px; color: var(--muted)">/ var / www / <span style="color: var(--text)">app</span></nav>'
main=hdr('Files','/var/www/app',btn('New folder')+btn('Upload','primary'))+f'<div style="display: flex; flex-wrap: wrap; gap: 12px; align-items: center"><label for="q" style="position: absolute; width: 1px; height: 1px; overflow: hidden">Search files</label><input id="q" type="search" placeholder="Search by name in this folder" style="flex: 1 1 280px; min-height: 44px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px">{mono("1 selected")}{btn("Download")}{btn("Compress")}{btn("✕ Delete","danger")}</div><div style="display: flex; flex-wrap: wrap; gap: 40px">{section("Contents",ft,path,grow="flex: 3 1 520px; ")}{section(".env · unsaved",editor,grow="flex: 2 1 380px; ")}</div>'
W['Files']=page('Files','files',main,h=1050)
# ---------- Security ----------
chk=table('Configuration checks',[('Check','left'),('Status','left'),('','right')],[
 ["TLS (HTTPS) · Let's Encrypt auto-renew",stat('run','on'),''],
 ['Two-factor authentication (TOTP) for admin',stat('run','on'),''],
 ['Lock out after 5 failed sign-ins (15 min)',stat('run','on'),''],
 ['Allow listed IPs only',stat('warn','off'),btn('Enable','link')],
 ['Admin port bound to 127.0.0.1:8443 (reach via proxy or tunnel)',stat('run','on'),''],
 ['Private SSH keys encrypted at rest',stat('run','on'),''],
 ['SSH daemon on this server (port 22 closed, managed through opsbox)',stat('run','off'),btn('Enable for 1 hour','link')],
 ['Watchdog restarts opsbox if it stops answering',stat('run','on'),''],
 ['Automatic rollback if an opsbox update fails its health check',stat('run','on'),btn('Safe mode page →','link',href='Recovery.dc.html')]],minw=520)
ip=f'<div style="display: flex; flex-direction: column; gap: 6px"><label for="ip" style="font-size: 14px; color: var(--muted)">Allowed IP / CIDR list</label><input id="ip" type="text" value="203.0.113.0/24, 198.51.100.7" style="min-height: 44px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px; max-width: 480px"></div>'
usr=table('Users',[('User','left'),('Role','left'),('2FA','left'),('','right')],[
 ['admin','Full access',stat('run','on'),btn('Change password','link')],
 ['minh.dev','Deploy and view logs, no terminal',stat('warn','off'),btn('Permissions','link')]],minw=520)
aud=table('Audit log',[('Time','left'),('User','left'),('Action','left'),('IP','left')],[
 [mono('03:36'),'minh.dev','deploy web a3f9c21',mono('203.0.113.21')],
 [mono('03:21'),'admin','restart web-app',mono('203.0.113.5')],
 [mono('03:02'),'admin','open terminal srv-sgn-01',mono('203.0.113.5')],
 [mono('02:47','var(--warn)'),'unknown',f'<span style="color: var(--warn)">▲ failed sign-in ×5, IP locked 15 min</span>',mono('203.0.113.9','var(--warn)')]],minw=560)
main=hdr('Security','8 of 9 checks passing',btn('✕ Sign out all sessions','danger'))+f'<div style="display: flex; flex-wrap: wrap; gap: 40px">{section("Configuration",chk+ip,grow="flex: 3 1 480px; ")}{section("Users",usr+btn("+ Invite user"),grow="flex: 2 1 380px; ")}</div>'+section('Audit log',aud,btn('Export CSV'))
W['Security']=page('Security','security',main,h=1150)
# ---------- Login ----------
inp=lambda i,l,t,v,ph='',ac='':f'<div style="display: flex; flex-direction: column; gap: 6px"><label for="{i}" style="font-size: 14px; color: var(--muted)">{l}</label><input id="{i}" type="{t}" value="{v}" placeholder="{ph}" autocomplete="{ac}" style="min-height: 48px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 15px"></div>'
lbody=f'''<div data-theme="{{{{theme}}}}" style="min-height: 100vh; display: flex; flex-wrap: wrap; background: var(--bg); color: var(--text); {UI}">
<div style="flex: 1 1 440px; box-sizing: border-box; padding: 56px 64px; display: flex; flex-direction: column; justify-content: space-between; gap: 48px; border-right: 1px solid var(--ls)">
{logo(36,23)}
<div style="display: flex; flex-direction: column; gap: 20px; max-width: 520px"><h1 style="margin: 0; font-size: 40px; font-weight: 600; letter-spacing: -0.03em; line-height: 1.1">Deploy from Git to your own Linux box.</h1><p style="margin: 0; font-size: 16px; line-height: 1.6; color: var(--muted)">Builds, domains, SSL, databases, terminal and files for one server, in a single binary.</p></div>
<div style="display: flex; flex-direction: column; gap: 8px"><span style="font-size: 13px; color: var(--muted)">Install on a fresh instance</span><pre style="margin: 0; padding: 14px 16px; background: var(--code); color: var(--codetext); font: 13px/1.6 'Geist Mono', monospace; overflow-x: auto; border-radius: 8px">$ curl -fsSL https://get.opsbox.example/install.sh | sudo sh</pre></div>
</div>
<form style="flex: 1 1 440px; box-sizing: border-box; padding: 56px 64px; display: flex; flex-direction: column; justify-content: center; gap: 18px; max-width: 640px">
<h2 style="margin: 0; font-size: 22px; font-weight: 600">Sign in to srv-sgn-01</h2>
{inp('u','Username','text','admin','', 'username')}{inp('p','Password','password','placeholder','','current-password')}{inp('t','Authentication code','text','','000000','one-time-code')}
{btn('Sign in','primary',href='Main.dc.html')}
<span style="font-size: 13px; color: var(--muted); line-height: 1.6">5 failed attempts lock your IP for 15 minutes. Every sign-in is logged.</span>
<div><button type="button" onClick="{{{{toggleTheme}}}}" style="min-height: 44px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: transparent; color: var(--text); font: 500 14px 'Geist', sans-serif; cursor: pointer">{{{{themeLabel}}}} theme</button></div>
</form></div>'''
W['Login']=head('opsbox — Sign in',lbody,'',h=900)

# ---------- New service: repo picker ----------
def stepper(active):
    names=['Repository','Configure','Deploy']
    out=''
    for i,n in enumerate(names):
        on=i==active; done=i<active
        c='var(--text)' if on else ('var(--muted)')
        bd='2px solid var(--accent)' if on else '2px solid var(--line)'
        mark='✓' if done else f'0{i+1}'
        out+=f'<div style="flex: 1 1 160px; padding: 10px 0; border-top: {bd}; display: flex; gap: 10px; align-items: baseline"><span style="{MO}; font-size: 13px; color: {"var(--accent)" if on else "var(--muted)"}">{mark}</span><span style="font-size: 15px; font-weight: 500; color: {c}">{n}</span></div>'
    return f'<nav aria-label="Steps" style="display: flex; flex-wrap: wrap; gap: 8px 24px">{out}</nav>'
def repo(name,vis,lang,upd,build,hl=False):
    v=stat('run','public') if vis=='public' else mono('■ private','var(--muted)',12,500,'letter-spacing: 0.04em')
    return {'cells':[f'<span style="{MO}; font-size: 15px; font-weight: 500">{name}</span>',v,mono(lang),mono(upd),mono(build,'var(--text)'),btn('Select →','primary',href='Configure.dc.html',aria='Select '+name) if hl else btn('Select →','default',href='Configure.dc.html',aria='Select '+name)],'hl':hl}
rt=table('Repositories',[('Repository','left'),('Visibility','left'),('Language','left'),('Pushed','left'),('Detected build','left'),('','right')],[
 repo('acme/shop','private','TypeScript','2 min ago','Dockerfile',True),
 repo('acme/shop-api','private','TypeScript','2 min ago','Nixpacks · Node 22'),
 repo('acme/landing','public','Astro','4 days ago','static site'),
 repo('acme/billing-worker','private','Go','last week','Nixpacks · Go 1.23'),
 repo('acme/infra-notes','private','Markdown','2 weeks ago','nothing to build')],minw=900)
srcs=tabs(['GitHub','Public repo URL','Docker image','Templates'],0)
acct=f'<div style="display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px 24px"><div style="display: flex; flex-direction: column; gap: 4px"><span style="font-size: 15px; font-weight: 500">github.com/acme-co</span>{mono("GitHub App installed · access to 14 of 31 repositories · read-only code access")}</div><div style="display: flex; flex-wrap: wrap; gap: 10px">{btn("Switch account")}{btn("Manage repository access ↗")}</div></div>'
search=f'<div style="display: flex; flex-wrap: wrap; gap: 12px; align-items: center"><label for="rq" style="position: absolute; width: 1px; height: 1px; overflow: hidden">Search repositories</label><input id="rq" type="search" placeholder="Search 14 repositories" style="flex: 1 1 300px; min-height: 44px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px"><label for="org" style="position: absolute; width: 1px; height: 1px; overflow: hidden">Owner</label><select id="org" style="min-height: 44px; padding: 0 12px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px"><option>acme-co (organization)</option><option>minh-dev (personal)</option></select></div>'
note=f'<p style="margin: 0; font-size: 14px; line-height: 1.6; color: var(--muted); max-width: 720px">opsbox asks GitHub for read-only access to the repository you pick, plus a webhook so each push to the chosen branch can deploy automatically. The access token is stored encrypted on this server only.</p>'
main=hdr('New service','shop-platform · production',btn('Cancel',href='Apps.dc.html'))+stepper(0)+srcs+section('GitHub account',acct)+section('Choose a repository',search+rt+note)
W['NewService']=page('New service','apps',main,h=1150)
# ---------- New service: configure ----------
def fld(i,label,inner,hint=''):
    h=f'<span style="font-size: 13px; color: var(--muted)">{hint}</span>' if hint else ''
    return f'<div style="display: flex; flex-direction: column; gap: 6px"><label for="{i}" style="font-size: 14px; font-weight: 500">{label}</label>{inner}{h}</div>'
def inp2(i,v,ph='',w=''):
    return f'<input id="{i}" type="text" value="{v}" placeholder="{ph}" style="min-height: 44px; padding: 0 14px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px; box-sizing: border-box; {w}">'
def radio(i,checked,title,desc):
    ck=' checked' if checked else ''
    return f'<div style="display: flex; gap: 12px; align-items: flex-start; min-height: 56px; padding: 8px 0; border-bottom: 1px solid var(--line)"><input id="{i}" name="bm" type="radio"{ck} style="width: 20px; height: 20px; margin-top: 2px; accent-color: var(--accent); flex: none"><label for="{i}" style="display: flex; flex-direction: column; gap: 2px; cursor: pointer"><span style="font-size: 15px; font-weight: 500">{title}</span><span style="font-size: 13px; color: var(--muted)">{desc}</span></label></div>'
def chk(i,checked,label):
    return f'<div style="display: flex; gap: 12px; align-items: center; min-height: 44px"><input id="{i}" type="checkbox"{" checked" if checked else ""} style="width: 20px; height: 20px; accent-color: var(--accent); flex: none"><label for="{i}" style="font-size: 15px; cursor: pointer">{label}</label></div>'
def envrow(k,v):
    return f'<div style="display: flex; flex-wrap: wrap; gap: 8px"><input type="text" aria-label="Variable name" value="{k}" style="flex: 1 1 160px; min-height: 44px; padding: 0 12px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px; box-sizing: border-box"><input type="password" aria-label="Value for {k}" value="{v}" style="flex: 2 1 220px; min-height: 44px; padding: 0 12px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px; box-sizing: border-box">{btn("✕","default",aria="Remove "+k)}</div>'
sel_=lambda i,opts:f'<select id="{i}" style="min-height: 44px; padding: 0 12px; border: 1px solid var(--ctl); border-radius: 8px; background: var(--input); color: var(--text); {MO}; font-size: 14px">'+''.join(f'<option>{o}</option>' for o in opts)+'</select>'
src=f'<div style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 12px"><div style="display: flex; flex-direction: column; gap: 4px"><span style="{MO}; font-size: 16px; font-weight: 500">acme/shop</span>{mono("github.com/acme-co/shop · private · last push 2 min ago")}</div>{btn("Change repository",href="NewService.dc.html")}</div>'
form1=f'<div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 20px">{fld("sn","Service name",inp2("sn","web"),"Used for the internal hostname: web")}{fld("br","Branch",sel_("br",["main","staging","develop"]),"Pushes to this branch trigger deploys")}{fld("rd","Root directory",inp2("rd","/"),"Change for monorepos, e.g. /apps/web")}</div>'
build=f'<div style="display: flex; flex-direction: column">{radio("b1",True,"Dockerfile","Detected at /Dockerfile · FROM node:22-alpine")}{radio("b2",False,"Auto-detect with Nixpacks","Picks Node, Go, Python, Ruby… from your code, no Dockerfile needed")}{radio("b3",False,"Static site","Build to a folder and serve it with the built-in proxy")}</div><div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 20px; margin-top: 8px">{fld("sc","Start command (optional)",inp2("sc","","use Dockerfile CMD"))}{fld("pt","Port",inp2("pt","3000"),"The port your app listens on")}{fld("hc","Health check path",inp2("hc","/healthz"),"New version must answer 200 before traffic switches")}</div>'
envs=f'<div style="display: flex; flex-direction: column; gap: 10px">{envrow("NODE_ENV","production")}{envrow("DATABASE_URL","postgres://app:secret@postgres:5432/app")}{envrow("REDIS_URL","redis://redis:6379")}<div style="display: flex; flex-wrap: wrap; gap: 10px">{btn("+ Add variable")}{btn("Paste .env")}{btn("Reference another service","link")}</div></div>'
dom=f'<div style="display: flex; flex-wrap: wrap; gap: 20px; align-items: flex-end">{fld("dm","Public domain",f"<div style=\"display: flex; align-items: stretch\"><input id=\"dm\" type=\"text\" value=\"shop\" style=\"min-height: 44px; padding: 0 14px; border: 1px solid var(--ctl); border-right: 0; border-radius: 8px 0 0 2px; background: var(--input); color: var(--text); {MO}; font-size: 14px; width: 160px; box-sizing: border-box\"><span style=\"{MO}; font-size: 14px; color: var(--muted); border: 1px solid var(--ctl); border-radius: 0 2px 2px 0; padding: 0 14px; display: flex; align-items: center\">.example.com</span></div>","HTTPS certificate is issued automatically")}{btn("Use a custom domain")}</div>'
opts=f'<div style="display: flex; flex-direction: column">{chk("o1",True,"Auto-deploy on every push")}{chk("o2",False,"Preview environments for pull requests")}{chk("o3",True,"Zero-downtime deploys (keep old version until health check passes)")}</div>'
left=section('Source',src)+section('General',form1)+section('Build',build)+section('Environment variables',envs)+section('Networking',dom)+section('Deploy behavior',opts)
plan=''.join(f'<div style="display: flex; gap: 14px; min-height: 40px; align-items: baseline; border-bottom: 1px solid var(--line)"><span style="{MO}; font-size: 13px; color: var(--accent)">0{i+1}</span><span style="font-size: 15px">{t}</span></div>' for i,t in enumerate(['Clone acme/shop at main','Build with Dockerfile','Start on srv-sgn-01, port 3000','Wait for /healthz, then switch traffic','Route https://shop.example.com','Install push webhook on GitHub']))
side=f'<aside aria-label="Deploy summary" style="flex: 1 1 300px; max-width: 380px; min-width: 0; align-self: flex-start; border-top: 1px solid var(--ls); padding-top: 14px; display: flex; flex-direction: column; gap: 16px"><h2 style="margin: 0; font-size: 16px; font-weight: 600; letter-spacing: -0.02em">What happens when you deploy</h2><div style="display: flex; flex-direction: column">{plan}</div>{mono("target srv-sgn-01 · 22 GB free · env production")}<div style="display: flex; flex-wrap: wrap; gap: 10px">{btn("Deploy","primary",href="Apps.dc.html")}{btn("Back",href="NewService.dc.html")}</div></aside>'
main=hdr('Configure web','acme/shop → shop-platform / production')+stepper(1)+f'<div style="display: flex; flex-wrap: wrap; gap: 48px; align-items: flex-start"><div style="flex: 3 1 560px; min-width: 0; display: flex; flex-direction: column; gap: 32px">{left}</div>{side}</div>'
W['Configure']=page('Configure service','apps',main,h=1900)

exec(open('pages2.py',encoding='utf-8').read())
exec(open('pages3.py',encoding='utf-8').read())
exec(open('pages4.py',encoding='utf-8').read())
exec(open('pages5.py',encoding='utf-8').read())
exec(open('pages6.py',encoding='utf-8').read())
for k,v in W.items(): open(f'project/{k}.dc.html','w',encoding='utf-8').write(v)
for k in ['Main','Apps']:
    open(f'project/{k}Light.dc.html','w',encoding='utf-8').write(W[k].replace('"default":"dark"','"default":"light"',1))
print('ok',list(W))
