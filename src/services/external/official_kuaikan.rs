//! Kuaikan (kkmh.com) official-source title search + chapter metadata.
//!
//! Used to verify that a title has an official Chinese publication before the
//! tracker marks it as officially licensed. All failures are non-fatal: the
//! caller treats an empty result (or an `Err`) as "no official match".
//!
//! Detail metadata comes from the mobile SSR page
//! (`https://m.kuaikanmanhua.com/mobile/<topic_id>/list/`): a Nuxt 2 payload
//! (`window.__NUXT__`) carrying the full oldest-first `comicList` with epoch-ms
//! dates. The `/v2/` JSON APIs sit behind an openresty bot wall (403) and are
//! not used. Chapter titles read `第N话 <own title>（上）/（下）`; numbering is
//! classified by the shared [`chapter_title`] helpers.

use crate::services::external::chapter_title::{
    EpisodeRow, classify_kuaikan_title, main_count, rows_to_chapters,
};
use crate::services::external::nuxt_payload;
use crate::services::external::official_meta::{self, OfficialMeta};
use crate::services::official_types::OfficialHit;

const KKKMH_SEARCH: &str = "https://search.kkmh.com/search/complex";
const KKKMH_MOBILE: &str = "https://m.kuaikanmanhua.com/mobile";

/// Browser-like User-Agent; the search endpoint rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Mobile User-Agent for the SSR list page (it serves the mobile layout).
const MOBILE_UA: &str = "Mozilla/5.0 (Linux; Android 13) AppleWebKit/537.36 \
                         (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36";

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "kuaikan".to_string(),
        ..OfficialMeta::default()
    }
}

/// GET https://search.kkmh.com/search/complex?q=<cn title>
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(KKKMH_SEARCH)?;
    url.query_pairs_mut().append_pair("q", title);

    let body = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_search(&body))
}

/// Pure parser, unit-testable without network.
///
/// Reads `data.topic.hit[]` where each hit has a `title` (string) and an `id`
/// that may be either a number or a string. Malformed JSON, a missing `data`
/// branch, or any non-array `hit` yields an empty vector - never a panic.
pub fn parse_search(body: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };

    let Some(hits) = value
        .get("data")
        .and_then(|data| data.get("topic"))
        .and_then(|topic| topic.get("hit"))
        .and_then(|hit| hit.as_array())
    else {
        return Vec::new();
    };

    hits.iter()
        .filter_map(|hit| {
            let title = hit.get("title")?.as_str()?.to_string();
            let id_value = hit.get("id")?;
            let id = id_value
                .as_str()
                .map(str::to_string)
                .or_else(|| id_value.as_number().map(|n| n.to_string()))?;
            Some(OfficialHit::new(id, title))
        })
        .collect()
}

/// GET the mobile topic list page and parse its Nuxt payload.
pub async fn fetch_meta(client: &reqwest::Client, topic_id: &str) -> anyhow::Result<OfficialMeta> {
    // The id lands in the URL path: only digits are ever valid (search returns
    // numeric topic ids), so a malformed binding cannot smuggle path segments.
    if !official_meta::is_digit_id(topic_id) {
        return Ok(empty_meta());
    }
    let url = format!("{KKKMH_MOBILE}/{topic_id}/list/");
    let response = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, MOBILE_UA)
        .send()
        .await?
        .error_for_status()?;
    // `content_length` is absent on chunked responses, which the shared
    // client's 20 s timeout still bounds.
    if response
        .content_length()
        .is_some_and(|len| len > official_meta::MAX_DETAIL_BYTES)
    {
        tracing::warn!(topic_id, "kuaikan: detail page too large, skipped");
        return Ok(empty_meta());
    }
    let html = response.text().await?;
    Ok(parse_detail(&html))
}

