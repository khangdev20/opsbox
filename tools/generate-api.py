#!/usr/bin/env python3
"""Keep the route inventory in docs/API.md synchronized with the axum router."""
import argparse
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
START = '<!-- GENERATED_ROUTES_START -->'
END = '<!-- GENERATED_ROUTES_END -->'


def inventory():
    source = (ROOT / 'crates/opsbox-api/src/lib.rs').read_text()
    rows = []
    for match in re.finditer(r'\.route\(', source):
        at, depth, quoted, escaped = match.end(), 1, False, False
        end = at
        while depth:
            char = source[end]
            if escaped:
                escaped = False
            elif quoted and char == '\\':
                escaped = True
            elif char == '"':
                quoted = not quoted
            elif not quoted:
                depth += (char == '(') - (char == ')')
            end += 1
        route = re.fullmatch(r'\s*"([^"]+)"\s*,\s*(.*?)\s*,?\s*', source[at:end-1], re.S)
        if not route:
            raise ValueError('Cannot parse route: ' + source[at:end-1])
        path, handlers = route.groups()
        methods = re.findall(r'\b(get|post|put|delete)\((\w+::\w+)\)', handlers)
        if not methods:
            raise ValueError('No handler for ' + path)
        for method, handler in methods:
            module = handler.split('::')[0]
            rows.append((path, method.upper(), handler, module))
    return '\n'.join([
        START,
        '| Method | Path | Handler / request types |',
        '|---|---|---|',
        *[f'| `{method}` | `{path}` | [{handler}](../crates/opsbox-api/src/{module}.rs) |' for path, method, handler, module in sorted(rows)],
        END,
    ])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    path = ROOT / 'docs/API.md'
    old = path.read_text()
    generated = inventory()
    new = re.sub(re.escape(START) + r'.*?' + re.escape(END), lambda _: generated, old, flags=re.S)
    if new == old and START not in old:
        raise SystemExit('Missing generated-route markers in docs/API.md')
    if args.check and new != old:
        raise SystemExit('API routes changed. Run python3 tools/generate-api.py')
    if not args.check:
        path.write_text(new)


if __name__ == '__main__':
    main()
