#!/usr/bin/env python3
"""Generate agent crate, symbol, route, privileged-operation, store and UI maps.

This is a lexical map, not Rust name resolution: public declarations in private
modules are useful navigation targets too. Platform/feature alternatives are kept;
test-only items, comments, literals and generated sources are excluded. Cargo's
ordinary package fields and dependency tables are scanned without invoking Cargo.
"""

import argparse
from dataclasses import dataclass
import difflib
import fnmatch
from functools import lru_cache
from pathlib import Path
import re
import sqlite3
import sys


ROOT = Path(__file__).resolve().parents[1]
START, END = "<!-- gen:start -->", "<!-- gen:end -->"
HEADER = """# Project index

Start here to locate a crate, then open its symbol map or entry file.
Regenerate with `python3 scripts/gen-index.py`; verify with
`python3 scripts/gen-index.py --check`. Edit prose outside the generated markers.

"""
SKIP = {".git", "target", "tests", "node_modules", "vendor", "generated"}
GENERATED = re.compile(r"(?im)^\s*(?://[!/]*|/\*+|\*)\s*.*(?:@generated|auto[- ]?generated|do not edit)")
# Concise descriptions for terse toolkit exports lacking adjacent documentation.
UI_SUMMARIES = {
    "DUR": "Shared animation durations.", "pct": "Format a percentage.",
    "esc": "Escape text for HTML.", "raw": "Wrap trusted markup without escaping.",
    "bytes": "Format a byte count with a unit.", "gb": "Format gigabytes.",
    "ago": "Format elapsed time since a timestamp.", "clock": "Format a local clock time.",
    "reducedMotion": "Read the user's reduced-motion preference.",
    "bar": "Render a usage meter with an accessible label.",
    "patch": "Update existing DOM while preserving keyed elements and animating changes.",
    "reportPoll": "Publish polling success and the last successful update time.",
    "onLive": "Subscribe to live polling state; return an unsubscribe function.",
    "liveState": "Read the current live polling state.", "chk": "Render a check-result indicator.",
    "btn": "Render a button or action link with state and accessibility attributes.",
    "iconAct": "Render an accessible icon action button.",
    "withBusy": "Run asynchronous work while showing a button's busy state.",
    "section": "Render a titled content section.", "header": "Render a page heading and actions.",
    "sw": "Render an accessible toggle switch.", "setSwitch": "Update a toggle's checked state.",
    "legend": "Render labelled colour swatches.", "code": "Render a formatted code block.",
    "skeletonSection": "Render placeholders for a loading section.",
    "skeletonPage": "Render a page-shaped loading placeholder.",
    "numberField": "Render a labelled numeric input with limits and step.",
    "dropzone": "Render a file drop area and browse control.",
    "confirmDialog": "Ask for confirmation, optionally requiring typed text.",
    "steps": "Render a stepped SVG line chart.",
    "bindActions": "Delegate UI actions and manage asynchronous button state.",
    "formDialog": "Collect and validate fields in a modal form.",
    "download": "Download text as a browser file.",
    "keepOrUndoDialog": "Offer Keep or Undo before a provisional change expires.",
    "initUi": "Install shared UI event handlers and accessibility behavior.",
}
ITEM = re.compile(
    r"\bpub\s+(?:(?:async|unsafe|default|extern|const)\s+)*"
    r"(?P<kind>fn|struct|enum|trait|type|const|mod)\s+(?P<name>(?:r#)?[A-Za-z_]\w*)"
)
LEXEME = re.compile(
    r"//[^\n]*|/\*|(?:br|cr|r)\#*\"|(?:b|c)?\"|"
    r"(?:b)?'(?:\\(?:[^\n]|u\{[0-9a-fA-F]+\})|[^'\\\n])'"
)


def blank(text):
    return re.sub(r"[^\n]", " ", text)


def mask_rust(source, keep_literals=False):
    """Mask comments/literals without moving offsets; retain outer doc locations."""
    parts, docs, pos = [], [], 0
    while True:
        match = LEXEME.search(source, pos)
        if not match:
            parts.append(source[pos:])
            break
        start, token = match.start(), match.group()
        parts.append(source[pos:start])
        end = match.end()
        if token == "/*":
            depth = 1
            while depth and end < len(source):
                nested = re.search(r"/\*|\*/", source[end:])
                if not nested:
                    end = len(source)
                    break
                depth += 1 if nested.group() == "/*" else -1
                end += nested.end()
            if source.startswith("/**", start) and not source.startswith("/***", start):
                content = source[start + 3:end - 2].strip()
                docs.append((start, end, content.splitlines()[0].lstrip("* ") if content else ""))
        elif token.startswith("//"):
            if token.startswith("///") and not token.startswith("////"):
                docs.append((start, end, token[3:].strip()))
        elif token.endswith('"'):
            if re.match(r"(?:br|cr|r)", token):
                closing = '"' + "#" * token.count("#")
                found = source.find(closing, end)
                end = len(source) if found < 0 else found + len(closing)
            else:
                closing = re.search(r'\\[\s\S]|"', source[end:])
                while closing:
                    end += closing.end()
                    if closing.group() == '"':
                        break
                    closing = re.search(r'\\[\s\S]|"', source[end:])
                if not closing:
                    end = len(source)
        parts.append(source[start:end] if keep_literals and not token.startswith(("//", "/*"))
                     else blank(source[start:end]))
        pos = end
    return "".join(parts), docs


def balanced_end(code, start, opening, closing):
    depth = 0
    for at in range(start, len(code)):
        if code[at] == opening:
            depth += 1
        elif code[at] == closing:
            depth -= 1
            if depth == 0:
                return at + 1
    raise ValueError("unbalanced Rust delimiters")


def attributes(code):
    result, pos = [], 0
    while True:
        match = re.search(r"#\s*!?\s*\[", code[pos:])
        if not match:
            return result
        start = pos + match.start()
        bracket = pos + match.end() - 1
        end = balanced_end(code, bracket, "[", "]")
        result.append((start, end, code[bracket + 1:end - 1]))
        pos = end


def test_only(condition):
    """Whether a cfg is definitely false without tests (other cfgs are unknown)."""
    condition = condition.strip()
    if condition == "test":
        return True
    match = re.fullmatch(r"(all|any)\s*\((.*)\)", condition, re.S)
    if not match:
        return False
    args, pos, depth = [], 0, 0
    for at, char in enumerate(match[2] + ","):
        depth += (char == "(") - (char == ")")
        if char == "," and depth == 0:
            if match[2][pos:at].strip():
                args.append(test_only(match[2][pos:at]))
            pos = at + 1
    return (any(args) if match[1] == "all" else bool(args) and all(args))


