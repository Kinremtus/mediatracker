//! Manga UP! Global (official EN simulpub of JP titles) via `__NEXT_DATA__`.
//!
//! Fixture reality checks (documented deviations from the plan):
//! - The plan's `"chapterTitle"` / `"publishedAt"` / `"mangaId"` keys do not
//!   exist. Actual payload: `props.pageProps.data.chapters[]` with
//!   `mainName` + `published` (`"Dec 03, 2025"`, parsed as `%b %d, %Y`),
//!   ordered newest-first -> reversed to oldest-first (Batches 0-10 invariant).
//! - The detail JSON carries **no** schedule/next-update keys at all
//!   (verified: zero `*schedul*`/`*next*`/`*update*` matches), so
//!   `update_schedule`/`next_update_at` stay `None` — nothing is invented;
//!   the plan's `updateSchedule`/`nextUpdateDate` lookups are kept for live
//!   payloads that may gain them.
//! - Search JSON exposes `titleId` (number) + `titleName` in
//!   `props.pageProps.data.titles[]` — 448 unique ids on the fixture page.
//!   `titleId` doubles as the `/manga/{id}` path segment.

use crate::services::external::chapter_title::{
    EpisodeRow, ShortTitle, classify_mangaup_title, main_count, rows_to_chapters,
};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

/// Host serving the cover assets (`imageUrl` paths are relative).
const MANGAUP_ASSET_BASE: &str = "https://global-api.manga-up.com";

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Detail page -> count/titles/dates (+ schedule keys when present).
pub async fn fetch_meta(client: &reqwest::Client, manga_id: &str) -> anyhow::Result<OfficialMeta> {
    // The id lands in the URL path: only digits are ever valid.
    if !official_meta::is_digit_id(manga_id) {
        return Ok(OfficialMeta {
            source: "mangaup".to_string(),
            ..OfficialMeta::default()
        });
    }
    let url = format!("https://global.manga-up.com/manga/{manga_id}");
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

/// `__NEXT_DATA__` script body trimmed to the first `{` (the tag's attribute
/// tail would otherwise break `serde_json`). No marker -> `""`.
fn page_json(html: &str) -> &str {
    official_meta::next_data_str(html).unwrap_or("")
}

/// Parse the detail page; any malformed/missing payload degrades to an empty
/// `OfficialMeta` with `source: "mangaup"`.
pub fn parse_detail(html: &str) -> OfficialMeta {
    let json = page_json(html);
    let value: serde_json::Value = serde_json::from_str(json).unwrap_or_default();

    // Optional free-text schedule: only real payload keys, never invented.
    let update_schedule = official_meta::json_string_field(json, "updateSchedule")
        .or_else(|| official_meta::json_string_field(json, "schedule"))
        .and_then(|s| official_meta::normalize_schedule(&s));
    let next_update_at = official_meta::json_string_field(json, "nextUpdateDate")
        .or_else(|| official_meta::json_string_field(json, "nextUpdateDateText"))
        .and_then(|s| official_meta::parse_date(&s));

    let chapters = parse_chapters(&value);
    let count = main_count(&chapters);

    OfficialMeta {
        source: "mangaup".to_string(),
        count: (count > 0).then_some(count as i32),
        chapters,
        update_schedule,
        next_update_at,
    }
}

/// `props.pageProps.data.chapters[]` (payload order = newest-first) ->
/// oldest-first `ChapterMeta` with global 1-based numbering
/// (`number_x100 = (i + 1) * 100`, batches 0-10 invariant).
pub fn parse_chapters(value: &serde_json::Value) -> Vec<ChapterMeta> {
    let Some(list) = value
        .pointer("/props/pageProps/data/chapters")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let mut rows: Vec<(Option<String>, Option<chrono::NaiveDate>)> = list
        .iter()
        .map(|ch| {
            let title = ch
                .get("mainName")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let release_date = ch
                .get("published")
                .and_then(|v| v.as_str())
                .and_then(parse_published);
            (title, release_date)
        })
        .collect();
    rows.reverse(); // newest-first payload -> oldest-first storage
    let episodes: Vec<EpisodeRow> = rows
        .into_iter()
        .map(|(title, release_date)| EpisodeRow {
            kind: title
                .as_deref()
                .map(classify_mangaup_title)
                .unwrap_or(ShortTitle::Unnumbered),
            title,
            release_date,
        })
        .collect();
    rows_to_chapters(episodes)
}

/// Fixture dates are US-style `"Dec 03, 2025"`; fall back to the generic
/// parser for ISO-ish live payloads.
fn parse_published(raw: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(raw.trim(), "%b %d, %Y")
        .ok()
        .or_else(|| official_meta::parse_date(raw))
}

/// Guest search page (no auth, no Origin).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse("https://global.manga-up.com/search")?;
    url.query_pairs_mut().append_pair("q", title);
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
    // The search page is a static catalogue (`__N_SSG`, the query is ignored
    // server-side): keep only the titles that match the query, or every search
    // would store the whole 450-title catalogue.
    Ok(crate::services::official_title::filter_hits_by_query(
        parse_search(&html),
        title,
    ))
}

