//! Qidian CN novel via mobile m.qidian.com (the PC site anti-bots guests).
//!
//! Fixtures (all captured for book `1010868264` 《诡秘之主》):
//! - `qidian_catalog.html` — 1419 chapter-ish anchors: one *header* block
//!   (`<h3 class="_lastChapterCN">` + `<p class="_lastChapterUT">`, latest-news
//!   shortcut, cid duplicates the real last chapter) and 1418 real rows
//!   (`<div><h2>title</h2></div>` + `alt="… 首发时间: 2018-04-01 12:28:26 …"`).
//!   Rows are already chronological (first = 2018-04-01, last = 2022-11-25).
//! - `qidian_detail.html` — status (`完本`/`连载`) + last-update time. Qidian
//!   publishes **no schedule field** (`updInfo` is empty), so `parse_status`
//!   exists for completeness but `update_schedule` stays `None` (per plan).
//! - `qidian_search.html` — 20 book hits: `data-bid="{id}"` + the title as a
//!   text node inside `<h2><mark>…</mark></h2>`. The anchor `title=` attribute
//!   carries the `在线阅读` suffix and is deliberately ignored.

use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Linux; Android 13) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36";

/// GET `https://m.qidian.com/book/{id}/catalog/` (mobile; PC → anti-bot).
pub async fn fetch_meta(client: &reqwest::Client, book_id: &str) -> anyhow::Result<OfficialMeta> {
    let url = format!("https://m.qidian.com/book/{book_id}/catalog/");
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

/// Catalog-driven meta: chapters + count; Qidian has no schedule field.
pub fn parse_detail(html: &str) -> OfficialMeta {
    let chapters = parse_catalog(html);
    OfficialMeta {
        source: "qidian".to_string(),
        count: (!chapters.is_empty()).then_some(chapters.len() as i32),
        chapters,
        // Status (`完本`/`连载`) is not a frequency and there is no field for
        // it in `OfficialMeta` — never invent a schedule value here.
        update_schedule: None,
        next_update_at: None,
    }
}

/// Scan `//m.qidian.com/chapter/{bid}/{cid}/` anchors. Title comes from the
/// `<h2>` inside the anchor (which also skips the header block), the date from
/// `alt="… 首发时间: YYYY-MM-DD HH:MM:SS …"` (first 10 chars). Deduped by cid.
/// Total: any anchor without `<h2>` or cid is skipped, never a panic.
pub fn parse_catalog(html: &str) -> Vec<ChapterMeta> {
    const MARKER: &str = "//m.qidian.com/chapter/";
    let mut rows: Vec<ChapterMeta> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rest = html;
    while let Some(pos) = rest.find(MARKER) {
        let after = &rest[pos + MARKER.len()..];
        // `{bid}/{cid}/` — ids are digit runs.
        let bid_end = after.find('/').unwrap_or(after.len());
        let cid_start = bid_end + usize::from(bid_end < after.len());
        let cid_end = after[cid_start..]
            .find(|c: char| !c.is_ascii_digit())
            .map(|i| cid_start + i)
            .unwrap_or(after.len());
        let cid = &after[cid_start..cid_end];
        // Anchor body runs up to its `</a>` (contains attrs + children).
        let Some(a_end) = after.find("</a>") else { break };
        let anchor = &after[..a_end];
        rest = &after[a_end + 4..];
        // The header shortcut (`_lastChapterUT`) and non-row anchors are out.
        if cid.is_empty() || anchor.contains("_lastChapterUT") || !seen.insert(cid.to_string()) {
            continue;
        }
        // Title: text of the anchor's `<h2>` (skip past the `>` of `<h2 …>`).
        let Some(title) = official_meta::between(anchor, "<h2", "</h2>")
            .and_then(|s| s.find('>').map(|i| &s[i + 1..]))
            .map(official_meta::strip_tags)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        // Date: `首发时间: 2018-04-01 12:28:26` -> first 10 chars.
        let release_date = official_meta::between(anchor, "首发时间: ", " ")
            .and_then(|s| s.get(..10))
            .and_then(official_meta::parse_date);
        let n = rows.len() as f64 + 1.0;
        if let Some(mut c) = ChapterMeta::from_f64(n) {
            c.title = Some(title);
            c.release_date = release_date;
            rows.push(c);
        }
    }
    rows
}

/// Status marker from a *detail* page: `完本` (completed) or `连载` (ongoing).
/// Only read from the window right before the header update-time span, so the
/// SEO keywords (`…完本`) in `<head>` never match. Nothing is stored — the
/// result is documented, not wired into `OfficialMeta`.
pub fn parse_status(html: &str) -> Option<String> {
    let idx = html.find("detail__header-detail__time")?;
    let window = &html[idx.saturating_sub(400)..idx];
    let completed = window.rfind("完本").map(|p| (p, "完本"));
    let ongoing = window.rfind("连载").map(|p| (p, "连载"));
    match (completed, ongoing) {
        (Some((a, s)), Some((b, _))) if a > b => Some(s.to_string()),
        (Some((_, s)), None) => Some(s.to_string()),
        (None, Some((_, s))) => Some(s.to_string()),
        _ => None,
    }
}

/// GET `https://m.qidian.com/soushu/{URL-encoded kw}.html` (the `/search?kw=`
/// form 302s here). UA is not critical; the WAF script does not trigger.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let kw: String = url::form_urlencoded::byte_serialize(title.as_bytes()).collect();
    let html = client
        .get(format!("https://m.qidian.com/soushu/{kw}.html"))
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_search(&html))
}

