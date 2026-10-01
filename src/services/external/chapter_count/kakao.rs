//! Kakao Page (KR webtoons) chapter-count source.
//! Verified: GET bff-page.kakao.com/api/gateway/api/v2/content/product/list
//!   ?series_id=<id> -> {"result":{"total_count":157,
//!   "series_item":{"on_sale_count":157}},...}; works from a datacenter IP, no token.
//! The BFF requires accept/origin/referer AND a non-empty User-Agent; a
//! UA-less request is answered with HTTP 500 (not 403).

use super::{ChapterCount, ChapterCountFuture, ChapterCountProvider};

const BASE: &str = "https://bff-page.kakao.com/api/gateway/api/v2/content/product/list";

pub struct KakaoProvider {
    client: reqwest::Client,
}

impl KakaoProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for KakaoProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(serde::Deserialize)]
struct KakaoResponse {
    #[serde(default)]
    result: Option<KakaoResult>,
}

#[derive(serde::Deserialize)]
struct KakaoResult {
    #[serde(default)]
    total_count: Option<i64>,
    #[serde(default)]
    series_item: Option<KakaoSeriesItem>,
}

#[derive(serde::Deserialize)]
struct KakaoSeriesItem {
    #[serde(default)]
    on_sale_count: Option<i64>,
}

/// Chapter count of a series: `result.series_item.on_sale_count` (episodes on
/// sale) with a fallback to `result.total_count` (products in the list).
fn chapter_total(resp: &KakaoResponse) -> Option<i64> {
    let result = resp.result.as_ref()?;
    result
        .series_item
        .as_ref()
        .and_then(|s| s.on_sale_count)
        .or(result.total_count)
}

impl ChapterCountProvider for KakaoProvider {
    fn name(&self) -> &'static str {
        "kakao"
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        Box::pin(async move {
            let mut url = url::Url::parse(BASE)?;
            url.query_pairs_mut().append_pair("series_id", external_id);
            // The BFF rejects header-less requests with 403, and answers a
            // request without a User-Agent with HTTP 500 (reqwest sends none
            // by default). Send the same headers a browser would.
            let resp = self
                .client
                .get(url)
                .header("accept", "application/json, text/plain, */*")
                .header("origin", "https://page.kakao.com")
                .header("referer", "https://page.kakao.com/")
                .header(
                    "user-agent",
                    "mediatracker/1.0 (+https://github.com/Kinremtus/mediatracker)",
                )
                .send()
                .await?
                .error_for_status()?
                .json::<KakaoResponse>()
                .await?;
            Ok(chapter_total(&resp).and_then(|n| {
                i32::try_from(n)
                    .ok()
                    .filter(|v| *v > 0)
                    .map(|total| ChapterCount {
                        total,
                        source: "auto:kakao",
                    })
            }))
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manhwa", "manhua"]
    }
}

#[cfg(test)]
mod tests {
    use super::{chapter_total, KakaoResponse};

    fn parse(raw: &str) -> Option<i64> {
        chapter_total(&serde_json::from_str::<KakaoResponse>(raw).unwrap())
    }

    #[test]
    fn parses_nested_on_sale_count() {
        let raw = r#"{"result":{"total_count":157,"series_item":{"on_sale_count":157}},"result_code":0}"#;
        assert_eq!(parse(raw), Some(157));
    }

    #[test]
    fn falls_back_to_result_total_count() {
        let raw = r#"{"result":{"total_count":42}}"#;
        assert_eq!(parse(raw), Some(42));
    }

    #[test]
    fn tolerates_missing_fields() {
        assert_eq!(parse(r#"{"result":{}}"#), None);
        assert_eq!(parse(r#"{}"#), None);
    }
}
