#!/usr/bin/env python3
"""Render every design board (design/project/*.dc.html) to a 1440 px wide PNG in both themes.

Dev only (needs `pip install playwright` and a Chromium). The boards are templates for the
design canvas (`<x-dc>`, `<dc-import>`, `<sc-if>`, `{{theme}}`); the canvas runtime is not part of
this repo, so this script resolves the few constructs the generator emits:

  * `{{theme}}`            -> the theme being rendered
  * `<dc-import name=X>`   -> the body of X.dc.html, with the same props (theme, active)
  * `<sc-if value="{{v}}">` -> kept or dropped from a small, per-board context (nav "on/off" state,
                              header theme icon, menu closed, sidebar expanded)
  * other `{{...}}` (click handlers, labels) -> a static default

Not rendered statically: hover/focus states, the open account menu, the collapsed sidebar and
anything driven by `setState` (see `design/gen.py` for those parts).

Usage: python3 design/preview.py [Board ...]      (default: every board)
       OPSBOX_CHROME=/path/to/chrome python3 design/preview.py   (override the browser)
Output: docs/screenshots/design/<Board>-<dark|light>.png
"""
import glob
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PROJECT = os.path.join(HERE, "project")
ROOT = os.path.dirname(HERE)
OUT = os.path.join(ROOT, "docs", "screenshots", "design")
FONTS = os.path.join(ROOT, "web", "fonts")
THEMES = ("dark", "light")

SC_IF = re.compile(r"<sc-if\b([^>]*)>((?:(?!<sc-if\b).)*?)</sc-if>", re.S)
IMPORT = re.compile(r"<dc-import\b([^>]*)>\s*(?:</dc-import>)?", re.S)
BIND = re.compile(r"\{\{\s*([^}]*?)\s*\}\}")
ATTR = re.compile(r'([\w-]+)="([^"]*)"')


def read_board(name):
    with open(os.path.join(PROJECT, name + ".dc.html"), encoding="utf-8") as f:
        text = f.read()
    m = re.search(r"<x-dc>(.*)</x-dc>", text, re.S)
    inner = m.group(1) if m else text
    h = re.search(r"<helmet>(.*?)</helmet>", inner, re.S)
    helmet = h.group(1) if h else ""
    body = (inner[: h.start()] + inner[h.end():]) if h else inner
    return helmet, body


def value_of(expr, ctx):
    """Truthiness of a binding like `on.apps`, `isDark` or a literal `true`/`false`."""
    cur = ctx
    for part in expr.split("."):
        if isinstance(cur, dict) and part in cur:
            cur = cur[part]
        else:
            return None
    return cur


def resolve_ifs(body, ctx, active):
    def one(m):
        attrs = dict(ATTR.findall(m.group(1)))
        b = BIND.fullmatch(attrs.get("value", "").strip())
        expr = b.group(1) if b else ""
        v = value_of(expr, ctx)
        if v is None:  # unknown binding: fall back to the placeholder the generator wrote
            v = BIND.sub(lambda x: x.group(1), attrs.get("hint-placeholder-val", "false")).strip() == "true"
        return m.group(2) if v else ""

    prev = None
    while prev != body:
        prev, body = body, SC_IF.sub(one, body)
    return body


def context(active):
    ids = ["overview", "apps", "containers", "system", "backups", "alerts", "cleanup", "terminal", "files", "security", "settings"]
    return {
        "on": {i: i == active for i in ids},
        "off": {i: i != active for i in ids},
        "isDark": True,
        "isLight": False,
        "menuOpen": False,
    }


STATIC = {
    "lbl": "inline", "sep": "none", "navW": "248px", "chev": "0deg",
    "toggleLabel": "Collapse sidebar", "toggleAlign": "flex-end", "menuOpen": "false",
}


def render_body(name, theme, props=None, depth=0):
    props = props or {}
    helmet, body = read_board(name)
    theme = props.get("theme", theme)
    ctx = context(props.get("active", "overview"))
    ctx["isDark"], ctx["isLight"] = theme != "light", theme == "light"

    def imp(m):
        a = dict(ATTR.findall(m.group(1)))
        if depth > 3 or "name" not in a:
            return ""
        sub = {k: v for k, v in a.items() if not v.startswith("{{")}
        sub["theme"] = theme
        return render_body(a["name"], theme, sub, depth + 1)[1]

    body = IMPORT.sub(imp, body)
    body = resolve_ifs(body, ctx, props.get("active"))
    labels = {"themeLabel": "Dark" if theme == "light" else "Light",
              "themeAria": "Switch to dark theme" if theme == "light" else "Switch to light theme"}

    def bind(m):
        k = m.group(1)
        if k == "theme":
            return theme
        return labels.get(k, STATIC.get(k, ""))

    return helmet, BIND.sub(bind, body)


def page_html(name, theme):
    helmet, body = render_body(name, theme)
    helmet = re.sub(r"<link[^>]*fonts\.googleapis[^>]*>", "", helmet)
    faces = (
        '@font-face{font-family:"Geist";font-weight:100 900;src:url("file://%s/Geist.woff2") format("woff2")}'
        '@font-face{font-family:"Geist Mono";font-weight:100 900;src:url("file://%s/GeistMono.woff2") format("woff2")}' % (FONTS, FONTS)
    )
    return ('<!doctype html><html lang="en" data-theme="%s"><head><meta charset="utf-8"><base href="file://%s/">'
            "<style>%s</style>%s</head><body>%s</body></html>" % (theme, PROJECT, faces, helmet, body))


def find_chrome():
    if os.environ.get("OPSBOX_CHROME"):
        return os.environ["OPSBOX_CHROME"]
    pats = [
        os.path.expanduser("~/Library/Caches/ms-playwright/chromium-*/chrome-mac*/*/Contents/MacOS/*"),
        os.path.expanduser("~/.cache/ms-playwright/chromium-*/chrome-linux*/chrome"),
    ]
    for p in pats:
        hit = sorted(glob.glob(p))
        if hit:
            return hit[-1]
    return None  # let Playwright use its own download


def main(argv):
    from playwright.sync_api import sync_playwright

    boards = argv or sorted(os.path.basename(p)[:-8] for p in glob.glob(os.path.join(PROJECT, "*.dc.html")))
    os.makedirs(OUT, exist_ok=True)
    with sync_playwright() as p:
        kw = {"executable_path": find_chrome()} if find_chrome() else {}
        browser = p.chromium.launch(**kw)
        for name in boards:
            for theme in THEMES:
                page = browser.new_page(viewport={"width": 1440, "height": 900})
                tmp = os.path.join(OUT, ".%s-%s.html" % (name, theme))
                with open(tmp, "w", encoding="utf-8") as f:
                    f.write(page_html(name, theme))
                page.goto("file://" + tmp)
                page.evaluate("document.fonts.ready")
                page.wait_for_timeout(150)
                dest = os.path.join(OUT, "%s-%s.png" % (name, theme))
                page.screenshot(path=dest, full_page=True)
                os.remove(tmp)
                page.close()
                print("wrote", os.path.relpath(dest, ROOT))
        browser.close()


if __name__ == "__main__":
    main(sys.argv[1:])
