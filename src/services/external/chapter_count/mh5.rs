//! mh5.app (CN manhua aggregator) chapter-count source. The site is plain
//! HTML (no Cloudflare challenge), so we fetch the title page and parse the
//! highest `第N话` / `第N話` marker. Fail-safe Ok(None) on any problem.

use super::{ChapterCount, ChapterCountFuture, ChapterCountProvider};

const BASE: &str = "https://mh5.app/";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub struct Mh5Provider {
    client: reqwest::Client,
}

impl Mh5Provider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for Mh5Provider {
    fn default() -> Self {
        Self::new()
    }
}

/// Highest `第N话`/`第N話` number in the page, if any.
fn max_chapter(html: &str) -> Option<i32> {
    use std::sync::OnceLock;
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"第(\d+)[话話]").expect("valid chapter regex"));
    re.captures_iter(html)
        .filter_map(|c| c.get(1))
        .filter_map(|m| m.as_str().parse::<i32>().ok())
        .max()
}

impl ChapterCountProvider for Mh5Provider {
    fn name(&self) -> &'static str {
        "mh5"
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        Box::pin(async move {
            let url = url::Url::parse(BASE)?.join(external_id)?;
            let html = self
                .client
                .get(url)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .header(reqwest::header::REFERER, BASE)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            Ok(max_chapter(&html)
                .filter(|v| *v > 0)
                .map(|total| ChapterCount {
                    total,
                    source: "auto:mh5",
                }))
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manhua"]
    }
}

#[cfg(test)]
mod tests {
    use super::max_chapter;
    use crate::services::external::chapter_count::ChapterCountProvider;

    #[test]
    fn picks_the_highest_chapter_marker() {
        let html = "<a>第1话</a><a>第2話</a><span>更新至:第570话 东海秘墟</span>";
        assert_eq!(max_chapter(html), Some(570));
    }

    #[test]
    fn ignores_non_chapter_numbers() {
        let html = "<div>2026年10月01日</div><p>第3話 标题</p>";
        assert_eq!(max_chapter(html), Some(3));
    }

    #[test]
    fn no_marker_yields_none() {
        assert_eq!(max_chapter("<html><body>no chapters here</body></html>"), None);
    }

    #[test]
    fn provider_metadata() {
        let p = super::Mh5Provider::new();
        assert_eq!(p.name(), "mh5");
        assert_eq!(p.media_types(), &["manhua"]);
    }
}