def production_code(source, keep_literals=False):
    code, docs = mask_rust(source)
    attrs = attributes(code)
    spans = []
    for start, end, body in attrs:
        cfg = re.fullmatch(r"\s*cfg\s*\((.*)\)\s*", body, re.S)
        is_test = re.fullmatch(r"\s*(?:\w+::)*test(?:\s*\(.*\))?\s*", body, re.S)
        if not is_test and not (cfg and test_only(cfg[1])):
            continue
        if spans and start < spans[-1][1]:
            continue
        # Skip the attributed item, not the rest of the file: test helpers can
        # appear between production items. Literals/comments are already blank.
        pos = end
        for attr_start, attr_end, _ in attrs:
            if attr_start >= pos and not code[pos:attr_start].strip():
                pos = attr_end
        while pos < len(code):
            char = code[pos]
            if char in "([":
                pos = balanced_end(code, pos, char, ")" if char == "(" else "]")
            elif char == "{":
                pos = balanced_end(code, pos, "{", "}")
                break
            elif char == ";":
                pos += 1
                break
            else:
                pos += 1
        spans.append((start, pos))
    docs = [(doc_start, doc_end, text) for doc_start, doc_end, text in docs
            if not any(start <= doc_start < end or
                       (doc_end <= start and not code[doc_end:start].strip())
                       for start, end in spans)]
    if keep_literals:
        code, _ = mask_rust(source, keep_literals=True)
    for start, end in reversed(spans):
        code = code[:start] + blank(code[start:end]) + code[end:]
    return code, docs


@lru_cache(maxsize=None)
def glob_match(pattern, path):
    """Gitignore-style path glob: ordinary stars never cross a slash."""
    if not pattern:
        return not path
    if pattern[0] == "**":
        return glob_match(pattern[1:], path) or bool(path) and glob_match(pattern, path[1:])
    return bool(path) and fnmatch.fnmatchcase(path[0], pattern[0]) and glob_match(pattern[1:], path[1:])


class Ignores:
    def __init__(self, root):
        self.root, self.rules = root, []

    def load(self, directory):
        ignore = directory / ".gitignore"
        if not ignore.is_file():
            return
        base = directory.relative_to(self.root).parts
        for line in ignore.read_text(encoding="utf-8").splitlines():
            line = line.rstrip()
            if not line or line.startswith("#"):
                continue
            negate = line.startswith("!")
            if negate:
                line = line[1:]
            directory_only = line.endswith("/")
            line = line.rstrip("/")
            anchored = "/" in line
            line = re.sub(r"\\(.)", r"\1", line)
            pattern = tuple(line.lstrip("/").split("/"))
            self.rules.append((base, pattern, anchored, directory_only, negate))

    def ignored(self, path, is_dir):
        parts, ignored = path.relative_to(self.root).parts, False
        for base, pattern, anchored, directory_only, negate in self.rules:
            if parts[:len(base)] != base:
                continue
            relative = parts[len(base):]
            for count in range(1, len(relative) + 1):
                if directory_only and count == len(relative) and not is_dir:
                    continue
                candidate = relative[:count] if anchored else (relative[count - 1],)
                # abc/** ignores descendants, not abc itself; leaving the
                # directory traversable lets !abc/keep.rs work like Git.
                if anchored and pattern[-1] == "**" and glob_match(pattern[:-1], candidate):
                    continue
                if glob_match(pattern, candidate):
                    ignored = not negate
                    break
        return ignored


def source_files(root):
    ignores = Ignores(root)
    ignores.load(root)
    found = []

    def walk(directory):
        ignores.load(directory)
        for path in sorted(directory.iterdir()):
            if path.is_symlink():
                continue
            is_dir = path.is_dir()
            if path.name in SKIP or ignores.ignored(path, is_dir):
                continue
            if is_dir:
                walk(path)
            elif path.suffix == ".rs" or path.name == "Cargo.toml":
                found.append(path)

    walk(root / "crates")
    return sorted(found)


def manifest_fields(text):
    """Scan the simple string fields/dependency names used by workspace crates."""
    section, package, dependencies = "", {}, set()
    dependency_table = None
    for line in text.splitlines():
        table = re.fullmatch(r"\s*\[([^\n]+)\]\s*(?:#.*)?", line)
        if table:
            section = table[1]
            dependency_table = re.search(r"(?:^|\.)(?:build-)?dependencies(?:\.([\w-]+))?$", section)
            if dependency_table and dependency_table[1]:
                dependencies.add(dependency_table[1])
            continue
        field = re.match(r'\s*([\w-]+)(?:\.workspace)?\s*=\s*(.*)', line)
        if not field:
            continue
        key, value = field.groups()
        if section == "package" and key in {"name", "description"}:
            quoted = re.match(r"(['\"])(.*?)\1", value)
            if quoted:
                package[key] = quoted[2]
        if dependency_table and dependency_table[1] is None:
            alias = re.search(r'\bpackage\s*=\s*"([\w-]+)"', value)
            dependencies.add(alias[1] if alias else key)
        elif dependency_table and dependency_table[1]:
            name = dependency_table[1]
            if key == "package":
                package_name = re.match(r"(['\"])(.*?)\1", value)
                if package_name:
                    dependencies.discard(name)
                    dependencies.add(package_name[2])
    return package, dependencies


def safe_summary(text):
    """Only emit documentation, never values; redact accidental sensitive prose."""
    text = " ".join(text.split())
    text = re.sub(r"(?:/Users/|/home/)[^\s`<>\"']+", "[home path]", text)
    text = re.sub(r"(?i)\b(?:gh[pousr]_[\w]+|github_pat_[\w]+|sk-(?:live|test)-[\w-]+|AKIA[A-Z0-9]{16})\b", "[redacted]", text)
    text = re.sub(r"(?i)\b((?:token|password|secret|api[_-]?key)\s*[:=]\s*)[^\s,;]+", r"\1[redacted]", text)
    return text.replace("|", "\\|")


def symbols(path, code, docs):
    # Attributes may sit between a doc comment and its declaration.
    doc_code = list(code)
    for start, end, _ in attributes(code):
        doc_code[start:end] = blank(code[start:end])
    doc_code = "".join(doc_code)

    def summary(at):
        preceding = [(start, end, text) for start, end, text in docs
                     if end <= at and not doc_code[end:at].strip()]
        return safe_summary(preceding[0][2]) if preceding else ""

    rows = []
    for match in ITEM.finditer(code):
        rows.append((path, match["kind"], match["name"], summary(match.start()), False))
    if Path(path).name == "lib.rs":
        # Keep re-export lists compact (one row per statement) and prioritize
        # crate-root exports if this crate needs truncation.
        for match in re.finditer(r"\bpub\s+use\s+([^;]+);", code):
            name = " ".join(match[1].split())
            rows.append((path, "use", name, summary(match.start()),
                         len(Path(path).parts) == 4 and path.endswith("/src/lib.rs")))
    return rows


@dataclass
class Crate:
    name: str
    purpose: str
    dependencies: list
    entry: str
    loc: int
    items: list


