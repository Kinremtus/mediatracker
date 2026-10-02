//! Naver Series (series.naver.com) chapter-count source.
//! Verified: GET /comic/detail.series?productNo=<id> ->
//!   <h5 class="end_total_episode"> 총 <strong>162</strong>화 </h5>
//! `titleId` is absent from both search and detail, so the article/list API is
//! unreachable for this app; the counter is taken from the detail page and
//! `source_id` is the `productNo`.

use super::{ChapterCount, ChapterCountFuture, ChapterCountProvider};

pub const DETAIL_BASE: &str = "https://series.naver.com/comic/detail.series";

/// Browser-like User-Agent; the endpoint rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub struct NaverProvider {
    client: reqwest::Client,
}

impl NaverProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for NaverProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ChapterCountProvider for NaverProvider {
    fn name(&self) -> &'static str {
        "naver"
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        Box::pin(async move {
            let mut url = url::Url::parse(DETAIL_BASE)?;
            url.query_pairs_mut().append_pair("productNo", external_id);
            let body = self
                .client
                .get(url)
                .header(reqwest::header::USER_AGENT, UA)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            Ok(parse_total(&body).map(|total| ChapterCount {
                total,
                source: "auto:naver",
            }))
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manhwa"]
    }
}

/// Pure parser: the number in `<strong>` right after the `end_total_episode`
/// marker. Missing marker / missing number / non-positive -> `None` (raise-only
/// no-op; the common `usable()` also rejects `<= 0`).
pub fn parse_total(html: &str) -> Option<i32> {
    let idx = html.find("end_total_episode")?;
    let tail = &html[idx..];
    let s = tail.find("<strong")?;
    let after_open = &tail[s..];
    let gt = after_open.find('>')?;
    let after = &after_open[gt + 1..];
    let e = after.find("</strong>")?;
    after[..e]
        .trim()
        .parse::<i32>()
        .ok()
        .filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::parse_total;

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/naver_detail.html");

    #[test]
    fn parses_fixture_count() {
        assert_eq!(parse_total(FIXTURE), Some(162));
    }

    #[test]
    fn missing_marker_is_none() {
        assert_eq!(parse_total("<html><body>no counter</body></html>"), None);
    }

    #[test]
    fn marker_without_strong_is_none() {
        assert_eq!(parse_total("end_total_episode"), None);
    }

    #[test]
    fn truncated_before_marker_is_none() {
        let cut = FIXTURE.find("end_total_episode").unwrap_or(FIXTURE.len());
        assert_eq!(parse_total(&FIXTURE[..cut]), None);
    }
}
