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
const PROGRESS_ROW_HTML: &str = "templates/partials/_progress_row.html";
const DETAIL_HTML: &str = "templates/media_detail.html";
const CHAPTER_LIST_HTML: &str = "templates/partials/_chapter_list.html";
const EPISODE_LIST_HTML: &str = "templates/partials/_episode_list.html";

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
                                        '$' if i + 1 < bytes.len() && bytes[i + 1] == b'{' => {
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
    let base = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
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
          — usually an extra `}});` left over from a botched edit)"
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
        "app.js: expected exactly 1 `}});` at column 0 between `DOMContentLoaded` \
         (line {}) and the next top-level helper (line {}); found {}: {:?}. \
         Extra `}});` at column 0 means a fragment of the previous code stayed \
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
        src.contains("drawer-action-btn") && (src.contains("delete") || src.contains("Удалить")),
        "{DRAWER_CONTENT_HTML}: drawer must expose a delete action button. \
         `drawer-action-btn` with class `delete` is what app.js's afterDelete fallback listens for."
    );
}

/// The drawer must render the progress row through `{% include %}` of the
/// shared partial. If it inlines its own copy again, that copy and the row
/// returned by the HTMX endpoints can drift — which is what produced the
/// asymmetric `−`/`+1` bug.
#[test]
fn drawer_content_includes_shared_progress_row() {
    let src = read_root(DRAWER_CONTENT_HTML);
    assert!(
        src.contains("{% include \"partials/_progress_row.html\" %}"),
        "{DRAWER_CONTENT_HTML}: expected `{{% include \"partials/_progress_row.html\" %}}`. \
         The row is the single source of truth shared with GET /tracking/{{id}}/progress-row."
    );
}

/// The shared partial must carry the tracking id (so the client can re-fetch
/// the row) and swap the *whole row* (so `+1`/`−` can be added back, not just
/// removed).
#[test]
fn progress_row_partial_swaps_whole_row() {
    let src = read_root(PROGRESS_ROW_HTML);
    assert!(
        src.contains("data-tracking-id="),
        "{PROGRESS_ROW_HTML}: missing `data-tracking-id` — app.js's `progressUpdated` \
         handler reads it to call GET /tracking/<id>/progress-row."
    );
    assert!(
        src.contains("hx-target=\"closest .drawer-progress-row\""),
        "{PROGRESS_ROW_HTML}: buttons must target `closest .drawer-progress-row`."
    );
    assert!(
        src.contains("hx-swap=\"outerHTML\""),
        "{PROGRESS_ROW_HTML}: buttons must swap the row with `outerHTML` so the \
         server-rendered buttons replace the old ones both ways."
    );
}

/// After an episode/chapter toggle `app.js` must ask the server for the
/// authoritative row instead of hand-patching buttons. The old one-way
/// `.remove()` of the `+1` button is the regression we are guarding against.
#[test]
fn app_js_refetches_progress_row() {
    let src = read_root(APP_JS);
    assert!(
        src.contains("/progress-row"),
        "app.js: expected `htmx.ajax('GET', '/tracking/<id>/progress-row', ...)` \
         after `progressUpdated`. Without it the +1/− buttons never come back \
         after a checkbox toggle."
    );
    assert!(
        !src.contains("plusBtn"),
        "app.js: `plusBtn` hand-patching was replaced by a server re-render. \
         If it is back, the one-way button-removal bug is back too."
    );
}

/// Progress clicks target `closest .drawer-progress-row`, so `e.detail.target`
/// is the row and only `e.detail.elt` still identifies the button. The global
/// listener must therefore read `elt`.
#[test]
fn app_js_detects_progress_click_via_elt() {
    let src = read_root(APP_JS);
    assert!(
        src.contains("e.detail.elt || e.detail.target"),
        "app.js: progress-button detection must use `e.detail.elt` (with a \
         `e.detail.target` fallback); `e.detail.target` is the freshly-swapped \
         row, not the button that fired the request."
    );
}

/// Redesigned drawer must expose the derived status badge (not the raw
/// provider string), collapsed alt-titles, the info list and the MU link.
#[test]
fn drawer_content_redesign_markers() {
    let src = read_root(DRAWER_CONTENT_HTML);
    assert!(
        src.contains("drawer-alt-titles"),
        "drawer: alt-titles block missing"
    );
    assert!(
        src.contains("status_label"),
        "drawer: derived status_label not rendered"
    );
    assert!(
        src.contains("drawer-progress-sticky"),
        "drawer: sticky progress block missing"
    );
    assert!(
        src.contains("drawer-info-list"),
        "drawer: info list missing"
    );
    assert!(
        !src.contains("{{ item.status.as_ref().unwrap() }}</span>"),
        "drawer: raw provider status must not be rendered as a badge"
    );
}

