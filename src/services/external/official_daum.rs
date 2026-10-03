//! Daum/Kakao Webtoon KR: decorator + episodes APIs (guest, Referer only).
//!
//! Fixture reality checks (documented deviations from the plan):
//! - `daum_episodes.json` is the capture **error body**
//!   `{"errors":[{"errorType":"GATEWAY_TOKEN_CHECK_FAILURE"}]}` — the live
//!   episodes endpoint demanded a token at capture time, so the fixture test
//!   asserts *degradation to an empty list*; the happy path is covered by the
//!   inline `{"data":{"episodes":[…]}}` test instead.
//! - `daum_search.json` is a valid `data.content[]` payload.
//!
//! Schedule: only the decorator's `onGoingStatus` free text is used; no value
//! is ever invented (`update_schedule` stays `None` when absent).

use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
/// The gateway rejects requests without this Referer.
const REFERER: &str = "https://webtoon.kakao.com";

/// Decorator (schedule) + paginated episode list for `content_id`.
pub async fn fetch_meta(client: &reqwest::Client, content_id: &str) -> anyhow::Result<OfficialMeta> {
    let decorator = format!(
        "https://gateway-kw.kakao.com/decorator/v1/decorator/contents/{content_id}"
    );
    let body = client
        .get(&decorator)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::REFERER, REFERER)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let update_schedule = official_meta::json_string_field(&body, "onGoingStatus")
        .and_then(|s| official_meta::normalize_schedule(&s));

    let mut chapters: Vec<ChapterMeta> = Vec::new();
    let mut offset = 0usize;
    // 30 rows per page; 20 pages cap (600 episodes) — stop on first empty page.
    for _ in 0..20 {
        let url = format!(
            "https://gateway-kw.kakao.com/episode/v1/views/content-home/contents/{content_id}\
             /episodes?sort=NO&offset={offset}&limit=30"
        );
        let json = client
            .get(&url)
            .header(reqwest::header::USER_AGENT, UA)
            .header(reqwest::header::REFERER, REFERER)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let page = parse_episodes(&json, offset);
        if page.is_empty() {
            break;
        }
        offset += page.len();
        chapters.extend(page);
    }

    Ok(OfficialMeta {
        source: "daum".to_string(),
        count: (!chapters.is_empty()).then_some(chapters.len() as i32),
        chapters,
        update_schedule,
        next_update_at: None,
    })
}

/// `data.episodes[]` each carry `title` and `serialStartDateTime` (fallback
/// `date`); the first 10 chars are the `YYYY-MM-DD` date. Numbering continues
/// from `base` so pagination keeps a global sequence. Total: malformed body
/// (including the capture error) -> `Vec::new()`.
pub fn parse_episodes(json: &str, base: usize) -> Vec<ChapterMeta> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(list) = value
        .pointer("/data/episodes")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    list.iter()
        .enumerate()
        .map(|(i, ep)| {
            let title = ep
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let release_date = ep
                .get("serialStartDateTime")
                .or_else(|| ep.get("date"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.get(..10))
                .and_then(official_meta::parse_date);
            ChapterMeta {
                number_x100: ((base + i + 1) * 100) as i32,
                title,
                release_date,
            }
        })
        .collect()
}

/// GET the guest search endpoint (URL-encoded by `Url`; no Origin/cookies).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse("https://gateway-kw.kakao.com/search/v2/content")?;
    url.query_pairs_mut()
        .append_pair("word", title)
        .append_pair("limit", "10")
        .append_pair("offset", "0");
    let json = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_search(&json))
}

/// `data.content[]` -> `{ id, title }` (`seoId` is informational only; ids
/// arrive as JSON numbers *or* strings). Malformed body -> empty vector.
pub fn parse_search(json: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(list) = value.pointer("/data/content").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for item in list {
        let id = match item.get("id") {
            Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => continue,
        };
        let Some(title) = item.get("title").and_then(|v| v.as_str()).map(str::trim) else {
            continue;
        };
        if title.is_empty() || hits.iter().any(|h| h.id == id) {
            continue;
        }
        hits.push(OfficialHit {
            id,
            title: title.to_string(),
        });
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episodes_fixture_degrades_to_empty() {
        // Documented: the capture stored the gateway's error body, not data.
        let json = include_str!("../../../tests/fixtures/daum_episodes.json");
        let rows = parse_episodes(json, 0);
        assert!(
            rows.is_empty(),
            "error body must degrade to no rows, got {}",
            rows.len()
        );
        assert!(parse_episodes("{}", 0).is_empty());
        assert!(parse_episodes("{not json", 0).is_empty());
    }

    #[test]
    fn parses_episodes_payload() {
        let json = r#"{"data":{"episodes":[
            {"title":"1화","serialStartDateTime":"2026-01-05 00:00:00"},
            {"title":"2화","date":"2026-01-12"}
        ]}}"#;
        let rows = parse_episodes(json, 30);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].number_x100, 3100);
        assert_eq!(rows[0].title.as_deref(), Some("1화"));
        assert_eq!(
            rows[0].release_date.map(|d| d.to_string()).as_deref(),
            Some("2026-01-05")
        );
        assert_eq!(rows[1].number_x100, 3200);
        assert_eq!(
            rows[1].release_date.map(|d| d.to_string()).as_deref(),
            Some("2026-01-12")
        );
    }

    #[test]
    fn decorator_without_ongoing_status_has_no_schedule() {
        // Real decorator bodies carry `status:"SELLING"` while the free-text
        // `onGoingStatus` is absent -> schedule must stay None (never invent).
        let body = r#"{"status":"SELLING","content":{"id":4620}}"#;
        assert!(official_meta::json_string_field(body, "onGoingStatus").is_none());
        let body = r#"{"status":"SELLING","onGoingStatus":""}"#;
        let schedule = official_meta::json_string_field(body, "onGoingStatus")
            .and_then(|s| official_meta::normalize_schedule(&s));
        assert!(schedule.is_none());
    }

    #[test]
    fn parses_search_fixture() {
        let json = include_str!("../../../tests/fixtures/daum_search.json");
        let hits = parse_search(json);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].id, "4620");
        assert_eq!(hits[0].title, "하백의 신부 1");
        assert!(hits.iter().all(|h| !h.id.is_empty() && !h.title.is_empty()));
        assert!(parse_search("{not json").is_empty());
    }
}
