//! Comic Walker (Kadokawa) JP manga: SSR `__NEXT_DATA__` JSON.
//!
//! The fixture stores episodes under `props.pageProps.dehydratedState.queries`
//! as several *duplicate* lists: `firstEpisodes.result` (oldest -> newest),
//! `latestEpisodes.result` (newest -> oldest) and one-element
//! `firstComic.episodes` / `comics.result[n].episodes` lists. The episode key
//! is `title` (the plan's candidate `episodeTitle` never appears). A
//! string-scan would therefore emit 145 rows for 71 real episodes, so the
//! parser fully parses the embedded JSON (`serde_json`, allowed for embedded
//! JSON) and keeps the *longest* array whose items carry `title`/`updateDate`,
//! then sorts rows by date (undated rows last) and numbers them 1..N.
//!
//! `nextUpdateDateText` (the guaranteed anchor) is absent on the captured
//! fixture, so `next_update_at` degrades to `None`. Search parses the SSR
//! `__NEXT_DATA__` work items keyed by `workCode`.

use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// GET `https://comic-walker.com/detail/<work_code>` and parse the SSR page.
pub async fn fetch_meta(client: &reqwest::Client, work_code: &str) -> anyhow::Result<OfficialMeta> {
    let url = format!("https://comic-walker.com/detail/{work_code}");
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

/// Pure parser: extract `__NEXT_DATA__`, pull the episode list and the exact
/// next-update date. Truncated / non-SSR HTML degrades to an empty meta.
pub fn parse_detail(html: &str) -> OfficialMeta {
    let next_data = next_data_json(html);
    let chapters = parse_episodes(next_data);
    let next_update_at = official_meta::json_string_field(next_data, "nextUpdateDateText")
        .and_then(|s| official_meta::parse_date(&s));
    OfficialMeta {
        source: "comicwalker".to_string(),
        count: (!chapters.is_empty()).then_some(chapters.len() as i32),
        chapters,
        update_schedule: None,
        next_update_at,
    }
}

/// Slice between `__NEXT_DATA__` and `</script>`, dropping the tag tail
/// (` type="application/json">`) so the result starts at the opening `{`.
fn next_data_json(html: &str) -> &str {
    let seg = official_meta::between(html, "__NEXT_DATA__", "</script>").unwrap_or("");
    match seg.find('{') {
        Some(i) => &seg[i..],
        None => "",
    }
}

/// Parse the episode list out of the `__NEXT_DATA__` JSON. Picks the longest
/// array of objects carrying `title`/`episodeTitle` + `updateDate`, sorts by
/// date ascending (stable; undated rows keep document order and land last) and
/// numbers chapters 1..N. Empty / malformed input yields `Vec::new()`.
pub fn parse_episodes(json: &str) -> Vec<ChapterMeta> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut lists: Vec<Vec<(Option<String>, Option<chrono::NaiveDate>)>> = Vec::new();
    collect_episode_lists(&value, &mut lists);
    let Some(mut rows) = lists.into_iter().max_by_key(|l| l.len()) else {
        return Vec::new();
    };
    // Comic Walker serves both directions; normalise to oldest-first.
    rows.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    rows.into_iter()
        .enumerate()
        .map(|(i, (title, release_date))| ChapterMeta {
            number_x100: ((i + 1) * 100) as i32,
            title,
            release_date,
        })
        .collect()
}

/// Depth-first walk over the JSON tree, collecting every array whose first
/// element looks like an episode (`title`/`episodeTitle` + `updateDate`).
fn collect_episode_lists(
    value: &serde_json::Value,
    out: &mut Vec<Vec<(Option<String>, Option<chrono::NaiveDate>)>>,
) {
    match value {
        serde_json::Value::Array(items) => {
            if let Some(rows) = episode_rows(items) {
                out.push(rows);
            }
            for item in items {
                collect_episode_lists(item, out);
            }
        }
        serde_json::Value::Object(map) => {
            for v in map.values() {
                collect_episode_lists(v, out);
            }
        }
        _ => {}
    }
}

