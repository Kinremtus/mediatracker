//! Kuaikan Comics (快看漫画) chapter-count source — the ORIGINAL CN platform
//! for CN manhua. Topic page is SSR HTML (Nuxt `window.__NUXT__` payload with
//! the full chapter list), so we parse the highest `第N话` / `第N話` marker.
//! Fail-safe Ok(None) on any problem.

use super::{ChapterCount, ChapterCountFuture, ChapterCountProvider};

const BASE: &str = "https://www.kuaikanmanhua.com/web/topic/";

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub struct KuaikanProvider {
    client: reqwest::Client,
}

impl KuaikanProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for KuaikanProvider {
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

impl ChapterCountProvider for KuaikanProvider {
    fn name(&self) -> &'static str {
        "kuaikan"
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        Box::pin(async move {
            let url = url::Url::parse(BASE)?.join(&format!("{external_id}/"))?;
            let html = self
                .client
                .get(url)
                .header(reqwest::header::USER_AGENT, USER_AGENT)
                .header(reqwest::header::REFERER, "https://www.kuaikanmanhua.com/")
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            Ok(max_chapter(&html)
                .filter(|v| *v > 0)
                .map(|total| ChapterCount {
                    total,
                    source: "auto:kuaikan",
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
    fn picks_the_highest_chapter_marker_from_nuxt_payload() {
        // Realistic fragment: chapter list in `window.__NUXT__` JSON + noise
        // numbers (ids, dates) that must not be mistaken for chapters.
        let html = r#"<script>window.__NUXT__=(function(){return{topicId:10698, \
            episodes:[{"id":743,"title":"第1话 起点"},{"id":811,"title":"第569话 试炼"}, \
            {"id":899,"title":"第570话 东海秘墟"}]}})()</script>
            <div>更新时间 2026年09月30日</div><span>第2話</span>"#;
        assert_eq!(max_chapter(html), Some(570));
    }

    #[test]
    fn ignores_non_chapter_numbers() {
        let html = "<div>2026年10月01日 topicId:10698</div><p>第3話 标题</p>";
        assert_eq!(max_chapter(html), Some(3));
    }

    #[test]
    fn no_marker_yields_none() {
        assert_eq!(
            max_chapter("<html><body>no chapters here</body></html>"),
            None
        );
    }

    #[test]
    fn provider_metadata() {
        let p = super::KuaikanProvider::new();
        assert_eq!(p.name(), "kuaikan");
        assert_eq!(p.media_types(), &["manhua"]);
    }
}