/// Pure parser: `window.__NUXT__` -> `data[0].comicList[]` -> chapter rows.
///
/// Rows are served oldest-first; numbering comes from the title (`第533话 …`),
/// never from the row position. `count` is the highest main base — a `（下）`
/// part of chapter 533 is not a 534th chapter. A broken payload degrades to an
/// empty meta (non-fatal for the resolve chain).
pub fn parse_detail(html: &str) -> OfficialMeta {
    let Some(payload) = nuxt_payload::extract(html) else {
        return empty_meta();
    };
    let Some(list) = payload
        .pointer("/data/0/comicList")
        .and_then(|v| v.as_array())
    else {
        return empty_meta();
    };
    let rows: Vec<EpisodeRow> = list
        .iter()
        .filter_map(|item| {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())?;
            let release_date = item
                .get("created_at")
                .and_then(|v| v.as_i64())
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|dt| dt.date_naive());
            Some(EpisodeRow {
                title: Some(title.to_string()),
                release_date,
                kind: classify_kuaikan_title(title),
            })
        })
        .collect();
    let chapters = rows_to_chapters(rows);
    let main = main_count(&chapters);
    if !chapters.is_empty() {
        tracing::debug!(
            source = "kuaikan",
            rows = chapters.len(),
            main,
            bonus = chapters.len() - main,
            "comicList parsed"
        );
    }
    OfficialMeta {
        source: "kuaikan".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        update_schedule: None,
        next_update_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/kuaikan_search.json");
    const DETAIL_FIXTURE: &str = include_str!("../../../tests/fixtures/kuaikan_detail.html");

    #[test]
    fn parses_fixture() {
        let hits = parse_search(FIXTURE);
        assert_eq!(hits.len(), 2);
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        let titles: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(ids, ["123", "456"]);
        assert_eq!(titles, ["我的英雄学院", "海贼王"]);
    }

    #[test]
    fn malformed_json_is_empty() {
        assert!(parse_search("{not json").is_empty());
    }

    #[test]
    fn missing_data_is_empty() {
        assert!(parse_search(r#"{"foo":1}"#).is_empty());
    }

    #[test]
    fn accepts_string_id() {
        let body = r#"{"data":{"topic":{"hit":[{"id":"abc","title":"T"}]}}}"#;
        let hits = parse_search(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "abc");
        assert_eq!(hits[0].title, "T");
    }

    #[test]
    fn detail_fixture_parses() {
        let m = parse_detail(DETAIL_FIXTURE);
        assert_eq!(m.source, "kuaikan");
        // The full list is served: 927 rows (923 numbered + 4 specials).
        assert_eq!(m.chapters.len(), 927);
        // Highest main base is 533: `（下）` parts collapse into their chapter.
        assert_eq!(m.count, Some(533));
        // Oldest-first: the first row is chapter 1 with its own title text.
        assert_eq!(m.chapters[0].number_x100, 100);
        assert!(!m.chapters[0].is_bonus);
        assert_eq!(m.chapters[0].title.as_deref(), Some("第1话 重生"));
        // `created_at` epoch ms becomes a real date.
        assert!(m.chapters[0].release_date.is_some());
        // 923 numbered rows minus 8 repeated keys -> 915 main; the 8 repeats
        // plus 4 unnumbered specials -> 12 bonus. (One repeat used to be a
        // missed mixed-bracket part marker: `47(上）`-style rows now pair up.)
        let main = m.chapters.iter().filter(|c| !c.is_bonus).count();
        let bonus = m.chapters.iter().filter(|c| c.is_bonus).count();
        assert_eq!((main, bonus), (915, 12));
        // Split parts stay tickable: `（上）/（下）` rows carry an `N-p` label.
        let parts = m.chapters.iter().filter(|c| c.label.is_some()).count();
        assert!(parts > 700, "expected split parts, got {parts}");
        assert_eq!(
            m.chapters[0].release_date,
            crate::services::external::official_meta::parse_date("2017-10-16")
        );
    }

    #[test]
    fn broken_payload_is_empty() {
        let m = parse_detail("<html>no nuxt here</html>");
        assert_eq!(m.source, "kuaikan");
        assert!(m.chapters.is_empty());
        assert!(m.count.is_none());
    }
}