def collect(root):
    files = source_files(root)
    crates = []
    for manifest in (path for path in files if path.name == "Cargo.toml"):
        package, deps = manifest_fields(manifest.read_text(encoding="utf-8"))
        if "name" not in package:
            continue
        entries = [manifest.parent / "src" / name for name in ("lib.rs", "main.rs")]
        entry = next((path for path in entries if path in files), None)
        if entry is None:
            raise ValueError("no indexable entry file for " + package["name"])
        source = entry.read_text(encoding="utf-8")
        purpose = re.search(r"(?m)^\s*//!\s*(\S[^\n]*)", source)
        if not purpose:
            print("TODO: add a //! line to " + entry.relative_to(root).as_posix(), file=sys.stderr)
        purpose = purpose[1] if purpose else package.get("description", "TODO: add a //! line")
        items, loc = [], 0
        for path in files:
            if path.suffix != ".rs" or manifest.parent not in path.parents:
                continue
            source = path.read_text(encoding="utf-8")
            if GENERATED.search("\n".join(source.splitlines()[:12])):
                continue
            code, docs = production_code(source)
            loc += sum(bool(line.strip()) for line in code.splitlines())
            items.extend(symbols(path.relative_to(root).as_posix(), code, docs))
        crates.append(Crate(package["name"], safe_summary(purpose), sorted(deps),
                            entry.relative_to(root).as_posix(), loc, items))
    names = {crate.name for crate in crates}
    for crate in crates:
        crate.dependencies = sorted(names.intersection(crate.dependencies))
    return sorted(crates, key=lambda crate: crate.name)


def symbol_map(crate):
    title = ["# " + crate.name + " symbols", "", "Generated by `scripts/gen-index.py`; paths are relative to the repository root.",
             "Public declarations (including methods); test-only/generated code omitted.", ""]
    rows = sorted(crate.items, key=lambda row: row[:4])
    # The entire file, including headings, blank lines and omission notice, is
    # bounded. In large crates retain root re-exports, then documented items.
    budget = 400 - len(title) - 2  # fenced block delimiters
    omitted = 0
    if len(rows) > budget:
        eligible = sorted((row for row in rows if row[4] or row[3]),
                          key=lambda row: (not row[4], row[:4]))
        retained = eligible[:budget - 1]
        omitted = len(rows) - len(retained)
        rows = sorted(retained, key=lambda row: row[:4])
    lines = title + ["```text"] + [f"{path}: {kind} {name}  {summary}".rstrip()
                                   for path, kind, name, summary, _ in rows] + ["```"]
    if omitted:
        lines.append(f"(truncated, {omitted} more)")
    return "\n".join(lines) + "\n"


def index_block(root, crates):
    lines = ["## Task maps", "", "- [HTTP and WebSocket routes](index/routes.md): handlers, permissions, API call links and documentation drift.",
             "- [Privileged operations](index/privops.md): request types, helper dispatch, client methods and timeouts.",
             "- [Store](index/store.md): current columns, indexes, migrations, Store methods and settings keys.",
             "- [UI](index/ui.md): pages, hashes, API functions, styles and shared helpers; mock backlog first.",
             "", "## Layout", "", "| Folder | Contents |", "|---|---|"]
    for directory, purpose in (
        ("crates", "Rust workspace: binary, privileged helper and shared libraries."),
        ("web", "Embedded vanilla-JS panel, styles, assets and UI checks."),
        ("site", "Public landing site."),
        ("design", "Panel design references and preview generators."),
        ("docs", "Specs, decisions, status, prompts and generated navigation maps."),
        ("scripts", "Repository maintenance and verification scripts."),
        ("packaging", "System service units and distribution packaging."),
        ("tools", "Build, API inventory, screenshot and VM verification tools."),
    ):
        lines.append(f"| [{directory}/](../{directory}/) | {purpose} |")
    lines += ["", "## Crates", "", "LOC counts nonblank Rust code lines, excluding comments/literals and test-only items.",
              "Dependencies include normal/build/target tables; dev dependencies are omitted.",
              "", "| Crate / symbols | Purpose | Internal dependencies | Non-test LOC | Entry |",
              "|---|---|---|---:|---|"]
    for crate in crates:
        deps = ", ".join(crate.dependencies) or "—"
        lines.append(f"| [{crate.name}](index/symbols/{crate.name}.md) | {crate.purpose} | {deps} | {crate.loc:,} | [{crate.entry}](../{crate.entry}) |")
    lines += ["", "## Dependency graph", "", "Each indented row is a direct dependency of the crate above it; read upward for callers.", "", "```text"]
    for crate in crates:
        lines.append(crate.name)
        lines.extend("  " + dependency for dependency in crate.dependencies)
    lines += ["```", "", "## Runtime roles", "", "All roles start in the `opsbox` crate (`crates/opsbox/src/main.rs`).",
              "`serve` delegates system actions to typed operations in the root helper.", "",
              "| Role | Startup | Runtime crate |", "|---|---|---|"]
    main = (root / "crates/opsbox/src/main.rs").read_text(encoding="utf-8")
    for role, variant, runtime in (("serve", "Serve", "opsbox-api"), ("priv", "Priv", "opsbox-priv"),
                                   ("proxy", "Proxy", "opsbox-proxy"), ("watchdog", "Watchdog", "opsbox-watchdog")):
        route = re.search(r"Command::" + variant + r"\([^\n]*\)\s*=>\s*\{(.*?)\n\s*\}", main, re.S)
        call = re.search(r"cmd::(\w+)::run\(", route[1]) if route else None
        if not call:
            raise ValueError("cannot locate startup for " + role)
        path = "crates/opsbox/src/cmd/" + call[1] + ".rs"
        if runtime.replace("-", "_") not in (root / path).read_text(encoding="utf-8"):
            raise ValueError("runtime crate changed for " + role)
        label = role + (" (root helper)" if role == "priv" else "")
        lines.append(f"| `{label}` | [{path}](../{path}) | {runtime} |")
    return "\n".join(lines) + "\n"


@dataclass
class Function:
    name: str
    start: int
    params: int
    body: int
    end: int


def functions(code, pattern=r"\bfn\s+(\w+)"):
    """Read balanced signatures/bodies, including destructured/default arguments."""
    result = []
    for match in re.finditer(pattern, code):
        params = code.find("(", match.end())
        if params < 0:
            continue
        after = balanced_end(code, params, "(", ")")
        body = re.search(r"[;{]", code[after:])
        if not body or body[0] == ";":
            continue
        body = after + body.start()
        result.append(Function(match[1], match.start(), params, body,
                               balanced_end(code, body, "{", "}")))
    return result


