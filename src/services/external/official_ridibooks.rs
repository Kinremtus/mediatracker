//! Ridibooks KR detail (Next.js SSR) + guest search.
//!
//! Both pages embed their state in `__NEXT_DATA__` JSON:
//!
//! - Search (`/search?q=`): `props.pageProps.gridData.riGrid.grid.cells[*]
//!   .cell__SearchBookListWithTab.books[*]` -> `book.series.id` (the series id
//!   the detail page needs; `book.id` is an episode id, kept as a fallback)
//!   plus `book.title.main` with the trailing `<n>화`/`<n>권` episode suffix
//!   and catalog markers stripped, so the hit title equals the series title for
//!   matching.
//! - Detail (`/series/<id>`): the `SeriesBooks` entry of
//!   `props.pageProps.dehydratedState.queries[*]` carries the FULL
//!   oldest-first episode list (`seriesTitle`, `items[].bookShell.book`).
//!   Numbers come from the trailing `<n>화` of each title (positions as the fallback); unnumbered rows in a numbered series
//!   (previews, notices) are bonus rows. The list carries no dates, so
//!   `release_date` stays `None`.

use crate::services::external::chapter_title::{
    self, EpisodeRow, ShortTitle, main_count, parse_kr_chapter_number, strip_catalog_markers,
    strip_kr_episode_suffix,
};
use crate::services::external::official_meta::{self, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Row cap for the series episode list: a hostile or degenerate payload must
/// not turn into tens of thousands of chapter rows (naver and kakao cap their
/// pages too).
const RIDIBOOKS_MAX_ROWS: usize = 2000;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "ridibooks".to_string(),
        ..OfficialMeta::default()
    }
}

/// GET `https://ridibooks.com/series/{series_id}` and parse the SSR payload.
pub async fn fetch_meta(client: &reqwest::Client, series_id: &str) -> anyhow::Result<OfficialMeta> {
    // The id lands in the URL path: only digits are ever valid.
    if !official_meta::is_digit_id(series_id) {
        return Ok(empty_meta());
    }
    let url = format!("https://ridibooks.com/series/{series_id}");
    let response = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?;
    if response
        .content_length()
        .is_some_and(|len| len > official_meta::MAX_DETAIL_BYTES)
    {
        tracing::warn!(series_id, "ridibooks: series page too large, skipping");
        return Ok(empty_meta());
    }
    let html = response.error_for_status()?.text().await?;
    Ok(parse_series(&html))
}

/// Pure parser (see module docs). Malformed / truncated HTML -> empty meta.
pub fn parse_series(html: &str) -> OfficialMeta {
    let Some(value) = official_meta::next_data_str(html)
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
    else {
        return empty_meta();
    };
    let Some(books) = value
        .pointer("/props/pageProps/dehydratedState/queries")
        .and_then(|v| v.as_array())
        .and_then(|queries| {
            queries.iter().find_map(|q| {
                let key = q.get("queryKey")?.as_array()?;
                if key.first()?.as_str()? != "SeriesBooks" {
                    return None;
                }
                q.pointer("/state/data/seriesBooks")
            })
        })
    else {
        return empty_meta();
    };
    let series_title = books
        .get("seriesTitle")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let Some(items) = books.get("items").and_then(|v| v.as_array()) else {
        return empty_meta();
    };

    // Oldest-first as served, capped. A numbered title wins; in a numbered
    // series an unnumbered row (preview, notice) is a bonus row; a fully
    // unnumbered list falls back to positions so a skeleton still appears.
    let stems: Vec<Option<String>> = items
        .iter()
        .take(RIDIBOOKS_MAX_ROWS)
        .map(|item| {
            item.pointer("/bookShell/book/title/main")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| strip_catalog_markers(s).to_string())
        })
        .collect();
    let numbered = stems
        .iter()
        .flatten()
        .any(|stem| parse_kr_chapter_number(stem).is_some_and(|n| n >= 1));
    let rows: Vec<EpisodeRow> = stems
        .into_iter()
        .enumerate()
        .filter_map(|(i, stem)| {
            let stem = stem?;
            let kind = match parse_kr_chapter_number(&stem) {
                Some(n) if n >= 1 => ShortTitle::Base(n),
                _ if chapter_title::is_kr_bonus_title(&stem) => ShortTitle::Unnumbered,
                _ if numbered => ShortTitle::Unnumbered,
                _ => ShortTitle::Base(i32::try_from(i + 1).unwrap_or(i32::MAX)),
            };
            Some(EpisodeRow {
                title: display_title(&stem, &series_title),
                release_date: None,
                kind,
            })
        })
        .collect();
    let chapters = chapter_title::rows_to_chapters(rows);
    let main = main_count(&chapters);
    if !chapters.is_empty() {
        tracing::debug!(
            source = "ridibooks",
            rows = chapters.len(),
            main,
            "series parsed"
        );
    }
    OfficialMeta {
        source: "ridibooks".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        update_schedule: None,
        next_update_at: None,
    }
}

/// `인류 최대의 실수 1화` (series `인류 최대의 실수`) -> `None`: the series
/// prefix plus a bare episode number carries nothing new. `… 예고편` keeps the
/// remainder as the row title.
fn display_title(stem: &str, series_title: &str) -> Option<String> {
    let remainder = stem
        .strip_prefix(series_title)
        .map(str::trim_start)
        .unwrap_or(stem);
    let remainder = strip_catalog_markers(remainder);
    if remainder.is_empty() || parse_kr_chapter_number(remainder).is_some() {
        None
    } else {
        Some(remainder.to_string())
    }
}

