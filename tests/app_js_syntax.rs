// Static-syntax check for `static/js/app.js` and contract test
// for the Alpine/HTMX integration points that templates rely on.
//
// Why: on 2026-06-05 a regression slipped through code review — an
// extra `});` (close `DOMContentLoaded`) was inserted into
// `app.js` while leftover fragments from the previous code stayed
// at file scope. The orphan `});` caused a JS SyntaxError, the
// browser refused to execute the entire `app.js`, and as a result
// the global `openMediaDrawer` function was never defined. Clicking
// a search-result poster then silently did nothing, because
// `search.html` calls `openMediaDrawer(...)` from an Alpine
// `@click.prevent`.
//
// This test catches that whole class of regressions at `cargo test`
// time, with no JS toolchain required.
//
// It is NOT a full JS parser (it ignores `?` ternaries, regex
// literals, etc.) — but it does walk comments, single/double-
// quoted strings, and template literals (including `${...}`
// expressions), so brace/paren/bracket balance is accurate enough
// to catch the kind of structural damage that broke the build.

use std::fs;
use std::path::{Path, PathBuf};

const APP_JS: &str = "static/js/app.js";
const SEARCH_HTML: &str = "templates/search.html";
const DRAWER_CONTENT_HTML: &str = "templates/media_drawer_content.html";

/// Walk the source tracking brace/paren/bracket depth and string/
/// comment state. Returns `(open_braces, close_braces,
/// open_parens, close_parens, open_brackets, close_brackets)`.
/// We don't fail on imbalance here — we want the test to print
/// the *whole* diff, not bail at the first mismatch.
fn balance(source: &str) -> (usize, usize, usize, usize, usize, usize) {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut ob = 0;
    let mut cb = 0;
    let mut op = 0;
    let mut cp = 0;
    let mut obr = 0;
    let mut cbr = 0;
    // Stack of (kind) so we know what each `}` should close when
    // we hit a template-literal `${...}` expression. Kinds: '{',
    // '(', '['.
    let mut stack: Vec<char> = Vec::new();

    while i < bytes.len() {
        let c = bytes[i] as char;

        // Line comment to end of line.
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if c == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // Single-quoted string.
        if c == '\'' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'\'' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Double-quoted string.
        if c == '"' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Template literal. Supports nested `${...}` and inner
        // template literals. Bare template literals (no ${}) are
        // skipped until the closing backtick.
        if c == '`' {
            i += 1;
            'tmpl: while i < bytes.len() {
                match bytes[i] as char {
                    '`' => {
                        i += 1;
                        break 'tmpl;
                    }
                    '$' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
                        // Enter a JS expression — push '{' so the
                        // outer `}` balances correctly.
                        stack.push('{');
                        ob += 1;
                        i += 2;
                        // Walk until matching `}`, honoring strings
                        // and nested templates.
                        let mut depth = 1usize;
                        while i < bytes.len() && depth > 0 {
                            let cc = bytes[i] as char;
                            if cc == '{' {
                                stack.push('{');
                                ob += 1;
                                depth += 1;
                            } else if cc == '}' {
                                cb += 1;
                                stack.pop();
                                depth -= 1;
                            } else if cc == '(' {
                                op += 1;
                                stack.push('(');
                            } else if cc == ')' {
                                cp += 1;
                                stack.pop();
                            } else if cc == '[' {
                                obr += 1;
                                stack.push('[');
                            } else if cc == ']' {
                                cbr += 1;
                                stack.pop();
                            } else if cc == '\'' {
                                i += 1;
                                while i < bytes.len() {
                                    if bytes[i] == b'\\' {
                                        i += 2;
                                        continue;
                                    }
                                    if bytes[i] == b'\'' {
                                        i += 1;
                                        break;
                                    }
                                    i += 1;
                                }
                                continue;
                            } else if cc == '"' {
                                i += 1;
                                while i < bytes.len() {
                                    if bytes[i] == b'\\' {
                                        i += 2;
                                        continue;
                                    }
                                    if bytes[i] == b'"' {
                                        i += 1;
                                        break;
                                    }
                                    i += 1;
                                }
                                continue;
                            } else if cc == '`' {
                                // Nested template — recurse: from
                                // here, walk as a fresh template.
                                i += 1;
                                'nested: while i < bytes.len() {
                                    match bytes[i] as char {
                                        '`' => {
                                            i += 1;
                                            break 'nested;
                                        }
                                        '$' if i + 1 < bytes.len()
                                            && bytes[i + 1] == b'{' =>
                                        {
                                            depth += 1;
                                            i += 2;
                                        }
                                        _ => i += 1,
                                    }
                                }
                                continue;
                            }
                            i += 1;
                        }
                    }
                    '\\' => {
                        i += 2;
                    }
                    _ => i += 1,
                }
            }
            continue;
        }
        // Regular punctuation.
        match c {
            '{' => {
                ob += 1;
                stack.push('{');
            }
            '}' => {
                cb += 1;
                stack.pop();
            }
            '(' => {
                op += 1;
                stack.push('(');
            }
            ')' => {
                cp += 1;
                stack.pop();
            }
            '[' => {
                obr += 1;
                stack.push('[');
            }
            ']' => {
                cbr += 1;
                stack.pop();
            }
            _ => {}
        }
        i += 1;
    }
    (ob, cb, op, cp, obr, cbr)
}

