//! Kakuyomu JP novel: `__NEXT_DATA__` (Apollo/Relay state).
//!
//! Episodes live in `props.pageProps.__APOLLO_STATE__` as
//! `Episode:<id>` objects (`__typename`, `id`, `title`, `publishedAt`) in
//! already-oldest-first order — 503 rows for the fixture work. A naive
//! string-scan over `"publishedAt"` would also pick up the 31 non-episode
//! objects (e.g. `Work.publishedAt`), so the parser fully parses the embedded
//! JSON (allowed for embedded JSON) and keeps only `__typename == "Episode"`
//! objects, then numbers them 1..N after a stable date sort.
//!
//! There is no schedule on the fixture: `Work.schedule` is `null` and the only
//! `"description"` string in the page is the SEO `<meta name="description">`,
//! not a schedule — `update_schedule` degrades to `None`.

use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// GET `https://kakuyomu.jp/works/<work_id>` and parse the SSR page.
pub async fn fetch_meta(client: &reqwest::Client, work_id: &str) -> anyhow::Result<OfficialMeta> {
    let url = format!("https://kakuyomu.jp/works/{work_id}");
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

/// Pure parser: extract `__NEXT_DATA__`, pull the schedule and the episode
/// list. The page carries no schedule text (`"schedule":null` -> None); the
/// SEO `<meta name="description">` synopsis is never treated as a schedule.
/// Truncated / non-SSR HTML degrades to an empty meta.
pub fn parse_detail(html: &str) -> OfficialMeta {
    let json = next_data_json(html);
    // `Work.schedule` is the only schedule carrier; `null` degrades to None
    // (json_string_field requires a quoted value). The SEO
    // `<meta name="description">` lives outside the JSON and is never read.
    let schedule = official_meta::json_string_field(json, "schedule")
        .and_then(|s| official_meta::normalize_schedule(&s));
    let chapters = parse_episodes(json);
    OfficialMeta {
        source: "kakuyomu".to_string(),
        count: (!chapters.is_empty()).then_some(chapters.len() as i32),
        chapters,
        update_schedule: schedule,
        next_update_at: None,
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

/// Episodes = Apollo objects with `__typename == "Episode"` carrying
/// `title` + `publishedAt`. Stable-sorted by date (undated rows keep
/// document order and land last), numbered 1..N. Empty / malformed input
/// yields `Vec::new()`.
pub fn parse_episodes(json: &str) -> Vec<ChapterMeta> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut rows: Vec<(Option<String>, Option<chrono::NaiveDate>)> = Vec::new();
    collect_episodes(&value, &mut rows);
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

/// Depth-first walk collecting `Episode` objects (`title` + `publishedAt`).
fn collect_episodes(
    value: &serde_json::Value,
    out: &mut Vec<(Option<String>, Option<chrono::NaiveDate>)>,
) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("__typename").and_then(|v| v.as_str()) == Some("Episode")
                && map.contains_key("title")
                && map.contains_key("publishedAt")
            {
                let title = map
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::to_string);
                let release_date = map
                    .get("publishedAt")
                    .and_then(|v| v.as_str())
                    .and_then(parse_published_at);
                out.push((title, release_date));
            }
            for v in map.values() {
                collect_episodes(v, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_episodes(item, out);
            }
        }
        _ => {}
    }
}

/// `publishedAt` arrives as RFC3339 (`2020-06-17T13:13:19.000Z`); `parse_date`
/// wants a bare `YYYY-MM-DD`, so drop the time part first.
fn parse_published_at(raw: &str) -> Option<chrono::NaiveDate> {
    let head = raw.split(['T', ' ']).next().unwrap_or(raw);
    official_meta::parse_date(head)
}

/// GET `https://kakuyomu.jp/search?q=<title>` (public SSR page).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse("https://kakuyomu.jp/search")?;
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

/// Pure search parser: every `/works/<digits>` link whose neighbourhood holds
/// an `<h3>…</h3>` title. Dedups by id; malformed input degrades to empty.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("works/") {
        let seg = &rest[pos..];
        let id: String = seg["works/".len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        rest = &seg["works/".len()..];
        if id.is_empty() {
            continue;
        }
        // `<h3 …>` may carry attributes; keep only the inner text.
        let title = official_meta::between(seg, "<h3", "</h3>")
            .map(|s| match s.find('>') {
                Some(i) => &s[i + 1..],
                None => s,
            })
            .map(official_meta::strip_tags)
            .unwrap_or_default();
        if !title.is_empty() && !hits.iter().any(|h| h.id == id) {
            hits.push(OfficialHit { id, title });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_detail_fixture() {
        let html = include_str!("../../../tests/fixtures/kakuyomu_detail.html");
        let m = parse_detail(html);
        assert_eq!(m.source, "kakuyomu");
        // 503 `Episode` objects == the work's publicEpisodeCount; the 31
        // non-episode `publishedAt` fields must not leak in.
        assert_eq!(m.chapters.len(), 503);
        assert_eq!(m.count, Some(503));
        assert_eq!(m.chapters[0].number_x100, 100);
        assert!(m.chapters[0].title.is_some());
        assert!(m.chapters[0].release_date.is_some());
        // Oldest-first on the fixture: dates ascend.
        let dates: Vec<_> = m
            .chapters
            .iter()
            .filter_map(|c| c.release_date)
            .collect();
        let mut sorted = dates.clone();
        sorted.sort();
        assert_eq!(dates, sorted);
        // Work.schedule is null; the page has no schedule text.
        assert!(m.update_schedule.is_none());
        assert!(m.next_update_at.is_none());
    }

    #[test]
    fn empty_is_empty() {
        let m = parse_detail("");
        assert!(m.chapters.is_empty());
        assert!(m.update_schedule.is_none());
        assert!(m.next_update_at.is_none());
        assert!(m.count.is_none());
        assert!(parse_episodes("<html></html>").is_empty());
        assert!(parse_search("").is_empty());
    }

    #[test]
    fn parses_search_from_work_links() {
        let html = r#"<a href="/works/16817330663976696237"><h3 class="ellipsis">The Title</h3></a>
                      <a href="/works/16817330663976696237">dup</a>
                      <a href="/works/16817330663976699999"><h3>Second</h3></a>"#;
        let hits = parse_search(html);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "16817330663976696237");
        assert_eq!(hits[0].title, "The Title");
        assert_eq!(hits[1].id, "16817330663976699999");
        assert_eq!(hits[1].title, "Second");
    }
}
