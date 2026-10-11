//! Alpha Polis: schedule/next-date enricher + full TOC + auto-bind guest search.
//!
//! The detail page carries the schedule block
//! (`<span class="date">毎月第4水曜日</span>更新`), the exact next date
//! (`(次回更新日 : 2026.10.28)`) and the complete episode list embedded in the
//! `app-official-*-toc` widget as `<script type="application/json">` with an
//! `episodes[]` array. That list is exactly what the widget would fetch from
//! `/manga/official/episodes.json` (verified live: the SSR count matches the
//! endpoint count for every probed title), so no extra request is made.
//!
//! Episode titles read `第N回` / `第N回（前編）` with full-width digits; they are
//! classified by the shared [`crate::services::external::chapter_title`] JP
//! helpers. The `mainTitle` carries the chapter's own title after the number
//! prefix, which is what gets stored as the row title.
//!
//! Search encodes the media kind in the hit id because the `OfficialSearch`
//! trait passes no `media_type`: `novel/{userId}/{workId}` (composite path)
//! vs `manga/official/{id}`. `official_resolve::hit_matches_media_type`
//! filters by the item's kind; the prefixed id **is** the site path
//! (Decision #9). Manga detail links appear both relative
//! (`/manga/official/{id}` — the real search results) and absolute
//! (`https://www.alphapolis.co.jp/manga/official/{id}` — the pickup carousel);
//! both are collected, and unrelated carousel entries are dropped later by
//! `titles_match`. Novel links must have
//! two numeric path segments, which excludes the `/novel/index` and
//! `/novel/add_tag_counter/{n}` navigation/tag links.

use crate::services::external::chapter_title::{self, EpisodeRow, classify_jp_title, main_count};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Schedule text lives inside `<span class="date">…</span>更新`.
const SCHEDULE_START: &str = "class=\"date\">";
const SCHEDULE_END: &str = "</span>";
/// Exact next date: `(次回更新日 : 2026.10.28)`.
const NEXT_START: &str = "次回更新日 : ";
const NEXT_END: &str = ")";

/// The TOC widget id (`app-official-manga-toc`, `app-official-novel-toc`, …).
const TOC_ID_PREFIX: &str = "id=\"app-official-";
const TOC_ID_SUFFIX: &str = "-toc";
const TOC_SCRIPT: &str = "<script type=\"application/json\">";
const TOC_SCRIPT_END: &str = "</script>";

const NOVEL_MARKER: &str = "href=\"/novel/";

/// GET one Alpha Polis title. Auto-bind ids are already site paths
/// (`novel/{u}/{w}` / `manga/official/{id}`); legacy admin-bound ids are
/// bare manga ids.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    // The id lands in the URL path: only the shapes the search can emit are
    // accepted.
    if !is_alphapolis_id(source_id) {
        return Ok(OfficialMeta {
            source: "alphapolis".to_string(),
            ..OfficialMeta::default()
        });
    }
    let url = if source_id.starts_with("novel/") || source_id.starts_with("manga/official/") {
        format!("https://www.alphapolis.co.jp/{source_id}")
    } else {
        format!("https://www.alphapolis.co.jp/manga/official/{source_id}")
    };
    let html = official_meta::body_capped(
        client
            .get(&url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?,
        official_meta::MAX_DETAIL_BYTES,
    )
    .await?;
    Ok(parse_detail(&html))
}

/// True for the id shapes the search emits: `novel/{n}/{n}`,
/// `manga/official/{n}`, or a bare digit run (legacy admin binding).
fn is_alphapolis_id(id: &str) -> bool {
    if official_meta::is_digit_id(id) {
        return true;
    }
    if let Some(rest) = id.strip_prefix("manga/official/") {
        return official_meta::is_digit_id(rest);
    }
    if let Some(rest) = id.strip_prefix("novel/") {
        let mut parts = rest.split('/');
        return matches!((parts.next(), parts.next(), parts.next()),
            (Some(a), Some(b), None)
                if official_meta::is_digit_id(a) && official_meta::is_digit_id(b));
    }
    false
}

