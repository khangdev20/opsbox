#!/usr/bin/env python3
"""Screenshot the running panel, screen by screen, in both themes (dev only; needs playwright).

The panel is served by `web/serve.py` in its built-in demo mode (`?mode=mock`) and the browser is
pre-signed-in as the demo admin, so the same screens can be compared before and after a UI change.
Use a real server instead with --base https://host:8443 (you must already be signed in: pass
--storage state.json saved by Playwright).

  python3 tools/screenshot-panel.py docs/screenshots/16d/before
  python3 tools/screenshot-panel.py docs/screenshots/16d/after --only overview,login

Files: <screen>-<theme>.png at 1440 px, plus <screen>-<theme>-390.png for the mobile set.
OPSBOX_CHROME=/path/to/chrome overrides the browser (see design/preview.py).
"""
import argparse
import glob
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# (file name, route, signed in, also capture at 390 px)
SCREENS = [
    ("login", "#/login", False, True),
    ("setup", "#/setup", False, False),
    ("recovery", "#/recovery", False, False),
    ("invite", "#/invite?token=demo", False, False),
    ("overview", "#/", True, True),
    ("apps", "#/apps", True, True),
    ("apps-domains", "#/apps?s=web&tab=domains", True, False),
    ("newservice", "#/apps/new", True, False),
    ("templates", "#/apps/new/templates", True, False),
    ("configure", "#/apps/new/configure", True, False),
    ("containers", "#/containers", True, True),
    ("system", "#/system", True, False),
    ("terminal", "#/terminal", True, False),
    ("files", "#/files", True, False),
    ("cleanup", "#/cleanup", True, False),
    ("backups", "#/backups", True, False),
    ("backupdest", "#/backups/new", True, False),
    ("alerts", "#/alerts", True, False),
    ("security", "#/security", True, False),
    ("access", "#/security/access", True, False),
    ("settings", "#/settings", True, False),
    ("database", "#/databases/postgres", True, False),
]


def find_chrome():
    if os.environ.get("OPSBOX_CHROME"):
        return os.environ["OPSBOX_CHROME"]
    for pat in ("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/*/Contents/MacOS/*",
                "~/.cache/ms-playwright/chromium-*/chrome-linux*/chrome"):
        hit = sorted(glob.glob(os.path.expanduser(pat)))
        if hit:
            return hit[-1]
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("out")
    ap.add_argument("--only", default="")
    ap.add_argument("--base", default="")
    ap.add_argument("--storage", default="")
    ap.add_argument("--port", default="8099")
    ap.add_argument("--web", default=os.path.join(ROOT, "web"), help="folder holding serve.py and the panel files (default: web/)")
    a = ap.parse_args()
    from playwright.sync_api import sync_playwright

    only = set(filter(None, a.only.split(",")))
    server = None
    base = a.base
    if not base:
        server = subprocess.Popen([sys.executable, os.path.join(a.web, "serve.py"), a.port], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        base = "http://127.0.0.1:%s/?mode=mock" % a.port
        time.sleep(1)
    os.makedirs(a.out, exist_ok=True)
    errors = []
    try:
        with sync_playwright() as p:
            exe = find_chrome()
            browser = p.chromium.launch(**({"executable_path": exe} if exe else {}))
            for theme in ("dark", "light"):
                for name, route, authed, mobile in SCREENS:
                    if only and name not in only:
                        continue
                    for width in ((1440, 390) if mobile else (1440,)):
                        ctx = browser.new_context(viewport={"width": width, "height": 900}, reduced_motion="reduce",
                                                  **({"storage_state": a.storage} if a.storage else {}))
                        ctx.add_init_script("localStorage.setItem('opsbox.theme','%s');%s" % (
                            theme, "sessionStorage.setItem('opsbox.user','admin');" if authed and not a.base else ""))
                        page = ctx.new_page()
                        page.on("console", lambda m, n=name: errors.append((n, m.text)) if m.type == "error" else None)
                        page.on("pageerror", lambda e, n=name: errors.append((n, str(e))))
                        page.goto(base + route if "?" not in base or route.startswith("#") else base + route)
                        page.wait_for_timeout(1400)
                        suffix = "" if width == 1440 else "-%d" % width
                        page.screenshot(path=os.path.join(a.out, "%s-%s%s.png" % (name, theme, suffix)), full_page=True)
                        ctx.close()
                        print("wrote", name, theme, width)
            browser.close()
    finally:
        if server:
            server.terminate()
    for n, e in errors:
        print("console error on", n, ":", e)


if __name__ == "__main__":
    main()