def split_args(code, start=0, end=None):
    """Offsets of comma-separated arguments with nested delimiters masked or balanced."""
    end = len(code) if end is None else end
    result, pos, at = [], start, start
    while at < end:
        if code[at] in "([{":
            at = balanced_end(code, at, code[at], {"(": ")", "[": "]", "{": "}"}[code[at]])
        elif code[at] == ",":
            result.append((pos, at))
            pos = at + 1
            at += 1
        else:
            at += 1
    # A final argument can consist entirely of a masked string literal.
    if code[pos:end].strip() or (pos < end and not result):
        result.append((pos, end))
    return result


def rust_sources(root, directory):
    result = {}
    for path in sorted((root / directory).rglob("*.rs")):
        raw = path.read_text(encoding="utf-8")
        if not GENERATED.search("\n".join(raw.splitlines()[:12])):
            result[path.relative_to(root).as_posix()] = (
                production_code(raw)[0], production_code(raw, keep_literals=True)[0])
    return result


def table(headers, rows):
    def cell(value):
        return " ".join(str(value).split()).replace("|", "\\|") or "—"
    return ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|"] + [
        "| " + " | ".join(cell(value) for value in row) + " |" for row in rows]


def map_header(title, prose):
    return ["# " + title, "", "Generated by `scripts/gen-index.py`; paths are relative to the repository root.",
            prose, ""]


def privop_data(root):
    proto = rust_sources(root, "crates/opsbox-proto/src")
    variants = {}
    for path, (code, text) in proto.items():
        match = re.search(r"\benum\s+PrivOp\s*\{", code)
        if not match:
            continue
        start = code.index("{", match.start())
        end = balanced_end(code, start, "{", "}")
        for lo, hi in split_args(code, start + 1, end - 1):
            chunk = re.sub(r"#\[[^]]*\]", "", code[lo:hi]).strip()
            variant = re.match(r"(\w+)\s*([({]?)", chunk)
            if not variant:
                continue
            name, shape = variant.groups()
            # No values are emitted: inline request fields contain only names/types.
            request = "unit"
            if shape == "(":
                request = chunk[chunk.index("(") + 1:chunk.rindex(")")].strip()
            elif shape == "{":
                request = "inline { " + " ".join(chunk[chunk.index("{") + 1:chunk.rindex("}")].split()) + " }"
            variants[name] = {"request": request, "handlers": [], "clients": []}
    sources = rust_sources(root, "crates/opsbox-priv/src")
    for path, (code, text) in sources.items():
        if path.endswith("/client.rs"):
            client_functions = functions(code)
            for fn in client_functions:
                body = code[fn.body:fn.end]
                timeout = re.search(r"\.with_timeout\s*\((.*?)\)\s*\)", body, re.S)
                suffix = " [timeout " + " ".join(timeout[1].split()) + ")]" if timeout else ""
                for name in sorted(set(re.findall(r"\bPrivOp::(\w+)", body)) & variants.keys()):
                    variants[name]["clients"].append(fn.name + suffix)
            # Convenience methods can delegate to a method constructing the enum.
            changed = True
            while changed:
                changed = False
                for fn in client_functions:
                    called = set(re.findall(r"\bself\s*\.\s*(\w+)\s*\(", code[fn.body:fn.end]))
                    for data in variants.values():
                        names = {client.split()[0] for client in data["clients"]}
                        if fn.name not in names and called & names:
                            data["clients"].append(fn.name + " [delegates]")
                            changed = True
        # Include module dispatch matches, not just ops/mod.rs; grouped arms are expanded.
        if "/ops/" in path:
            for fn in functions(code):
                if fn.name != "dispatch":
                    continue
                dispatch = re.search(r"\bmatch\s+op\s*\{", code[fn.body:fn.end])
                if not dispatch:
                    continue
                match_start = fn.body + dispatch.end()
                match_end = balanced_end(code, match_start - 1, "{", "}")
                for match in re.finditer(r"\bPrivOp::(\w+)", code[match_start:match_end]):
                    name, at = match[1], match_start + match.start()
                    if name not in variants:
                        continue
                    arrow = code.find("=>", at, fn.end)
                    if arrow < 0:
                        continue
                    begin = arrow + 2
                    if code[begin:].lstrip().startswith("{"):
                        begin = code.index("{", begin)
                        finish = balanced_end(code, begin, "{", "}")
                    else:
                        finish = next((hi for lo, hi in split_args(code, begin, fn.end - 1)), fn.end - 1)
                    arm = code[begin:finish]
                    calls = re.findall(r"\b([a-z_]\w*::[a-z_]\w*)\s*\(", arm)
                    calls = [call for call in calls if not call.startswith(("PrivReply::", "PrivErrorCode::"))]
                    handler = calls[0] if calls and path.endswith("/ops/mod.rs") else Path(path).stem + "::dispatch"
                    if handler == "mod::dispatch":
                        handler = "ops::dispatch (inline)"
                    target = str(Path(path).parent / (handler.split("::")[0] + ".rs"))
                    if target not in sources:
                        target = path
                    variants[name]["handlers"].append((handler, target))
        if path.endswith("/server.rs"):
            for match in re.finditer(r"if\s+let\s+PrivOp::(\w+)", code):
                name = match[1]
                if name not in variants:
                    continue
                # Pattern braces precede the condition body: locate its assignment first.
                assignment = code.index("=", match.end())
                start = code.index("{", assignment)
                end = balanced_end(code, start, "{", "}")
                call = re.search(r"\b([a-z_]\w*)\s*\(", code[start + 1:end])
                if call:
                    variants[name]["handlers"] = [(call[1] + " (stream)", path)]
    return variants


def privops_map(root, variants):
    lines = map_header("Privileged operations", "Protocol: `crates/opsbox-proto/src/privilege.rs`. Client: `crates/opsbox-priv/src/client.rs`; default CALL_TIMEOUT is 10 seconds (unless the caller overrides it). Dispatch entries include streaming interception and grouped module dispatch.")
    rows, unwired = [], []
    for name, data in sorted(variants.items()):
        handlers = sorted(set(data["handlers"]))
        # Prefer the specific module dispatch over the top-level forwarding arm.
        specific = [row for row in handlers if not row[0].endswith("::dispatch")]
        handlers = specific or handlers
        rows.append((name, data["request"], "; ".join(sorted({row[0] for row in handlers})),
                     "; ".join(sorted({row[1] for row in handlers})), "; ".join(sorted(set(data["clients"])))))
        missing = [label for key, label in (("handlers", "handler"), ("clients", "client method")) if not data[key]]
        if missing:
            unwired.append(name + ": no " + " or ".join(missing))
    lines += table(["Variant", "Request type", "Handler fn", "File", "Client method / timeout override"], rows)
    lines += ["", "## Unwired", "", "No dedicated client method does not mean unreachable: callers can use `PrivClient::call` with a typed variant.", ""] + (["- " + value for value in unwired] or ["None."])
    return "\n".join(lines) + "\n"