/// Chapter list must be server-windowed and expose the HTMX "Все главы" toggle.
#[test]
fn chapter_list_is_windowed_with_htmx_toggle() {
    let src = read_root(CHAPTER_LIST_HTML);
    assert!(
        src.contains("chapter-list-wrapper"),
        "chapter list: wrapper for outerHTML swap missing"
    );
    assert!(
        src.contains("?all=true"),
        "chapter list: full-list HTMX toggle missing"
    );
    assert!(
        src.contains("closest .chapter-list-wrapper"),
        "chapter list: toggle must target the wrapper"
    );
}

/// Detail page must render the Structure sections and the MU link.
#[test]
fn detail_page_redesign_markers() {
    let src = read_root(DETAIL_HTML);
    assert!(
        src.contains("Переводы"),
        "detail: translations section missing"
    );
    assert!(
        src.contains("detail-info-list"),
        "detail: info list missing"
    );
    assert!(
        src.contains("Открыть на MangaUpdates"),
        "detail: MU link missing"
    );
    assert!(
        !src.contains("{{ item.status.as_ref().unwrap() }}</span>"),
        "detail: raw provider status must not be rendered as a badge"
    );
}

/// Redesign follow-up: content lists must sit *below* the description section.
/// This assertion is red before the layout change and must be green after it.
#[test]
fn drawer_content_lists_below_description() {
    let src = read_root(DRAWER_CONTENT_HTML);

    let description_pos = src.find("drawer-description-details").unwrap_or_else(|| {
        panic!(
            "{DRAWER_CONTENT_HTML}: description section (`drawer-description-details`) not found"
        )
    });
    let chapters_pos = src.find("/chapters\"").unwrap_or_else(|| {
        panic!("{DRAWER_CONTENT_HTML}: chapters content-list endpoint (`/chapters\"`) not found")
    });
    assert!(
        chapters_pos > description_pos,
        "{DRAWER_CONTENT_HTML}: content lists must appear AFTER the description \
         section (description at byte {description_pos}, chapters at byte {chapters_pos}). \
         Metadata and description are primary; episode/season/chapter/addition lists are secondary."
    );
}

/// Redesign follow-up: the shared progress row must move into the hero, i.e.
/// its `{% include %}` must appear BEFORE the sticky status block. The sticky
/// wrapper itself (`drawer-progress-sticky`) stays and still renders the status
/// chips; only the progress row leaves it.
#[test]
fn drawer_content_progress_row_in_hero() {
    let src = read_root(DRAWER_CONTENT_HTML);

    let include_pos = src
        .find("{% include \"partials/_progress_row.html\" %}")
        .unwrap_or_else(|| {
            panic!("{DRAWER_CONTENT_HTML}: `partials/_progress_row.html` include not found")
        });
    let sticky_pos = src.find("drawer-progress-sticky").unwrap_or_else(|| {
        panic!("{DRAWER_CONTENT_HTML}: sticky status block (`drawer-progress-sticky`) not found")
    });
    assert!(
        include_pos < sticky_pos,
        "{DRAWER_CONTENT_HTML}: the progress-row include must appear BEFORE the \
         sticky status block — i.e. inside `.drawer-hero-info` (include at byte \
         {include_pos}, sticky at byte {sticky_pos})."
    );
}

/// The progress include is gated on `has_progress`, which is derived from the
/// media type only (`supports_progress`) — an *untracked* anime/manga also has
/// `has_progress == true`. The include must therefore sit behind a
/// `tracking_id` guard too, or its buttons POST to `Uuid::nil`.
#[test]
fn drawer_content_progress_row_requires_tracking_id() {
    let src = read_root(DRAWER_CONTENT_HTML);

    let include_pos = src
        .find("{% include \"partials/_progress_row.html\" %}")
        .unwrap_or_else(|| {
            panic!("{DRAWER_CONTENT_HTML}: `partials/_progress_row.html` include not found")
        });
    // Nearest preceding `{% if let Some( ... ) = tracking_id %}` guard.
    let guard_pos = src[..include_pos]
        .rfind("= tracking_id %}")
        .unwrap_or_else(|| {
            panic!(
                "{DRAWER_CONTENT_HTML}: no `tracking_id` guard before the progress include; \
                 untracked items would render the row with Uuid::nil and POST to a nil id"
            )
        });
    assert!(
        guard_pos < include_pos,
        "{DRAWER_CONTENT_HTML}: progress include must be wrapped in a `tracking_id` guard"
    );
}

