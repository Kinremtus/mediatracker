//! Syosetu (JP novels) chapter-count source. Novels expose chapters via the
//! public API; parse the total episode/chapter number. Fail-safe Ok(None).

use super::{ChapterCount, ChapterCountFuture, ChapterCountProvider};

const BASE: &str = "https://api.syosetu.com/novelapi/api/";

pub struct SyosetuProvider {
    client: reqwest::Client,
}

impl SyosetuProvider {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for SyosetuProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ChapterCountProvider for SyosetuProvider {
    fn name(&self) -> &'static str {
        "syosetu"
    }

    fn fetch<'a>(&'a self, external_id: &'a str) -> ChapterCountFuture<'a> {
        Box::pin(async move {
            // `out=json`, `of=ga` returns the general-all count field.
            let mut url = url::Url::parse(BASE)?;
            url.query_pairs_mut()
                .append_pair("ncode", external_id)
                .append_pair("out", "json")
                .append_pair("of", "ga");
            let text = self
                .client
                .get(url)
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            // Syosetu returns JSON when `out=json`; `general_all_no` is the count.
            let rows: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
            let total = rows
                .iter()
                .filter_map(|v| v.get("general_all_no").and_then(|n| n.as_i64()))
                .max();
            Ok(total.and_then(|n| {
                i32::try_from(n)
                    .ok()
                    .filter(|v| *v > 0)
                    .map(|total| ChapterCount {
                        total,
                        source: "auto:syosetu",
                    })
            }))
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["novel"]
    }
}

#[cfg(test)]
mod tests {
    /// Mirrors the `general_all_no` extraction used in `fetch`, without network.
    #[derive(serde::Deserialize)]
    struct Row {
        #[serde(default)]
        general_all_no: Option<i64>,
    }

    #[test]
    fn extracts_max_general_all_no_without_network() {
        let rows: Vec<Row> = serde_json::from_str(
            r#"[{"general_all_no": 7}, {"general_all_no": 42}, {"other": 1}]"#,
        )
        .unwrap();
        let total = rows.iter().filter_map(|r| r.general_all_no).max();
        assert_eq!(total, Some(42));
    }

    #[test]
    fn missing_general_all_no_yields_none() {
        let rows: Vec<Row> = serde_json::from_str(r#"[{"other": 1}]"#).unwrap();
        let total = rows.iter().filter_map(|r| r.general_all_no).max();
        assert_eq!(total, None);
    }
}