def route_data(root):
    sources = rust_sources(root, "crates/opsbox-api/src")
    registry, by_name = {}, {}
    for path, (code, text) in sources.items():
        module = Path(path).stem
        for fn in functions(code):
            key = module + "::" + fn.name
            registry[key] = (path, fn, code, text)
            by_name.setdefault(fn.name, []).append(key)

    def resolve(name, module):
        if "::" in name:
            return name.removeprefix("crate::")
        local = module + "::" + name
        return local if local in registry else next(iter(by_name.get(name, [])), local)

    def router(key, prefix="", stack=()):
        if key not in registry or key in stack:
            raise ValueError("cannot resolve nested router " + key)
        path, fn, code, text = registry[key]
        rows = []
        for match in re.finditer(r"\.(route|nest)\s*\(", code[fn.body:fn.end]):
            start = fn.body + match.end()
            end = balanced_end(code, start - 1, "(", ")")
            args = split_args(code, start, end - 1)
            literal = re.fullmatch(r'\s*"([^"\n]*)"\s*', text[slice(*args[0])])
            if len(args) != 2 or not literal:
                raise ValueError("unsupported route/nest in " + path)
            url = prefix + literal[1]
            lo, hi = args[1]
            if match[1] == "nest":
                nested = re.search(r"([\w:]+)\s*\(", code[lo:hi])
                if not nested:
                    raise ValueError("unsupported nested router in " + path)
                rows += router(resolve(nested[1], key.split("::")[0]), url, stack + (key,))
                continue
            handlers = re.findall(r"\b(get|post|put|patch|delete|head|options|trace|any)\s*\(\s*([\w:]+)\s*\)", code[lo:hi])
            if not handlers:
                raise ValueError("unsupported route handler in " + path)
            for method, handler in handlers:
                target = resolve(handler, key.split("::")[0])
                if target not in registry:
                    raise ValueError("cannot resolve handler " + target)
                file, handler_fn, handler_code, _ = registry[target]
                signature = handler_code[handler_fn.params:handler_fn.body]
                auth = list(dict.fromkeys(re.findall(r"\b(Admin|Deployer|Viewer|Client|Enrolling|ToolGateway)\b", signature)))
                body = handler_code[handler_fn.body:handler_fn.end]
                if "deny_token" in body:
                    auth.append("token refused")
                if "WebSocketUpgrade" in signature:
                    method += " (WS)"
                rows.append((target.split("::")[0], method.upper(), url, target, file, ", ".join(auth) or "public"))
        return rows
    rows = router("lib::router") if "lib::router" in registry else []
    return sorted(set(rows)), registry, by_name


def routes_map(root, routes, registry, by_name, variants, store_methods):
    lines = map_header("HTTP and WebSocket routes", "Auth lists signature extractors (Viewer included); Client records IP/user-agent, and is not an authorization check. Public means no role extractor; handler checks and global setup, Origin/CSRF, token-scope and allowlist middleware still apply. WebSocket rows retain their GET upgrade method. Grouped by handler module.")
    for module in sorted({row[0] for row in routes}):
        lines += ["## " + module, ""] + table(["Method", "Path", "Handler", "File", "Auth"],
            [row[1:] for row in routes if row[0] == module]) + [""]
    # Direct call links deliberately keep intermediate functions, so readers can trace
    # background enqueue paths without implying that every branch runs per request.
    client_methods = {re.match(r"\w+", client)[0] for data in variants.values() for client in data["clients"]}
    links = {}
    for key, (path, fn, code, text) in registry.items():
        body = code[fn.body:fn.end]
        calls = set()
        for call in re.finditer(r"(?<![.\w:])([a-z_]\w*(?:::[a-z_]\w*)*)\s*\(", body):
            name = call[1].removeprefix("crate::")
            local = key.split("::")[0] + "::" + name
            target = name if name in registry else local if local in registry else None
            if target is None and len(by_name.get(name, [])) == 1:
                target = by_name[name][0]
            if target and target != key:
                calls.add(target)
        methods = set(re.findall(r"\.\s*(\w+)\s*\(", body))
        helpers = sorted(methods & client_methods)
        generic = sorted(set(re.findall(r"\bPrivOp::(\w+)", body)) & variants.keys())
        helpers += ["call(PrivOp::" + name + ")" for name in generic]
        stores = sorted(methods & store_methods)
        requests = sorted(set(re.findall(r"(?:ApiJson|Json)\s*<\s*(\w+)", code[fn.params:fn.body])))
        links[key] = (path, sorted(calls), helpers, stores, requests)
    reachable, pending = set(), [row[3] for row in routes]
    while pending:
        key = pending.pop()
        if key not in reachable:
            reachable.add(key)
            pending.extend(links[key][1])
    lines += ["## API call links", "", "Lexical direct calls reachable from route handlers, including background functions. Method names are navigation candidates, not type resolution; follow calls to intermediate helpers. PrivClient methods map to variants in `privops.md`; Store methods map to tables in `store.md`.", ""]
    lines += table(["Function", "File", "Request body", "Calls", "PrivClient methods", "Store methods"], [
        (key, links[key][0], ", ".join(links[key][4]), ", ".join(links[key][1]),
         ", ".join(links[key][2]), ", ".join(links[key][3])) for key in sorted(reachable)])
    capacity = root / "crates/opsbox-api/src/capacity.rs"
    lines += ["", "## Capacity middleware", "", "`capacity::guard` in `crates/opsbox-api/src/capacity.rs` holds admission through persistent enqueue. Its matched method/path patterns (underscore = dynamic segment):", ""]
    if capacity.exists():
        source = capacity.read_text(encoding="utf-8")
        matched = re.search(r"let interested\s*=\s*matches!\((.*?)\n\s*\);", source, re.S)
        if matched:
            for method, segments in re.findall(r'\("(\w+)",\s*\[([^]]+)\]\)', matched[1]):
                segments = [part.strip().replace('"', '') for part in segments.split(",")]
                lines.append("- `" + method + " /api/v1/" + "/".join(segments) + "`")
        else:
            raise ValueError("cannot parse capacity middleware route patterns")
    documented = set()
    api_doc = root / "docs/API.md"
    if api_doc.exists():
        for method, path in re.findall(r"\b(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS|WS)\s+(/[^\s`?]*)", api_doc.read_text(encoding="utf-8")):
            if not path.startswith(("/api/", "/health", "/ready", "/tools")):
                path = "/api/v1" + path
            documented.add(("GET" if method == "WS" else method, path.rstrip(".,;")))
    actual = {(row[1].split()[0], row[2]) for row in routes}
    lines += ["", "## Drift", "", "Informational comparison with explicit method/path mentions in `docs/API.md`; query strings ignored, relative paths use `/api/v1`. Prose shorthand without a method is not expanded. Drift does not independently fail `--check`.", "", "### In code, absent from API.md", ""]
    lines += ["- `" + method + " " + path + "`" for method, path in sorted(actual - documented)] or ["None."]
    lines += ["", "### In API.md, absent from code", ""]
    lines += ["- `" + method + " " + path + "`" for method, path in sorted(documented - actual)] or ["None."]
    return "\n".join(lines) + "\n"


