//! Kuaikan (kkmh.com) official-source title search.
//!
//! Used to verify that a title has an official Chinese publication before the
//! tracker marks it as officially licensed. All failures are non-fatal: the
//! caller treats an empty result (or an `Err`) as "no official match".

use crate::services::official_types::OfficialHit;

const KKKMH_SEARCH: &str = "https://search.kkmh.com/search/complex";

/// Browser-like User-Agent; the search endpoint rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

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
            Some(OfficialHit { id, title })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/kuaikan_search.json");

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
}