/// GET `https://ridibooks.com/search?q={kw}` (anonymous; CF cookies ok).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse("https://ridibooks.com/search")?;
    url.query_pairs_mut().append_pair("q", title);
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

/// Walk `cells[*].cell__SearchBookListWithTab.books[*]`. The stored id is the
/// SERIES id (`book.series.id`) — the `/series/<id>` detail page needs it;
/// `book.id` (an episode id) is only a fallback. The hit title is the series
/// name: the episode suffix (`1화`/`1권`) and catalog markers are stripped so
/// `titles_match` can compare it with the clean candidate.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let Some(value) = official_meta::next_data_str(html)
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
    else {
        return Vec::new();
    };
    let Some(cells) = value
        .pointer("/props/pageProps/gridData/riGrid/grid/cells")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for cell in cells {
        let Some(books) = cell
            .get("cell__SearchBookListWithTab")
            .and_then(|b| b.get("books"))
            .and_then(|b| b.as_array())
        else {
            continue;
        };
        for entry in books {
            let book = entry.get("book").unwrap_or(entry);
            let Some(id) = book
                .pointer("/series/id")
                .and_then(|v| id_string(Some(v)))
                .or_else(|| id_string(book.get("id")))
                .or_else(|| id_string(entry.get("id")))
            else {
                continue;
            };
            // The id lands in the `/series/<id>` URL path: digits only.
            if !official_meta::is_digit_id(&id) {
                continue;
            }
            let Some(main) = book
                .pointer("/title/main")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let title = strip_kr_episode_suffix(strip_catalog_markers(main));
            if title.is_empty() {
                continue;
            }
            if !hits.iter().any(|h| h.id == id) {
                hits.push(OfficialHit {
                    id,
                    title: title.to_string(),
                });
            }
        }
    }
    hits
}

/// JSON id (number or string) -> `String`; anything else -> `None`.
fn id_string(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn series_fixture_parses_the_full_list() {
        let html = include_str!("../../../tests/fixtures/ridibooks_series.html");
        let m = parse_series(html);
        assert_eq!(m.source, "ridibooks");
        // 19 episodes, oldest-first, all main; the last one is `19화 (완결)`.
        assert_eq!(m.chapters.len(), 19);
        assert_eq!(m.count, Some(19));
        assert_eq!(m.chapters[0].number_x100, 100);
        assert!(!m.chapters[0].is_bonus);
        // `… 1화` carries no title beyond the series name.
        assert_eq!(m.chapters[0].title, None);
        assert_eq!(m.chapters[18].number_x100, 1900);
        assert!(!m.chapters[18].is_bonus);
        assert!(m.chapters.iter().all(|c| !c.is_bonus));
        assert!(m.update_schedule.is_none());
    }

    #[test]
    fn unnumbered_rows_in_a_numbered_series_are_bonuses() {
        let html = r#"<script id="__NEXT_DATA__">{"props":{"pageProps":{"dehydratedState":{"queries":[{"queryKey":["SeriesBooks"],"state":{"data":{"seriesBooks":{"seriesTitle":"T","totalItemCount":3,"items":[{"bookShell":{"book":{"id":1,"title":{"main":"T 1화"}}}},{"bookShell":{"book":{"id":2,"title":{"main":"T 예고편"}}}},{"bookShell":{"book":{"id":3,"title":{"main":"T 2화"}}}}]}}}}]}}}}</script>"#;
        let m = parse_series(html);
        let numbers: Vec<i32> = m.chapters.iter().map(|c| c.number_x100).collect();
        assert_eq!(numbers, vec![100, 0, 200]);
        let bonus: Vec<bool> = m.chapters.iter().map(|c| c.is_bonus).collect();
        assert_eq!(bonus, vec![false, true, false]);
        assert_eq!(m.chapters[1].title.as_deref(), Some("예고편"));
        assert_eq!(m.count, Some(2));
    }

    #[test]
    fn series_without_numbers_falls_back_to_positions() {
        let html = r#"<script id="__NEXT_DATA__">{"props":{"pageProps":{"dehydratedState":{"queries":[{"queryKey":["SeriesBooks"],"state":{"data":{"seriesBooks":{"seriesTitle":"T","totalItemCount":2,"items":[{"bookShell":{"book":{"id":1,"title":{"main":"T 첫번째"}}}},{"bookShell":{"book":{"id":2,"title":{"main":"T 두번째"}}}}]}}}}]}}}}</script>"#;
        let m = parse_series(html);
        assert_eq!(m.chapters.len(), 2);
        assert_eq!(m.chapters[0].number_x100, 100);
        assert_eq!(m.chapters[1].number_x100, 200);
        assert_eq!(m.count, Some(2));
    }

    #[test]
    fn empty_and_malformed_input() {
        let m = parse_series("<html></html>");
        assert_eq!(m.source, "ridibooks");
        assert!(m.chapters.is_empty() && m.count.is_none());
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn parses_search_fixture() {
        let html = include_str!("../../../tests/fixtures/ridibooks_search.html");
        let hits = parse_search(html);
        // 24 books of 142 matches are on the first page of the fixture.
        assert_eq!(hits.len(), 24);
        assert!(hits.iter().all(|h| !h.id.is_empty() && !h.title.is_empty()));
        assert_eq!(hits[0].id, "777035894");
        // The episode suffix is stripped: the hit title is the series name.
        assert_eq!(hits[0].title, "나 혼자만 레벨업");
    }
}
