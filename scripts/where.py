#!/usr/bin/env python3
"""Locate Rust/JS definitions, then usages: python3 scripts/where.py <symbol>.

Literal substrings and Type::method are accepted. Qualified definitions are
filtered by impl/trait owner (or module filename); instance usages are lexical
name candidates, not type resolution. Print at most 39 definitions and 20 usage
lines. Search crates/ and web/, excluding dependencies/build output and symlinks.
"""

import argparse
import importlib.util
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("gen_index", Path(__file__).with_name("gen-index.py"))
GEN = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = GEN
SPEC.loader.exec_module(GEN)
SKIP = {"target", "node_modules", "vendor", "generated"}
RUST = re.compile(
    r"\b(?:pub(?:\s*\([^)]*\))?\s+)?"
    r"(?:(?:async|unsafe|extern|const)\s+)*"
    r"(?P<kind>fn|struct|enum|trait|const)\s+(?P<name>(?:r#)?[A-Za-z_]\w*)"
)
JS = re.compile(
    r"\b(?P<kind>export)\s+(?:default\s+)?(?:async\s+)?"
    r"(?:function\s*\*?\s*|class\s+|const\s+|let\s+|var\s+)?(?P<name>[A-Za-z_$][\w$]*)"
    r"|\b(?P<function>function)\s*\*?\s*(?P<fn_name>[A-Za-z_$][\w$]*)"
)


def candidates(root, name):
    """Use rg to read only matching files; fallback uses the same ignore rules."""
    directories = [part for part in ("crates", "web") if (root / part).is_dir()]
    if not directories:
        return []
    rg = shutil.which("rg")
    if rg:
        command = [rg, "--files-with-matches", "--null", "--fixed-strings",
                   "--hidden", "--no-require-git", "--glob", "!.git/**",
                   "--type-add", "lookup:*.rs", "--type-add", "lookup:*.js", "--type", "lookup"]
        for directory in sorted(SKIP):
            command += ["--glob", "!**/" + directory + "/**"]
        result = subprocess.run(command + ["--", name, *directories], cwd=root,
                                capture_output=True, check=False)
        if result.returncode not in (0, 1):
            raise OSError(result.stderr.decode("utf-8", errors="replace").strip())
        return sorted(root / os.fsdecode(path) for path in result.stdout.split(b"\0") if path)
    ignores = GEN.Ignores(root)
    ignores.load(root)
    pattern = re.compile(re.escape(name))
    found = []

    def walk(directory):
        ignores.load(directory)
        for path in sorted(directory.iterdir()):
            if path.is_symlink() or path.name.startswith(".git") or path.name in SKIP:
                continue
            is_dir = path.is_dir()
            if ignores.ignored(path, is_dir):
                continue
            if is_dir:
                walk(path)
            elif path.suffix in {".rs", ".js"} and pattern.search(path.read_text(encoding="utf-8")):
                found.append(path)

    for directory in directories:
        walk(root / directory)
    return sorted(found)


def owners(code):
    """Return lexical impl/trait scopes, including generic and trait impls."""
    scopes = []
    for match in re.finditer(r"\b(?:impl|trait)\b[^{};]*\{", code):
        start = match.end() - 1
        scopes.append((start, GEN.balanced_end(code, start, "{", "}"), match.group()))
    return scopes


def signature(source, code, match):
    start, end = match.start(), match.end()
    # Skip parameter defaults/destructuring braces before finding the body.
    if match.groupdict().get("kind") == "fn" or "function" in match.group():
        opening = code.find("(", end)
        if opening >= 0:
            end = GEN.balanced_end(code, opening, "(", ")")
    stop = re.search(r"[{};=]", code[end:])
    finish = end + stop.start() if stop else end
    return " ".join(source[start:finish].split())[:240]


def lookup(root, query):
    owner, separator, name = query.rpartition("::")
    if not separator:
        name = query
    definitions, usages = [], []
    pattern = re.compile(re.escape(name))
    for path in candidates(root, name):
        source = path.read_text(encoding="utf-8")
        code = GEN.mask_rust(source)[0] if path.suffix == ".rs" else GEN.mask_js(source)
        scopes = owners(code) if owner and path.suffix == ".rs" else []
        declaration_lines = set()
        for match in (RUST if path.suffix == ".rs" else JS).finditer(code):
            declared = match.groupdict().get("name") or match.group("fn_name")
            if name not in declared:
                continue
            line = code.count("\n", 0, match.start()) + 1
            end_line = code.count("\n", 0, match.end()) + 1
            declaration_lines.update(range(line, end_line + 1))
            if owner and owner != path.stem and not any(
                lo < match.start() < hi and re.search(r"\b" + re.escape(owner) + r"\b", header)
                for lo, hi, header in scopes
            ):
                continue
            kind = match.groupdict().get("kind") or "function"
            definitions.append((path.relative_to(root).as_posix(), line, kind, signature(source, code, match)))
        for line, (masked, original) in enumerate(zip(code.splitlines(), source.splitlines()), 1):
            if line not in declaration_lines and pattern.search(masked):
                usages.append((path.relative_to(root).as_posix(), line, "use", " ".join(original.split())[:240]))
    return definitions[:39] + usages[:20]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("symbol", help="literal substring or Type::method")
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z_$][\w$]*(?:::[A-Za-z_$][\w$]*)?", args.symbol):
        parser.error("use a symbol substring or Type::method")
    try:
        rows = lookup(ROOT, args.symbol)
        for path, line, kind, text in rows:
            print(f"{path}:{line}  {kind}  {text}")
        return 0 if rows else 1
    except (OSError, ValueError) as error:
        print("where: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