/// Episode list must be server-windowed and expose the HTMX "Все эпизоды"
/// toggle, mirroring the chapter list contract.
#[test]
fn episode_list_is_windowed_with_htmx_toggle() {
    let src = read_root(EPISODE_LIST_HTML);
    assert!(
        src.contains("episode-list-wrapper"),
        "{EPISODE_LIST_HTML}: wrapper for outerHTML swap missing"
    );
    assert!(
        src.contains("?all=true"),
        "{EPISODE_LIST_HTML}: full-list HTMX toggle missing"
    );
    assert!(
        src.contains("closest .episode-list-wrapper"),
        "{EPISODE_LIST_HTML}: toggle must target the wrapper"
    );
    assert!(
        src.contains("Все эпизоды ({{ total }})"),
        "{EPISODE_LIST_HTML}: toggle must carry the total count label"
    );
}

/// The episodes section is rendered statically (no Alpine toggle); the season
/// accordion lives in its own partial, not in the shared drawer. Scoped to the
/// episodes section so an unrelated Alpine use elsewhere cannot satisfy it.
#[test]
fn drawer_episodes_section_has_no_alpine_toggle() {
    let src = read_root(DRAWER_CONTENT_HTML);
    let start = src
        .find("{# === 5. EPISODES")
        .expect("episodes section marker missing");
    let end = src
        .find("{# === 6.")
        .expect("seasons section marker missing");
    let section = &src[start..end];
    assert!(
        !section.contains("x-data"),
        "the episodes section must not declare Alpine `x-data`"
    );
    assert!(
        !section.contains("x-show"),
        "the episodes section must not use Alpine `x-show`"
    );
}

/// The INFO divider must render only for untracked items — a tracked item
/// already gets its line from the sticky status block's `border-bottom`.
#[test]
fn drawer_info_divider_only_for_untracked() {
    let src = read_root(DRAWER_CONTENT_HTML);
    let guard = src
        .find("tracking_id.is_none()")
        .expect("no `tracking_id.is_none()` guard in the drawer template");
    let block_end = src[guard..]
        .find("{% endif %}")
        .expect("no `{% endif %}` after the `tracking_id.is_none()` guard")
        + guard;
    let divider = src
        .find("<div class=\"drawer-divider\"></div>")
        .expect("no `drawer-divider` in the drawer template");
    assert!(
        guard < divider && divider < block_end,
        "the INFO divider must be INSIDE the `tracking_id.is_none()` block \
         (guard at byte {guard}, divider at byte {divider}, block ends at byte {block_end})"
    );
}

/// Western comics track chapters, so the CHAPTERS section condition must
/// include `comic`. Scoped to section 7: the hero poster placeholder also
/// mentions `other-comics`/`comic`, so a bare `contains` would be fooled.
#[test]
fn drawer_chapters_include_comic() {
    let src = read_root(DRAWER_CONTENT_HTML);
    let start = src
        .find("{# === 7. CHAPTERS")
        .expect("chapters section marker missing");
    let end = src
        .find("{# === 8.")
        .expect("game additions section marker missing");
    let section = &src[start..end];
    assert!(
        section.contains("item.media_type == \"comic\""),
        "the chapters section condition must include `comic` \
         (Comic Vine items track chapters too)"
    );
}

/// Section labels carry the tracked-unit count where the data exists at
/// render time (`total_count`).
#[test]
fn drawer_labels_carry_counts() {
    let src = read_root(DRAWER_CONTENT_HTML);
    assert!(
        src.contains("Эпизоды{% if let Some(tc) = total_count %} · {{ tc }}{% endif %}"),
        "the episodes label must carry the `total_count` counter"
    );
    assert!(
        src.contains("Главы{% if let Some(tc) = total_count %} · {{ tc }}{% endif %}"),
        "the chapters label must carry the `total_count` counter"
    );
}

/// «Альтернативные названия» renders as a chip with a chevron in both the
/// drawer and the detail page.
#[test]
fn alt_titles_summary_has_chevron_everywhere() {
    let needle = "Альтернативные названия <span class=\"drawer-section-arrow\">▾</span>";
    for path in [DRAWER_CONTENT_HTML, DETAIL_HTML] {
        assert!(
            read_root(path).contains(needle),
            "{path}: alt-titles `<summary>` must render the chip + chevron"
        );
    }
}