fn read_root(path: &str) -> String {
    let p = repo_path(path);
    fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!("read {}: {e}", p.display());
    })
}

fn repo_path(rel: &str) -> PathBuf {
    // CARGO_MANIFEST_DIR is set by `cargo test` to the crate root.
    let base = std::env::var("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|_| ".".to_string());
    Path::new(&base).join(rel)
}

/// Regression: `app.js` must parse (brace-wise) without orphan
/// brackets. The original bug was an extra `});` at file scope
/// after `DOMContentLoaded` had already been closed.
#[test]
fn app_js_is_structurally_balanced() {
    let src = read_root(APP_JS);
    let (ob, cb, op, cp, obr, cbr) = balance(&src);
    assert_eq!(
        ob, cb,
        "app.js: brace imbalance: {ob} '{{' vs {cb} '}}'\n\
         (this kind of mismatch is what broke `openMediaDrawer` on 2026-06-05 \
          — usually an extra `});` left over from a botched edit)"
    );
    assert_eq!(op, cp, "app.js: paren imbalance: {op} '(' vs {cp} ')'");
    assert_eq!(
        obr, cbr,
        "app.js: bracket imbalance: {obr} '[' vs {cbr} ']'"
    );
}

/// `app.js` must define the global `openMediaDrawer` function that
/// `search.html` and other templates call from Alpine `@click`.
#[test]
fn app_js_defines_open_media_drawer() {
    let src = read_root(APP_JS);
    let defines_function = src.contains("function openMediaDrawer(")
        || src.contains("function openMediaDrawer (")
        || src.contains("openMediaDrawer = function")
        || src.contains("openMediaDrawer = (");
    assert!(
        defines_function,
        "app.js: expected to find `openMediaDrawer` as a function. \
         templates/search.html calls it from `@click.prevent=\"openMediaDrawer(...)\"`; \
         if you renamed it, update the template too."
    );
}

/// Between the `DOMContentLoaded` callback open and the first
/// top-level helper (e.g. `// --- Theme ---`) there must be
/// exactly one `});` at column 0 — the close of the
/// `DOMContentLoaded` callback itself. More than one means a
/// stale fragment from a previous edit is hanging at file scope;
/// that's what broke the build on 2026-06-05.
#[test]
fn app_js_closes_dom_content_loaded_exactly_once() {
    let src = read_root(APP_JS);

    let dom_open_line = src
        .lines()
        .position(|l| l.contains("addEventListener('DOMContentLoaded'"))
        .unwrap_or_else(|| {
            panic!("app.js: no `addEventListener('DOMContentLoaded'` found");
        });
    // First top-level helper marker (e.g. `// --- Theme ---`).
    // Anything between DOMContentLoaded and that marker is still
    // inside the callback.
    let end_line = src
        .lines()
        .enumerate()
        .skip(dom_open_line)
        .find(|(_, l)| l.trim_start().starts_with("// ---"))
        .map(|(i, _)| i)
        .unwrap_or(src.lines().count());

    let closes_inside: Vec<usize> = src
        .lines()
        .enumerate()
        .skip(dom_open_line)
        .take(end_line - dom_open_line)
        // Strict column-0 match. `l.trim() == "});"` would also
        // catch `});` lines that are inside callbacks (just
        // indented); we only care about the top-level close of
        // the DOMContentLoaded callback itself.
        .filter(|(_, l)| *l == "});")
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(
        closes_inside.len(),
        1,
        "app.js: expected exactly 1 `});` at column 0 between `DOMContentLoaded` \
         (line {}) and the next top-level helper (line {}); found {}: {:?}. \
         Extra `});` at column 0 means a fragment of the previous code stayed \
         behind when an edit replaced the closing braces — delete the orphan lines.",
        dom_open_line + 1,
        end_line,
        closes_inside.len(),
        closes_inside,
    );
}

/// `search.html` must reference `openMediaDrawer` so the click on
/// the media card actually opens the drawer.
#[test]
fn search_html_calls_open_media_drawer() {
    let src = read_root(SEARCH_HTML);
    assert!(
        src.contains("openMediaDrawer("),
        "{SEARCH_HTML}: no call to `openMediaDrawer(` found. \
         The media card `@click.prevent` must call this function or \
         clicking posters will silently do nothing."
    );
}

/// `media_drawer_content.html` must contain a clickable delete
/// action so users can remove items from their tracking list.
#[test]
fn drawer_content_has_delete_action() {
    let src = read_root(DRAWER_CONTENT_HTML);
    assert!(
        src.contains("drawer-action-btn")
            && (src.contains("delete") || src.contains("Удалить")),
        "{DRAWER_CONTENT_HTML}: drawer must expose a delete action button. \
         `drawer-action-btn` with class `delete` is what app.js's afterDelete fallback listens for."
    );
}
