//! Alpha Polis: schedule/next-date enricher + auto-bind guest search.
//!
//! The manga detail page carries the schedule block
//! (`<span class="date">毎月第4水曜日</span>更新`) and the exact next date
//! (`(次回更新日 : 2026.10.28)`). The TOC is behind a JS charge-widget, so the
//! enricher deliberately reports **no count and no chapters** — schedule and
//! next-date only.
//!
//! Search encodes the media kind in the hit id because the `OfficialSearch`
//! trait passes no `media_type`: `novel/{userId}/{workId}` (composite path)
//! vs `manga/official/{id}`. `official_resolve::hit_matches_media_type`
//! filters by the item's kind; the prefixed id **is** the site path
//! (Decision #9). Manga links in the results page are **absolute**
//! (`https://www.alphapolis.co.jp/manga/official/{id}`) — a relative
//! `href="/manga/official/"` marker matches nothing. Novel links must have
//! two numeric path segments, which excludes the `/novel/index` and
//! `/novel/add_tag_counter/{n}` navigation/tag links.

use crate::services::external::official_meta::{self, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Schedule text lives inside `<span class="date">…</span>更新`.
const SCHEDULE_START: &str = "class=\"date\">";
const SCHEDULE_END: &str = "</span>";
/// Exact next date: `(次回更新日 : 2026.10.28)`.
const NEXT_START: &str = "次回更新日 : ";
const NEXT_END: &str = ")";

const NOVEL_MARKER: &str = "href=\"/novel/";
/// Absolute manga result links (a relative marker matches zero rows).
const MANGA_MARKER: &str = "href=\"https://www.alphapolis.co.jp/manga/official/";

/// GET one Alpha Polis title. Auto-bind ids are already site paths
/// (`novel/{u}/{w}` / `manga/official/{id}`); legacy admin-bound ids are
/// bare manga ids.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    let url = if source_id.starts_with("novel/") || source_id.starts_with("manga/official/") {
        format!("https://www.alphapolis.co.jp/{source_id}")
    } else {
        format!("https://www.alphapolis.co.jp/manga/official/{source_id}")
    };
    let html = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_detail(&html))
}

/// Pure parser: schedule + exact next date. Never reports a count or chapters
/// (the TOC is not readable without JS). Malformed input degrades to `None`.
pub fn parse_detail(html: &str) -> OfficialMeta {
    let schedule = official_meta::between(html, SCHEDULE_START, SCHEDULE_END)
        .map(|s| official_meta::strip_tags(s).trim().to_string())
        .filter(|s| !s.is_empty())
        // The `</span>` is immediately followed by the literal `更新`.
        .map(|s| format!("{s}更新"))
        .and_then(|s| official_meta::normalize_schedule(&s));
    let next_update_at = official_meta::between(html, NEXT_START, NEXT_END)
        .and_then(|s| official_meta::parse_date(s.trim()));
    OfficialMeta {
        source: "alphapolis".to_string(),
        chapters: Vec::new(),
        count: None,
        update_schedule: schedule,
        next_update_at,
    }
}

/// Guest search `https://www.alphapolis.co.jp/search?query=<kw>` (no category
/// filter: the trait passes no media_type, both kinds are returned).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse("https://www.alphapolis.co.jp/search")?;
    url.query_pairs_mut().append_pair("query", title);
    let html = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_search(&html))
}

/// Pure search parser. Novel links must be `href="/novel/{num}/{num}"` (two
/// numeric segments — rejects `/novel/index`, `/novel/add_tag_counter/{n}`);
/// manga links are absolute `…/manga/official/{num}`. Title = anchor text,
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

/// Absolute `href="https://www.alphapolis.co.jp/manga/official/{id}"`.
fn collect_manga_hits(html: &str, hits: &mut Vec<OfficialHit>) {
    let mut rest = html;
    while let Some(pos) = rest.find(MANGA_MARKER) {
        let after = &rest[pos + MANGA_MARKER.len()..];
        rest = after;
        let Some((id_digits, tail)) = take_digits(after) else {
            continue;
        };
        if !(tail.starts_with('"') || tail.starts_with('?')) {
            continue;
        }
        let id = format!("manga/official/{id_digits}");
        let title = anchor_title(tail);
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
        // Schedule-only enricher: never a count, never chapters.
        assert!(m.count.is_none());
        assert!(m.chapters.is_empty());
        assert_eq!(m.update_schedule.as_deref(), Some("毎月第4水曜日更新"));
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        assert_eq!(m.next_update_at, Some(expected));
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
    fn empty_search_is_empty() {
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html></html>").is_empty());
    }
}