/// Pure parser: schedule + exact next date + the embedded TOC episode list.
/// Malformed input degrades to `None` / an empty list.
pub fn parse_detail(html: &str) -> OfficialMeta {
    let schedule = official_meta::between(html, SCHEDULE_START, SCHEDULE_END)
        .map(|s| official_meta::strip_tags(s).trim().to_string())
        .filter(|s| !s.is_empty())
        // The `</span>` is immediately followed by the literal `更新`.
        .map(|s| format!("{s}更新"))
        .and_then(|s| official_meta::normalize_schedule(&s));
    let next_update_at = official_meta::between(html, NEXT_START, NEXT_END)
        .and_then(|s| official_meta::parse_date(s.trim()));
    let chapters = parse_episodes(html);
    let main = main_count(&chapters);
    OfficialMeta {
        source: "alphapolis".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        update_schedule: schedule,
        next_update_at,
    }
}

/// The `episodes[]` array of the `app-official-*-toc` widget, oldest-first.
/// Rows are classified by [`classify_jp_title`]; the row title is the chapter's
/// own title (`mainTitle` minus the `第N回…` prefix). Absent widget or broken
/// JSON -> empty.
pub fn parse_episodes(html: &str) -> Vec<ChapterMeta> {
    let Some(json) = toc_json(html) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.get("episodes").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let rows: Vec<EpisodeRow> = items
        .iter()
        .filter_map(|item| {
            let short = str_field(item, "shortTitle");
            let main = str_field(item, "mainTitle");
            let raw = short.or(main)?;
            let title = main
                .zip(short)
                .and_then(|(m, s)| m.strip_prefix(s))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let release_date = str_field(item, "upTime").and_then(parse_up_time);
            Some(EpisodeRow {
                title,
                release_date,
                kind: classify_jp_title(raw),
            })
        })
        .collect();
    chapter_title::rows_to_chapters(rows)
}

fn str_field<'a>(item: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    item.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `2026.09.30更新` -> `2026-09-30`.
fn parse_up_time(raw: &str) -> Option<chrono::NaiveDate> {
    official_meta::parse_date(raw.trim().trim_end_matches("更新").trim())
}

/// Slice the JSON body of the `app-official-*-toc` widget. `None` when the
/// widget (or its JSON script) is absent.
fn toc_json(html: &str) -> Option<&str> {
    let mut from = 0usize;
    while let Some(rel) = html[from..].find(TOC_ID_PREFIX) {
        let start = from + rel + TOC_ID_PREFIX.len();
        // Always advance past this marker so a broken widget cannot abort the
        // scan for a later one.
        from = start;
        let rest = &html[start..];
        let Some(quote) = rest.find('"') else {
            continue;
        };
        if !rest[..quote].ends_with(TOC_ID_SUFFIX) {
            continue;
        }
        let after_id = &rest[quote..];
        let Some(script) = after_id.find(TOC_SCRIPT) else {
            continue;
        };
        let body = &after_id[script + TOC_SCRIPT.len()..];
        let Some(end) = body.find(TOC_SCRIPT_END) else {
            continue;
        };
        return Some(body[..end].trim());
    }
    None
}

/// Guest search `https://www.alphapolis.co.jp/search?query=<kw>` (no category
/// filter: the trait passes no media_type, both kinds are returned).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse("https://www.alphapolis.co.jp/search")?;
    url.query_pairs_mut().append_pair("query", title);
    let html = official_meta::body_capped(
        client
            .get(url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?,
        official_meta::MAX_DETAIL_BYTES,
    )
    .await?;
    Ok(parse_search(&html))
}

/// Pure search parser. Novel links must be `href="/novel/{num}/{num}"` (two
/// numeric segments — rejects `/novel/index`, `/novel/add_tag_counter/{n}`);
/// manga links are `…/manga/official/{num}`, relative (real results) or
/// absolute (carousel). Title = anchor text,
/// falling back to the banner `alt="…"` (manga anchors hold only an `<img>`).
/// Deduped by prefixed id; malformed input degrades to an empty vector.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let mut hits: Vec<OfficialHit> = Vec::new();
    collect_novel_hits(html, &mut hits);
    collect_manga_hits(html, &mut hits);
    hits
}

