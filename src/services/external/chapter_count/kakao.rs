//! Kakao Page (KR webtoons) chapter-count source.
//! Verified: GET bff-page.kakao.com/api/gateway/api/v2/content/product/list
//!   ?series_id=<id> -> {"total_count":157,...}; works from a datacenter IP, no token.

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
struct KakaoList {
    #[serde(default)]
    total_count: Option<i64>,
}

impl ChapterCountProvider for KakaoProvider {
    fn name(&self) -> &'static str {
        "kakao"
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        Box::pin(async move {
            let mut url = url::Url::parse(BASE)?;
            url.query_pairs_mut().append_pair("series_id", external_id);
            let resp = self
                .client
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .json::<KakaoList>()
                .await?;
            Ok(resp.total_count.and_then(|n| {
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
    use super::KakaoList;

    #[test]
    fn parses_total_count_and_tolerates_missing() {
        let j: KakaoList = serde_json::from_str(r#"{"total_count":157}"#).unwrap();
        assert_eq!(j.total_count, Some(157));
        let j: KakaoList = serde_json::from_str(r#"{"items":[]}"#).unwrap();
        assert_eq!(j.total_count, None);
    }
}
