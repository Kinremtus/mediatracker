//! Kakao Page (page.kakao.com) official-source title search.
//!
//! Used to verify that a title has an official Korean publication before the
//! tracker marks it as officially licensed. All failures are non-fatal: the
//! caller treats an empty result (or an `Err`) as "no official match".

use crate::services::official_types::OfficialHit;

pub const KAKAO_SEARCH_BASE: &str = "https://bff-page.kakao.com/api/gateway/api/v2/search/series";

/// Browser-like User-Agent; the search endpoint rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// GET https://bff-page.kakao.com/api/gateway/api/v2/search/series
///   ?keyword=<kr title>&page=0&size=10
///
/// No cookies: the BFF only needs the accept/origin/referer headers plus a
/// non-empty User-Agent.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(KAKAO_SEARCH_BASE)?;
    url.query_pairs_mut()
        .append_pair("keyword", title)
        .append_pair("page", "0")
        .append_pair("size", "10");
    let body = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ORIGIN, "https://page.kakao.com")
        .header(reqwest::header::REFERER, "https://page.kakao.com/")
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
/// Reads `result.list[]` where each entry has a `title` (string) and a
/// `series_id` that may be either a number or a string. Malformed JSON, a
/// missing `result` branch, or any non-array `list` yields an empty vector -
/// never a panic. Entries without a title or series_id are skipped.
pub fn parse_search(body: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };

    let Some(entries) = value
        .get("result")
        .and_then(|result| result.get("list"))
        .and_then(|list| list.as_array())
    else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let title = entry.get("title")?.as_str()?.to_string();
            let series_id = entry.get("series_id")?;
            let id = series_id
                .as_str()
                .map(str::to_string)
                .or_else(|| series_id.as_number().map(|n| n.to_string()))?;
            Some(OfficialHit { id, title })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/kakao_search.json");

    #[test]
    fn parses_fixture() {
        let hits = parse_search(FIXTURE);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "64096846");
        assert_eq!(hits[0].title, "용사파티");
    }

    #[test]
    fn absent_result_is_empty() {
        assert!(parse_search(r#"{"x":1}"#).is_empty());
    }

    #[test]
    fn empty_list_is_empty() {
        assert!(parse_search(r#"{"result":{"list":[]}}"#).is_empty());
    }

    #[test]
    fn accepts_string_series_id() {
        let body = r#"{"result":{"list":[{"series_id":"abc","title":"T"}]}}"#;
        let hits = parse_search(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "abc");
        assert_eq!(hits[0].title, "T");
    }
}