/// `href="/novel/{a}/{b}"` where both segments are pure digits.
fn collect_novel_hits(html: &str, hits: &mut Vec<OfficialHit>) {
    let mut rest = html;
    while let Some(pos) = rest.find(NOVEL_MARKER) {
        let after = &rest[pos + NOVEL_MARKER.len()..];
        // Always advance past this marker, even when it does not match.
        rest = after;
        let Some((user, tail)) = take_digits(after) else {
            continue;
        };
        let Some(tail) = tail.strip_prefix('/') else {
            continue;
        };
        let Some((work, tail)) = take_digits(tail) else {
            continue;
        };
        // Close quote (or query string) right after the second segment.
        if !(tail.starts_with('"') || tail.starts_with('?')) {
            continue;
        }
        let id = format!("novel/{user}/{work}");
        let title = anchor_title(tail);
        push_hit(hits, id, title);
    }
}

/// Every `href` whose target is a manga detail path, relative
/// (`/manga/official/{id}` — the real search results) or absolute
/// (`https://www.alphapolis.co.jp/manga/official/{id}` — the carousel).
fn collect_manga_hits(html: &str, hits: &mut Vec<OfficialHit>) {
    const HREF: &str = "href=\"";
    const HOST: &str = "https://www.alphapolis.co.jp";
    const PATH: &str = "/manga/official/";
    let mut rest = html;
    while let Some(pos) = rest.find(HREF) {
        let after = &rest[pos + HREF.len()..];
        rest = after;
        let Some(end) = after.find('"') else { break };
        let href = &after[..end];
        // Relative result links and absolute carousel links share the path.
        let path = href.strip_prefix(HOST).unwrap_or(href);
        let Some(tail) = path.strip_prefix(PATH) else {
            continue;
        };
        let Some((id_digits, rest_of_href)) = take_digits(tail) else {
            continue;
        };
        // The id ends the path: the value itself, or a query/fragment.
        if !(rest_of_href.is_empty()
            || rest_of_href.starts_with('?')
            || rest_of_href.starts_with('#'))
        {
            continue;
        }
        let id = format!("manga/official/{id_digits}");
        // `after[end..]` starts at the closing quote so `anchor_title` can walk
        // to the tag end and read the anchor body (cover `alt`).
        let title = anchor_title(&after[end..]);
        push_hit(hits, id, title);
    }
}

/// Leading ASCII digit run of `s` (`("", …)` never — `None` when empty).
fn take_digits(s: &str) -> Option<(&str, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    (end > 0).then(|| s.split_at(end))
}

