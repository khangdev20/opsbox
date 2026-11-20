#!/usr/bin/env python3
"""Generate the agent crate/symbol maps using only the Python 3 standard library.

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


def mask_rust(source):
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
        parts.append(blank(source[start:end]))
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


def production_code(source):
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
    lines = ["## Layout", "", "| Folder | Contents |", "|---|---|"]
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
    except (OSError, ValueError) as error:
        print("gen-index: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