/// Raw scan over the `__NEXT_DATA__` JSON for `"titleId": <number>` +
/// the following `"titleName"` — survives nesting changes that would break a
/// fixed pointer path. Duplicates dropped; malformed body -> empty vector.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let json = page_json(html);
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut rest = json;
    while let Some(pos) = rest.find("\"titleId\"") {
        let seg = &rest[pos..];
        rest = &seg["\"titleId\"".len()..];
        let Some(num) = official_meta::json_number_field(seg, "titleId") else {
            continue;
        };
        let Some(title) = official_meta::json_string_field(seg, "titleName")
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let id = num.to_string();
        if !hits.iter().any(|h| h.id == id) {
            // The cover path follows `titleName` inside the same answer object.
            let poster_url = official_meta::json_string_field(seg, "imageUrl")
                .map(|path| format!("{MANGAUP_ASSET_BASE}{path}"));
            hits.push(OfficialHit::with_extra(
                id,
                title,
                HitExtra {
                    poster_url,
                    ..HitExtra::default()
                },
            ));
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_fixture_parses_oldest_first() {
        let html = include_str!("../../../tests/fixtures/mangaup_detail.html");
        let m = parse_detail(html);
        assert_eq!(m.source, "mangaup");
        assert_eq!(m.chapters.len(), 66, "fixture carries 66 chapters");
        // `count` is the highest main base (`Chapter 25.5` is a part of 25),
        // not the row count.
        assert_eq!(m.count, Some(25));
        assert_eq!(main_count(&m.chapters), 25);
        // Oldest first after the reverse.
        assert_eq!(m.chapters[0].title.as_deref(), Some("Chapter 1"));
        assert_eq!(
            m.chapters[0].release_date.map(|d| d.to_string()).as_deref(),
            Some("2022-07-01")
        );
        assert_eq!(m.chapters[0].number_x100, 100);
        // A split part keeps its own number: `Chapter 10.1` -> 1001.
        assert!(
            m.chapters
                .iter()
                .any(|c| c.number_x100 == 1001 && c.label.as_deref() == Some("10-1"))
        );
        // The newest row is `Chapter 25.5` -> Part(25, 5), not chapter 66.
        assert_eq!(m.chapters[65].title.as_deref(), Some("Chapter 25.5"));
        assert_eq!(m.chapters[65].number_x100, 2505);
        assert_eq!(m.chapters[65].label.as_deref(), Some("25-5"));
        assert!(!m.chapters[65].is_bonus);
        assert_eq!(
            m.chapters[65]
                .release_date
                .map(|d| d.to_string())
                .as_deref(),
            Some("2025-12-03")
        );
        // Every fixture row is a numbered chapter (no bonus markers).
        assert!(m.chapters.iter().all(|c| !c.is_bonus));
        // No schedule keys in this payload — nothing is invented.
        assert!(m.update_schedule.is_none());
        assert!(m.next_update_at.is_none());
    }

    #[test]
    fn empty_and_malformed_degrade() {
        let m = parse_detail("");
        assert_eq!(m.source, "mangaup");
        assert!(m.chapters.is_empty() && m.count.is_none());
        let m = parse_detail("<html><script id=\"__NEXT_DATA__\">{broken</script></html>");
        assert!(m.chapters.is_empty());
    }

    #[test]
    fn search_fixture_parses_all_titles() {
        let html = include_str!("../../../tests/fixtures/mangaup_search.html");
        let hits = parse_search(html);
        assert_eq!(hits.len(), 448, "fixture page lists 448 unique titleIds");
        assert_eq!(hits[0].id, "1023");
        assert_eq!(
            hits[0].title,
            "25 Years in a Dungeon: A Suspicious Man Returns as a Dungeon Streamer!"
        );
        assert!(hits.iter().all(|h| !h.id.is_empty() && !h.title.is_empty()));
        // Ids double as the /manga/{id} path segment: digits only.
        assert!(
            hits.iter()
                .all(|h| h.id.chars().all(|c| c.is_ascii_digit()))
        );
        // No duplicates.
        let mut ids: Vec<_> = hits.iter().map(|h| h.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), hits.len());
        // Rich: the cover follows `titleName` in the same answer object, and
        // the JSON `\u0026` escape decodes into the URL's `&`.
        assert_eq!(
            hits[0].extra.poster_url.as_deref(),
            Some(
                "https://global-api.manga-up.com/asset/Qud/en/manga_list/1023.webp?h=AQp2kMW-8Uq5arnwSAIN2w&e=1791212400"
            )
        );
        // The query filter keeps the exact hit out of the 448-title catalogue.
        let filtered = crate::services::official_title::filter_hits_by_query(
            parse_search(html),
            "25 Years in a Dungeon: A Suspicious Man Returns as a Dungeon Streamer!",
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "1023");
        // A query nothing matches yields no hits instead of the catalogue.
        assert!(
            crate::services::official_title::filter_hits_by_query(parse_search(html), "zzzqwerty")
                .is_empty()
        );
        // Empty page degrades.
        assert!(parse_search("<html>no data</html>").is_empty());
    }
}