/// Book hits: `data-bid="{id}"` + title from `<h2>…<mark>title</mark>…</h2>`
/// (the `title=` attribute carries `在线阅读` and is never read). Empty ids,
/// missing `<h2>` and duplicate ids are skipped; malformed HTML -> empty.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    const MARKER: &str = "data-bid=\"";
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find(MARKER) {
        let after = &rest[pos + MARKER.len()..];
        let id_end = after
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(after.len());
        let id = &after[..id_end];
        let Some(a_end) = after.find("</a>") else { break };
        let anchor = &after[..a_end];
        rest = &after[a_end + 4..];
        if id.is_empty() || hits.iter().any(|h| h.id == id) {
            continue;
        }
        let Some(title) = official_meta::between(anchor, "<h2", "</h2>")
            .and_then(|s| s.find('>').map(|i| &s[i + 1..]))
            .map(official_meta::strip_tags)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        hits.push(OfficialHit {
            id: id.to_string(),
            title,
        });
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_fixture_rows() {
        let html = include_str!("../../../tests/fixtures/qidian_catalog.html");
        let m = parse_detail(html);
        assert_eq!(m.source, "qidian");
        assert!(m.update_schedule.is_none(), "qidian has no schedule field");
        assert!(m.next_update_at.is_none());
        // 1418 unique rows; the `_lastChapterUT` header shortcut is skipped.
        assert_eq!(m.chapters.len(), 1418);
        assert_eq!(m.count, Some(1418));
        assert_eq!(m.chapters[0].number_x100, 100);
        assert_eq!(m.chapters[0].title.as_deref(), Some("第一章 绯红"));
        assert_eq!(
            m.chapters[0].release_date,
            official_meta::parse_date("2018-04-01")
        );
        assert_eq!(m.chapters[1417].number_x100, 141800);
        assert_eq!(
            m.chapters[1417].title.as_deref(),
            Some("12月1日《诡秘之主》新番外发布")
        );
        assert_eq!(
            m.chapters[1417].release_date,
            official_meta::parse_date("2022-11-25")
        );
    }

    #[test]
    fn detail_fixture_status_only() {
        // The detail page is NOT a catalog: no `<h2>` rows -> zero chapters,
        // but the status marker is readable.
        let html = include_str!("../../../tests/fixtures/qidian_detail.html");
        let m = parse_detail(html);
        assert!(m.chapters.is_empty(), "detail page must not yield rows");
        assert!(m.count.is_none());
        assert!(m.update_schedule.is_none());
        assert_eq!(parse_status(html).as_deref(), Some("完本"));
        assert!(parse_status("<html></html>").is_none());
    }

    #[test]
    fn search_fixture_hits_without_suffix() {
        let html = include_str!("../../../tests/fixtures/qidian_search.html");
        let hits = parse_search(html);
        assert_eq!(hits.len(), 20);
        assert_eq!(hits[0].id, "1010868264");
        assert_eq!(hits[0].title, "诡秘之主");
        assert!(
            hits.iter().all(|h| !h.id.is_empty() && !h.title.is_empty()),
            "every hit needs id and title"
        );
        assert!(
            hits.iter().all(|h| !h.title.contains("在线阅读")),
            "the title= attribute suffix must never leak into titles"
        );
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn empty_input() {
        assert!(parse_catalog("").is_empty());
        assert!(parse_search("").is_empty());
        let m = parse_detail("");
        assert!(m.chapters.is_empty() && m.count.is_none());
    }
}