def store_data(root):
    sources = rust_sources(root, "crates/opsbox-store/src")
    methods = {}
    for path, (code, text) in sources.items():
        impls = []
        for match in re.finditer(r"\bimpl\s+Store\s*\{", code):
            start = code.index("{", match.start())
            impls.append((start, balanced_end(code, start, "{", "}")))
        for fn in functions(code):
            if any(lo < fn.start < hi for lo, hi in impls):
                methods[fn.name] = (path, fn, code, text)
    return sources, methods


def store_map(root, sources, methods):
    lines = map_header("Store schema and methods", "Schema is reconstructed in an in-memory SQLite database from ordered migrations, including ALTER TABLE columns and implicit primary/unique indexes. No application database is opened. Method associations scan literal table-name mentions in production Store methods; delegated calls are listed separately.")
    migrations, created = [], {}
    for path in sorted((root / "crates/opsbox-store/migrations").glob("*.sql")):
        migrations.append((path.relative_to(root).as_posix(), path.read_text(encoding="utf-8")))
    # Include the migration runner's bookkeeping table, which is not in a .sql file.
    for path, (code, text) in sources.items():
        match = re.search(r"CREATE TABLE IF NOT EXISTS schema_migrations\s*\(.*?\)\s*STRICT;", text, re.S)
        if match:
            migrations.insert(0, (path + " (bookkeeping)", match[0]))
    with sqlite3.connect(":memory:") as connection:
        for path, sql in migrations:
            clean = re.sub(r"--[^\n]*|/\*.*?\*/", "", sql, flags=re.S)
            for name in re.findall(r"(?i)CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?(\w+)", clean):
                created.setdefault(name, path)
            connection.executescript(sql)
            for old, new in re.findall(r"(?i)ALTER\s+TABLE\s+(\w+)\s+RENAME\s+TO\s+(\w+)", clean):
                created.setdefault(new, created.get(old, path))
        rows = []
        current = {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type = 'table'")}
        for name, path in sorted(created.items()):
            if name not in current:
                continue  # Rebuild migrations leave no temporary tables behind.
            columns = ", ".join(row[1] + " " + row[2] for row in connection.execute('PRAGMA table_info("' + name + '")'))
            indexes = []
            for row in connection.execute('PRAGMA index_list("' + name + '")'):
                sql = connection.execute("SELECT sql FROM sqlite_master WHERE name = ?", (row[1],)).fetchone()[0]
                cols = ", ".join(col[2] or "expression" for col in connection.execute('PRAGMA index_info("' + row[1] + '")'))
                indexes.append(" ".join(sql.split()) if sql else ("UNIQUE (" + cols + ") [implicit]"))
            touches = [method + " (" + value[0] + ")" for method, value in sorted(methods.items())
                       if re.search(r"\b" + re.escape(name) + r"\b", value[3][value[1].body:value[1].end])]
            rows.append((name, columns, "; ".join(sorted(indexes)), path, "; ".join(touches)))
    lines += table(["Table", "Columns / types (current)", "Indexes", "Created in", "Store methods mentioning table"], rows)
    lines += ["", "## Store method calls", "", "Follow delegation when a public method does not contain SQL itself.", ""]
    calls = []
    for name, (path, fn, code, text) in sorted(methods.items()):
        delegated = sorted(set(re.findall(r"\.\s*(\w+)\s*\(", code[fn.body:fn.end])) & methods.keys() - {name})
        if delegated:
            calls.append((name, path, ", ".join(delegated)))
    lines += table(["Store method", "File", "Calls Store methods"], calls)
    # Settings reads live throughout the Rust workspace, not just in the store crate.
    keys = {}
    for path in source_files(root):
        if path.suffix != ".rs":
            continue
        text = production_code(path.read_text(encoding="utf-8"), keep_literals=True)[0]
        for key in re.findall(r'\bget_setting\s*\(\s*"([^"\n]+)"', text):
            keys.setdefault(key, set()).add(path.relative_to(root).as_posix())
    lines += ["", "## Settings keys", "", "Literal `get_setting(\"…\")` reads only; computed keys and values are omitted.", ""]
    lines += ["- `" + key + "`: " + ", ".join(sorted(paths)) for key, paths in sorted(keys.items())] or ["None."]
    return "\n".join(lines) + "\n"


def js_string_end(source, start):
    quote, pos = source[start], start + 1
    while pos < len(source):
        if source[pos] == "\\":
            pos += 2
        elif source[pos] == quote:
            return pos + 1
        elif quote == "`" and source.startswith("${", pos):
            pos = js_expression_end(source, pos + 1)
        else:
            pos += 1
    raise ValueError("unterminated JavaScript string")


def js_token_end(source, pos):
    if source.startswith("//", pos):
        end = source.find("\n", pos)
        return len(source) if end < 0 else end
    if source.startswith("/*", pos):
        end = source.find("*/", pos + 2)
        return len(source) if end < 0 else end + 2
    if source[pos] in "'\"`":
        return js_string_end(source, pos)
    # Slash is a regex at expression starts; elsewhere it is division. Character
    # classes can include quotes, slashes and braces without ending the regex.
    before = source[:pos].rstrip() if source[pos] == "/" else ""
    if source[pos] == "/" and (not before or before[-1] in "=(:,[!&|?;{" or re.search(r"\b(?:return|case)$", before)):
        at, in_class = pos + 1, False
        while at < len(source) and source[at] != "\n":
            if source[at] == "\\":
                at += 2
                continue
            if source[at] == "[":
                in_class = True
            elif source[at] == "]":
                in_class = False
            elif source[at] == "/" and not in_class:
                at += 1
                while at < len(source) and source[at].isalpha():
                    at += 1
                return at
            at += 1
    return pos + 1


def js_expression_end(source, start):
    depth, at = 1, start + 1
    while at < len(source):
        end = js_token_end(source, at)
        if end == at + 1:
            depth += (source[at] == "{") - (source[at] == "}")
            if not depth:
                return at + 1
        at = end
    raise ValueError("unbalanced JavaScript template expression")


def mask_js(source):
    parts, at = [], 0
    while at < len(source):
        end = js_token_end(source, at)
        parts.append(blank(source[at:end]) if end > at + 1 else source[at:end])
        at = end
    return "".join(parts)


def js_functions(source, exported=False):
    code = mask_js(source)
    prefix = r"\bexport\s+" if exported else r"\b"
    return code, functions(code, prefix + r"(?:async\s+)?function\s+(\w+)")


def endpoint_path(expression):
    """Normalize only literal URL structure; never emit request/credential values."""
    expression = expression.strip()
    if not expression or expression[0] not in "'\"`":
        return None
    end = js_string_end(expression, 0)
    value = expression[1:end - 1]
    if expression[0] == "`":
        parts, at = [], 0
        while at < len(value):
            if value.startswith("${", at):
                finish = js_expression_end(value, at + 1)
                expr = value[at + 2:finish - 1]
                # Optional query string expressions do not change the endpoint path.
                if "?" not in expr and not re.search(r"URLSearchParams|\w*Query\(|^\s*qs\b", expr):
                    names = re.findall(r"\b[A-Za-z_]\w*(?:\.\w+)?\b", expr)
                    names = [name for name in names if name not in {"encodeURIComponent", "enc", "String"}]
                    parts.append("{" + (names[-1] if names else "dynamic") + "}")
                at = finish
            else:
                parts.append(value[at])
                at += 1
        value = "".join(parts)
    tail = expression[end:].strip()
    if tail.startswith("+") and not value.endswith("?") and "?" not in value:
        dynamic = tail[1:].strip()
        if value.endswith("/"):
            name = re.search(r"(?:encodeURIComponent|enc)\((\w+)\)|^(\w+)", dynamic)
            value += "{" + (next(group for group in name.groups() if group) if name else "dynamic") + "}"
    return value.split("?")[0] if value.startswith("/") else None


def js_endpoints(source, fn, code):
    rows = set()
    for call in re.finditer(r"\b(http|fetch|WebSocket|changeFirewall)\s*\(", code[fn.body:fn.end]):
        start = fn.body + call.end()
        end = balanced_end(code, start - 1, "(", ")")
        args = split_args(code, start, end - 1)
        expression = source[slice(*args[0])].strip() if args else ""
        paths = [endpoint_path(expression)]
        if paths == [None]:
            # A named/conditional path can select between literal endpoints.
            if re.fullmatch(r"\w+", expression):
                assignment = re.search(r"\b(?:const|let)\s+" + re.escape(expression) + r"\s*=([^;]+);",
                                       source[fn.body:fn.end])
                expression = assignment[1] if assignment else expression
            paths = []
            at = 0
            while at < len(expression):
                end = js_token_end(expression, at)
                if expression[at] in "'\"`":
                    paths.append(endpoint_path(expression[at:end]))
                at = end
        if call[1] == "WebSocket":
            # Origin is computed; retain just the API path inside the template.
            found = expression.find("/api/v1/")
            if found >= 0:
                paths = [endpoint_path("`" + expression[found:].rstrip("`") + "`")]
        paths = [path for path in paths if path]
        if not paths:
            continue
        method = "GET (WS)" if call[1] == "WebSocket" else "GET"
        if len(args) > 1:
            options = source[slice(*args[1])]
            match = re.search(r"\bmethod\s*:\s*['\"](\w+)['\"]", options)
            if match:
                method = match[1]
        for path in paths:
            if call[1] in {"http", "changeFirewall"}:
                path = "/api/v1" + path
            rows.add((method, path))
    return sorted(rows)


def ui_map(root):
    root = root.resolve()
    api_path = root / "web/js/api.js"
    if not api_path.exists():
        return "\n".join(map_header("UI map", "No web/js/api.js present."))
    api = api_path.read_text(encoding="utf-8")
    code, all_functions = js_functions(api)
    _, exported = js_functions(api, exported=True)
    functions_by_name = {fn.name: fn for fn in all_functions}
    # Arrow function exports are small local utilities rather than HTTP functions.
    arrow_exports = re.findall(r"\bexport\s+const\s+(\w+)\s*=\s*(?:async\s*)?\([^;]*?=>", code)
    for name in arrow_exports:
        match = re.search(r"\bexport\s+const\s+" + name + r"\s*=", code)
        end = code.find(";", match.end())
        api_f = Function(name, match.start(), match.end(), match.end(), end)
        # Expression-bodied arrows also participate in mode and URL detection below.
        exported.append(api_f)
    real_match = re.search(r"const REAL\s*=\s*new Set\s*\(\s*\[", code)
    real = set()
    if real_match:
        end = balanced_end(code, real_match.end() - 1, "[", "]")
        real = set(re.findall(r"['\"]([\w.]+)['\"]", api[real_match.end():end - 1]))
    api_fns = {fn.name: fn for fn in exported}
    # Exported auth object methods are part of the public API boundary too.
    auth = re.search(r"export const auth\s*=\s*\{", code)
    if auth:
        end = balanced_end(code, auth.end() - 1, "{", "}")
        for fn in functions(code[auth.end():end - 1], r"\b(?:async\s+)?(\w+)\s*(?=\()"):
            # Only methods at the object level, excluding calls in their bodies.
            offset = auth.end()
            shifted = Function("auth." + fn.name, fn.start + offset, fn.params + offset, fn.body + offset, fn.end + offset)
            if not any(existing.start < shifted.start < existing.end for existing in api_fns.values()):
                api_fns[shifted.name] = shifted
    main = (root / "web/js/main.js").read_text(encoding="utf-8")
    hashes = {}
    for pattern, page in re.findall(r"\[\s*/(\^.*?\$)/\s*,\s*['\"](\w+)['\"]\s*\]", main):
        # Keep the exact router regex, including optional/dynamic segments.
        hashes.setdefault(page, []).append("# matching /" + pattern + "/")
    html = (root / "web/index.html").read_text(encoding="utf-8")
    global_css = sorted({"web/" + name for name in re.findall(r'href=["\']([^"\']+\.css)["\']', html)})
    pages, uses = [], {}
    for path in sorted((root / "web/js/pages").glob("*.js")):
        source = path.read_text(encoding="utf-8")
        page_code = mask_js(source)
        names = sorted(set(re.findall(r"\bapi\.([A-Za-z_]\w*(?:\.\w+)?)", page_code)) & api_fns.keys())
        for name in names:
            uses.setdefault(name, set()).add(path.stem)
        css = set(global_css)
        # Capture explicit local CSS references (e.g. Monaco) and imported sheets.
        bases = {var: (path.parent / name).resolve() for var, name in re.findall(
            r"\bconst\s+(\w+)\s*=\s*new URL\(['\"]([^'\"]+)['\"],\s*import.meta.url\)", source)}
        for match in re.finditer(r"(?:(\w+)\s*\+\s*)?['\"]([^'\"\n]+\.css)['\"]", source):
            base = bases.get(match[1], path.parent)
            css.add((base / match[2]).resolve().relative_to(root).as_posix())
        # AMD modules can load a stylesheet with the same basename (Monaco).
        for var, base in bases.items():
            for name in re.findall(r"['\"](vs/[^'\"]+)['\"]", source):
                sheet = base / (name.removeprefix("vs/") + ".css")
                if sheet.is_file():
                    css.add(sheet.relative_to(root).as_posix())
        views = re.findall(r"\bexport\s+(?:async\s+)?function\s+(view)\b", page_code)
        pages.append((path.relative_to(root).as_posix(), "; ".join(hashes.get(path.stem, ["unrouted"])),
                      ", ".join(views), ", ".join(names), ", ".join(sorted(css))))
    def resolve_endpoints(fn, seen):
        if fn.name in seen:
            return set()
        found = set(js_endpoints(api, fn, code))
        body = code[fn.body:fn.end]
        if not found:
            calls = set(re.findall(r"(?<![.\w])([a-zA-Z_]\w*)\s*\(", body))
            calls.update(re.findall(r"\bauth\.(\w+)\s*\(", body))
            for name in calls & functions_by_name.keys() - {"http", "httpRequest", "waitTask"}:
                found.update(resolve_endpoints(functions_by_name[name], seen | {fn.name}))
            for name in re.findall(r"\bauth\.(\w+)\s*\(", body):
                target = api_fns.get("auth." + name)
                if target:
                    found.update(resolve_endpoints(target, seen | {fn.name}))
        return found

    rows, still_mock = [], []
    for name in sorted(set(api_fns) | set(arrow_exports)):
        fn = api_fns.get(name)
        body = code[fn.body:fn.end] if fn else ""
        mock = "sample(" in body and name not in real
        found = sorted(resolve_endpoints(fn, set())) if fn and not mock else []
        if name == "downloadUrl":
            # URL-returning exports do not call fetch themselves.
            match = re.search(r"`(/api/v1/[^`]+)`", api[fn.body:fn.end])
            if match:
                found = [("GET", endpoint_path("`" + match[1] + "`"))]
        # Some streams and direct HTTP utilities use MODE rather than REAL membership.
        is_real = name in real or bool(found) and ("MODE" in body or "REAL.has" in body or name == "reauth")
        local = not found and "sample(" not in body and name not in real
        state = "REAL" if is_real or local else "mock-only"
        if state == "mock-only":
            still_mock.append((name, ", ".join(sorted(uses.get(name, [])))))
        rows.append((name, state, "; ".join(method + " " + path for method, path in found) or ("local utility (no HTTP)" if local else "unresolved" if is_real else "—"),
                     ", ".join(sorted(uses.get(name, [])))))
    lines = map_header("UI pages, API layer and shared helpers", "Page usage is scanned from actual api.* references, not the hand-maintained PAGE_USES map. Hashes preserve exact ROUTES regexes from web/js/main.js; CSS includes global web/index.html styles and explicit page stylesheet references. HTTP paths normalize dynamic arguments to braces and omit queries. Local utilities are distinguished from the mock backlog.")
    lines += ["## Still mock", ""] + (table(["API function", "Pages"], still_mock) if still_mock else ["None."])
    lines += ["", "## Pages", ""] + table(["Page", "Route hash", "Exported view", "api.* references", "CSS"], pages)
    lines += ["", "## API layer", "", "Exports from `web/js/api.js`, including auth object methods. REAL means an HTTP-mode implementation exists (REAL set, explicit mode gate, or a mode-independent local utility). Local utilities need no endpoint. Endpoint calls include statically named local wrappers; unresolved paths are reported explicitly.", ""]
    lines += table(["Exported function", "Mode", "Method / path", "Pages"], rows)
    ui = (root / "web/js/ui.js").read_text(encoding="utf-8")
    ui_code, ui_fns = js_functions(ui, exported=True)
    summaries = []
    declarations = [(fn.name, fn.start, "function") for fn in ui_fns]
    declarations += [(m[1], m.start(), "utility") for m in re.finditer(r"\bexport\s+const\s+(\w+)", ui_code)]
    for name, at, kind in sorted(declarations):
        before = ui[:at].rstrip()
        comments = re.search(r"((?:^[ \t]*//[^\n]*\n?)+)\Z", before, re.M)
        summary = comments[1].splitlines()[0].lstrip(" /-") if comments else ""
        if not summary or summary.endswith("----------"):
            # Provide a useful declaration-derived description without copying HTML or values.
            words = re.sub(r"([a-z])([A-Z])", r"\1 \2", name).lower()
            summary = UI_SUMMARIES.get(name, "Shared " + words + " " + kind + ".")
        summaries.append((name, safe_summary(summary)))
    lines += ["", "## Shared UI helpers", "", "Exports from `web/js/ui.js`; summaries use adjacent comments, falling back to the declaration name.", ""]
    lines += table(["Name", "One-line summary"], summaries)
    return "\n".join(lines) + "\n"


def render(root):
    crates = collect(root)
    index = root / "docs/INDEX.md"
    old = index.read_text(encoding="utf-8") if index.exists() else HEADER + START + "\n" + END + "\n"
    if old.count(START) != 1 or old.count(END) != 1 or old.index(START) >= old.index(END):
        raise ValueError("docs/INDEX.md must contain exactly one ordered pair of generation markers")
    before, rest = old.split(START)
    _, after = rest.split(END)
    outputs = {index: before + START + "\n" + index_block(root, crates) + END + after}
    for crate in crates:
        outputs[root / "docs/index/symbols" / (crate.name + ".md")] = symbol_map(crate)
    variants = privop_data(root)
    routes, registry, by_name = route_data(root)
    store_sources, store_methods = store_data(root)
    for name, content in (
        ("routes", routes_map(root, routes, registry, by_name, variants, store_methods.keys())),
        ("privops", privops_map(root, variants)),
        ("store", store_map(root, store_sources, store_methods)),
        ("ui", ui_map(root)),
    ):
        outputs[root / "docs/index" / (name + ".md")] = content
    # Removed/renamed crates must not leave stale symbol files behind.
    obsolete = sorted(set((root / "docs/index/symbols").glob("*.md")) - outputs.keys())
    if len(outputs[index].encode("utf-8")) >= 15_000 or len(outputs[index].splitlines()) >= 250:
        raise ValueError("docs/INDEX.md exceeds its 15 KB / 250 line budget")
    other_size = sum(path.stat().st_size for path in (root / "docs/index").rglob("*")
                     if path.is_file() and path not in outputs and path not in obsolete)
    if other_size + sum(len(value.encode("utf-8")) for value in outputs.values()) >= 500_000:
        raise ValueError("docs/index/ plus INDEX.md exceeds its 500 KB budget")
    return outputs, obsolete


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="print stale-file diffs and exit 1 without writing")
    args = parser.parse_args()
    try:
        outputs, obsolete = render(ROOT)
        stale = False
        for path in sorted(set(outputs) | set(obsolete)):
            old = path.read_text(encoding="utf-8") if path.exists() else ""
            new = outputs.get(path, "")
            if old == new and path not in obsolete:
                continue
            stale = True
            if args.check:
                relative = path.relative_to(ROOT).as_posix()
                sys.stdout.writelines(difflib.unified_diff(old.splitlines(True), new.splitlines(True),
                                                         fromfile="a/" + relative, tofile="b/" + relative))
            elif path in obsolete:
                path.unlink()
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(new, encoding="utf-8")
        return int(args.check and stale)
    except (OSError, ValueError, sqlite3.Error) as error:
        print("gen-index: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
