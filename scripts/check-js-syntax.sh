#!/usr/bin/env bash
# Check that static/js/app.js is structurally well-formed and
# matches the contract that templates rely on.
#
# Why this exists: the project has a Rust test
# (tests/app_js_syntax.rs) that does the same thing, but cargo
# is not installed on the production server. This script is a
# shell-out that anyone on the server can run without a Rust
# toolchain.
#
# Usage:
#   ./scripts/check-js-syntax.sh
#
# Exit code 0 = OK, 1 = problems found.
#
# The brace/paren/bracket balance check is a 100-line state
# machine in Python — it walks comments, single/double-quoted
# strings, and template literals (with ${...} expressions) but
# does not parse regex literals. It catches the specific failure
# mode that broke `openMediaDrawer` on 2026-06-05 (an extra
# `});` at file scope).

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_JS="$REPO_ROOT/static/js/app.js"
SEARCH_HTML="$REPO_ROOT/templates/search.html"
DRAWER_HTML="$REPO_ROOT/templates/media_drawer_content.html"

if [[ ! -f "$APP_JS" ]]; then
    echo "ERROR: $APP_JS not found" >&2
    exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
    echo "ERROR: python3 required" >&2
    exit 1
fi

python3 - "$APP_JS" "$SEARCH_HTML" "$DRAWER_HTML" <<'PY'
import sys
import re

app_js_path, search_path, drawer_path = sys.argv[1], sys.argv[2], sys.argv[3]

with open(app_js_path, "r", encoding="utf-8") as f:
    src = f.read()

# --- Brace / paren / bracket balance walker ---
# Same logic as the Rust test, in Python.
def balance(source):
    ob = cb = 0
    op = cp = 0
    obr = cbr = 0
    stack = []
    i = 0
    n = len(source)
    while i < n:
        c = source[i]
        # Line comment
        if c == '/' and i + 1 < n and source[i + 1] == '/':
            i += 2
            while i < n and source[i] != '\n':
                i += 1
            continue
        # Block comment
        if c == '/' and i + 1 < n and source[i + 1] == '*':
            i += 2
            while i + 1 < n and not (source[i] == '*' and source[i + 1] == '/'):
                i += 1
            i = min(i + 2, n)
            continue
        # Single-quoted string
        if c == "'":
            i += 1
            while i < n:
                if source[i] == '\\':
                    i += 2
                    continue
                if source[i] == "'":
                    i += 1
                    break
                i += 1
            continue
        # Double-quoted string
        if c == '"':
            i += 1
            while i < n:
                if source[i] == '\\':
                    i += 2
                    continue
                if source[i] == '"':
                    i += 1
                    break
                i += 1
            continue
        # Template literal
        if c == '`':
            i += 1
            while i < n:
                if source[i] == '`':
                    i += 1
                    break
                if source[i] == '$' and i + 1 < n and source[i + 1] == '{':
                    stack.append('{')
                    ob += 1
                    i += 2
                    depth = 1
                    while i < n and depth > 0:
                        cc = source[i]
                        if cc == '{':
                            stack.append('{')
                            ob += 1
                            depth += 1
                        elif cc == '}':
                            cb += 1
                            if stack:
                                stack.pop()
                            depth -= 1
                        elif cc == '(':
                            op += 1
                            stack.append('(')
                        elif cc == ')':
                            cp += 1
                            if stack:
                                stack.pop()
                        elif cc == '[':
                            obr += 1
                            stack.append('[')
                        elif cc == ']':
                            cbr += 1
                            if stack:
                                stack.pop()
                        elif cc == "'":
                            i += 1
                            while i < n:
                                if source[i] == '\\':
                                    i += 2
                                    continue
                                if source[i] == "'":
                                    i += 1
                                    break
                                i += 1
                            continue
                        elif cc == '"':
                            i += 1
                            while i < n:
                                if source[i] == '\\':
                                    i += 2
                                    continue
                                if source[i] == '"':
                                    i += 1
                                    break
                                i += 1
                            continue
                        elif cc == '`':
                            i += 1
                            while i < n:
                                if source[i] == '`':
                                    i += 1
                                    break
                                if source[i] == '$' and i + 1 < n and source[i + 1] == '{':
                                    depth += 1
                                    i += 2
                                    continue
                                i += 1
                            continue
                        i += 1
                    continue
                if source[i] == '\\':
                    i += 2
                    continue
                i += 1
            continue
        if c == '{':
            ob += 1
            stack.append('{')
        elif c == '}':
            cb += 1
            if stack:
                stack.pop()
        elif c == '(':
            op += 1
            stack.append('(')
        elif c == ')':
            cp += 1
            if stack:
                stack.pop()
        elif c == '[':
            obr += 1
            stack.append('[')
        elif c == ']':
            cbr += 1
            if stack:
                stack.pop()
        i += 1
    return ob, cb, op, cp, obr, cbr, stack

ob, cb, op, cp, obr, cbr, stack = balance(src)

errors = []
if ob != cb:
    errors.append(f"brace imbalance: {ob} '{{' vs {cb} '}}'")
if op != cp:
    errors.append(f"paren imbalance: {op} '(' vs {cp} ')'")
if obr != cbr:
    errors.append(f"bracket imbalance: {obr} '[' vs {cbr} ']'")
if stack:
    errors.append(f"unclosed: {stack[:5]}")

# --- Single top-level `});` between DOMContentLoaded and the next helper marker ---
lines = src.split('\n')
try:
    dom_line = next(i for i, l in enumerate(lines)
                    if "addEventListener('DOMContentLoaded'" in l)
except StopIteration:
    errors.append("no `addEventListener('DOMContentLoaded'` found in app.js")
    dom_line = None

if dom_line is not None:
    end_line = next(
        (i for i, l in enumerate(lines[dom_line + 1:], dom_line + 1)
         if l.lstrip().startswith("// ---")),
        len(lines),
    )
    closes = [i + 1 for i, l in enumerate(lines[dom_line:end_line], dom_line)
              if l == "});"]
    if len(closes) != 1:
        errors.append(
            "expected exactly 1 top-level `}});` between DOMContentLoaded "
            "(line {}) and next top-level helper (line {}); found {} at lines {}".format(
                dom_line + 1, end_line + 1, len(closes), closes
            )
        )

# --- `openMediaDrawer` is defined ---
if not re.search(r"function\s+openMediaDrawer\s*\(", src):
    errors.append("`function openMediaDrawer(` not found in app.js")

# --- Contract: search.html references it ---
with open(search_path, "r", encoding="utf-8") as f:
    search_src = f.read()
if "openMediaDrawer(" not in search_src:
    errors.append(f"{search_path}: no `openMediaDrawer(` reference")

# --- Contract: drawer has delete action ---
with open(drawer_path, "r", encoding="utf-8") as f:
    drawer_src = f.read()
if "drawer-action-btn" not in drawer_src or "delete" not in drawer_src:
    errors.append(
        f"{drawer_path}: drawer must contain a delete action button "
        f"(.drawer-action-btn with `delete` class)"
    )

if errors:
    print("FAILED:")
    for e in errors:
        print("  - " + e)
    sys.exit(1)

print("OK: app.js is structurally balanced, "
      "DOMContentLoaded closed exactly once, "
      "openMediaDrawer defined and called from search.html, "
      "drawer has delete action.")
PY