/// Rows of one candidate array; `None` when the array is not an episode list.
fn episode_rows(items: &[serde_json::Value]) -> Option<Vec<(Option<String>, Option<chrono::NaiveDate>)>> {
    let first = items.first()?.as_object()?;
    if !first.contains_key("updateDate")
        || !(first.contains_key("title") || first.contains_key("episodeTitle"))
    {
        return None;
    }
    Some(
        items
            .iter()
            .filter_map(|item| {
                let obj = item.as_object()?;
                let title = obj
                    .get("title")
                    .or_else(|| obj.get("episodeTitle"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let release_date = obj
                    .get("updateDate")
                    .and_then(|v| v.as_str())
                    .and_then(parse_update_date);
                Some((title, release_date))
            })
            .collect(),
    )
}

/// `updateDate` arrives as RFC3339 (`2022-10-06T02:00:00Z`); `parse_date`
/// wants a bare `YYYY-MM-DD`, so drop the time part first.
fn parse_update_date(raw: &str) -> Option<chrono::NaiveDate> {
    let head = raw.split(['T', ' ']).next().unwrap_or(raw);
    official_meta::parse_date(head)
}

/// GET `https://comic-walker.com/search?keyword=<title>` (public SSR page).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse("https://comic-walker.com/search")?;
    url.query_pairs_mut().append_pair("keyword", title);
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

/// Pure search parser: scan `__NEXT_DATA__` for `"workCode":"<id>"` items and
/// take the sibling `title`/`workTitle` as the display name. Dedups by code;
/// malformed input degrades to an empty vector.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let next_data = next_data_json(html);
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut rest = next_data;
    while let Some(pos) = rest.find("\"workCode\"") {
        let seg = &rest[pos..];
        let code = official_meta::json_string_field(seg, "workCode");
        rest = &seg["\"workCode\"".len()..];
        let Some(code) = code else { continue };
        let name = official_meta::json_string_field(seg, "title")
            .or_else(|| official_meta::json_string_field(seg, "workTitle"))
            .unwrap_or_default();
        if !name.is_empty() && !hits.iter().any(|h| h.id == code) {
            hits.push(OfficialHit { id: code, title: name });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_detail_fixture() {
        let html = include_str!("../../../tests/fixtures/comicwalker_detail.html");
        let m = parse_detail(html);
        assert_eq!(m.source, "comicwalker");
        // One 71-episode list wins over the duplicated first/latest views.
        assert_eq!(m.chapters.len(), 71);
        assert_eq!(m.count, Some(71));
        // Oldest-first numbering: the fixture's first row is dated.
        assert_eq!(m.chapters[0].number_x100, 100);
        assert!(m.chapters[0].release_date.is_some());
        assert!(m.chapters[0].title.is_some());
        // Date-sorted ascending.
        let dates: Vec<_> = m
            .chapters
            .iter()
            .filter_map(|c| c.release_date)
            .collect();
        let mut sorted = dates.clone();
        sorted.sort();
        assert_eq!(dates, sorted);
        // `nextUpdateDateText` is not on this page.
        assert!(m.next_update_at.is_none());
        assert!(m.update_schedule.is_none());
    }

    #[test]
    fn sorts_newest_first_list_ascending() {
        // A single newest-first list must still come out oldest-first.
        let json = r#"{"latest":[{"title":"B","updateDate":"2024-05-01"},
                      {"title":"A","updateDate":"2023-01-01"}]}"#;
        let rows = parse_episodes(json);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title.as_deref(), Some("A"));
        assert_eq!(rows[1].title.as_deref(), Some("B"));
        assert_eq!(rows[1].number_x100, 200);
    }

    #[test]
    fn truncated_is_empty() {
        assert!(parse_episodes("").is_empty());
        assert!(parse_episodes("<html></html>").is_empty());
        let m = parse_detail("<html></html>");
        assert!(m.chapters.is_empty());
        assert!(m.count.is_none());
        assert!(m.next_update_at.is_none());
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn parses_search_from_next_data() {
        let html = r#"<script id="__NEXT_DATA__" type="application/json">
            {"works":[{"workCode":"KC_000001_S","title":"One Piece"},
                      {"workCode":"KC_000001_S","title":"dup"},
                      {"workCode":"KC_000002_S","workTitle":"Naruto"}]}
            </script>"#;
        let hits = parse_search(html);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "KC_000001_S");
        assert_eq!(hits[0].title, "One Piece");
        assert_eq!(hits[1].title, "Naruto");
    }
}