/// Title inside the anchor that starts at `s` (s points at the closing quote
/// of `href`): anchor text, falling back to the first `alt="…"` attribute.
fn anchor_title(s: &str) -> String {
    let Some(gt) = s.find('>') else {
        return String::new();
    };
    let body = &s[gt + 1..];
    let inner = match body.find("</a>") {
        Some(close) => &body[..close],
        None => body,
    };
    let text = official_meta::strip_tags(inner);
    if !text.is_empty() {
        return text;
    }
    // Banner anchors hold only `<img … alt="Title">`.
    official_meta::between(inner, "alt=\"", "\"")
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Dedup by prefixed id; empty titles are dropped.
fn push_hit(hits: &mut Vec<OfficialHit>, id: String, title: String) {
    if !title.is_empty() && !hits.iter().any(|h| h.id == id) {
        hits.push(OfficialHit { id, title });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_fixture_schedule_and_next_date() {
        let html = include_str!("../../../tests/fixtures/alphapolis_manga.html");
        let m = parse_detail(html);
        assert_eq!(m.source, "alphapolis");
        // The embedded TOC widget carries the episode list.
        assert_eq!(m.chapters.len(), 2);
        // Both rows are parts of chapter 1 (`第１回（前編）` / `（後編）`).
        assert_eq!(m.chapters[0].number_x100, 110);
        assert_eq!(m.chapters[0].label.as_deref(), Some("1-10"));
        assert_eq!(m.chapters[1].number_x100, 120);
        assert_eq!(m.chapters[1].label.as_deref(), Some("1-20"));
        assert_eq!(m.count, Some(1));
        assert_eq!(
            m.chapters[0].release_date,
            official_meta::parse_date("2026-09-30")
        );
        // The chapter's own title rides after the `第N回…` prefix.
        assert!(
            m.chapters[0]
                .title
                .as_deref()
                .is_some_and(|t| t.starts_with('『'))
        );
        assert_eq!(m.update_schedule.as_deref(), Some("毎月第4水曜日更新"));
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        assert_eq!(m.next_update_at, Some(expected));
    }

    #[test]
    fn parses_toc_episodes_and_bonuses() {
        let html = r#"<div id="app-official-manga-toc">
            <script type="application/json">
            {"mangaTitle":"T","episodes":[
              {"shortTitle":"第1回","mainTitle":"第1回『A』","upTime":"2024.01.02更新"},
              {"shortTitle":"第2回","mainTitle":"第2回","upTime":"2024.02.03更新"},
              {"shortTitle":"番外編","mainTitle":"番外編『特別』","upTime":"2024.03.04更新"}
            ]}
            </script></div>"#;
        let m = parse_detail(html);
        assert_eq!(m.chapters.len(), 3);
        // `count` is main-only: the `番外編` row is a bonus.
        assert_eq!(m.count, Some(2));
        assert_eq!(m.chapters[0].number_x100, 100);
        assert_eq!(m.chapters[0].title.as_deref(), Some("『A』"));
        // A bare `第2回` carries no own title.
        assert_eq!(m.chapters[1].number_x100, 200);
        assert!(m.chapters[1].title.is_none());
        assert!(m.chapters[2].is_bonus);
        assert_eq!(m.chapters[2].number_x100, 0);
    }

    #[test]
    fn empty_detail_degrades_to_none() {
        let m = parse_detail("<html></html>");
        assert!(m.update_schedule.is_none());
        assert!(m.next_update_at.is_none());
        assert!(m.count.is_none());
        assert!(m.chapters.is_empty());
    }

    #[test]
    fn search_fixture_ids_are_kind_prefixed_and_deduped() {
        let html = include_str!("../../../tests/fixtures/alphapolis_search.html");
        let hits = parse_search(html);
        assert!(!hits.is_empty());
        assert!(
            hits.iter()
                .all(|h| h.id.starts_with("novel/") || h.id.starts_with("manga/official/"))
        );
        // No navigation/tag links leak in (they lack two numeric segments).
        assert!(
            hits.iter()
                .all(|h| { !h.id.contains("index") && !h.id.contains("add_tag_counter") })
        );
        // Deduped by id, non-empty titles.
        let mut ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before);
        assert!(hits.iter().all(|h| !h.title.is_empty()));
        // Concrete rows from both kinds.
        assert!(hits.iter().any(|h| h.id == "novel/381661058/785090081"));
        assert!(hits.iter().any(|h| h.id == "manga/official/977000734"));
    }

    #[test]
    fn search_rejects_non_numeric_novel_segments() {
        let html = r#"<a href="/novel/index">Index</a>
                      <a href="/novel/add_tag_counter/1204">Tag</a>
                      <a href="/novel/184411353">One segment</a>
                      <a href="/novel/184411353/267621262">Good</a>"#;
        let hits = parse_search(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "novel/184411353/267621262");
        assert_eq!(hits[0].title, "Good");
    }

    #[test]
    fn manga_title_falls_back_to_alt_attribute() {
        let html = r#"<a
                        href="https://www.alphapolis.co.jp/manga/official/977000734"
                        class="manga-banner"
                      >
                        <img src="x.webp" alt="悪役令嬢は想定外の溺愛に困惑中">
                      </a>"#;
        let hits = parse_search(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "manga/official/977000734");
        assert_eq!(hits[0].title, "悪役令嬢は想定外の溺愛に困惑中");
    }

    #[test]
    fn manga_hits_from_relative_results_and_absolute_carousel() {
        // Live results use a relative path and carry the title in the cover
        // `alt`; the pickup carousel below uses absolute links.
        let html = r#"<span class="label-tag">ノーチェ（TL）</span>
            <a href="/manga/official/977000734" class="manga-banner">
              <img src="x.webp" alt="悪役令嬢は想定外の溺愛に困惑中">
            </a>
            <a href="/manga/official/977000734" class="title">悪役令嬢は想定外の溺愛に困惑中</a>
            <a href="https://www.alphapolis.co.jp/manga/official/111222333">
              <img src="y.webp" alt="Carousel Title">
            </a>"#;
        let hits = parse_search(html);
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["manga/official/977000734", "manga/official/111222333"]
        );
        // The banner anchor wins the id, so the title comes from its `alt`.
        assert_eq!(hits[0].title, "悪役令嬢は想定外の溺愛に困惑中");
    }

    #[test]
    fn empty_search_is_empty() {
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html></html>").is_empty());
    }
}
