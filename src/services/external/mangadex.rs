use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone)]
pub struct MangaDexService {
    client: Client,
}

impl Default for MangaDexService {
    fn default() -> Self {
        Self::new()
    }
}

impl MangaDexService {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("MediaTracker/1.0 (+https://github.com/Kinremtus/mediatracker)")
            .build()
            .unwrap_or_else(|_| Client::new());
        Self { client }
    }

    /// Search manga by title to find MangaDex UUID.
    pub async fn search_manga(&self, query: &str) -> Result<Vec<MangaDexSearchResult>> {
        let url = format!(
            "https://api.mangadex.org/manga?title={}&limit=10",
            urlencoding::encode(query)
        );

        let response = self.client.get(&url).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("MangaDex search failed: {} - {}", status, body);
        }

        let response_text = response.text().await?;
        let data: MangaDexSearchResponse = match serde_json::from_str(&response_text) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(url=%url, error=%e, response=%response_text, "MangaDex search JSON decode failed");
                anyhow::bail!("MangaDex search JSON decode failed: {}", e);
            }
        };
        Ok(data.data)
    }

    /// Get chapter list for a manga by MangaDex UUID.
    /// Uses the v2 API: /chapter?manga=<UUID>&translatedLanguage[]=en&translatedLanguage[]=ru
    ///
    /// MangaDex allows ~5 requests/second. Large series span several pages of
    /// 100, so we sleep between pages and retry a page up to 3 times on 429.
    pub async fn get_chapters(&self, manga_uuid: &str) -> Result<Vec<MangaDexChapter>> {
        const LIMIT: u32 = 100;
        const PAGE_DELAY: Duration = Duration::from_millis(250);
        const MAX_RETRIES: u32 = 3;

        let mut chapters = Vec::new();
        let mut offset = 0;

        loop {
            let url = format!(
                "https://api.mangadex.org/chapter?manga={}&limit={}&offset={}&order[chapter]=asc&translatedLanguage[]=en&translatedLanguage[]=ru",
                manga_uuid, LIMIT, offset
            );

            let mut attempt = 0u32;
            let response = loop {
                let resp = self.client.get(&url).send().await?;
                if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < MAX_RETRIES
                {
                    attempt += 1;
                    let backoff = Duration::from_millis(500 * u64::from(attempt));
                    tracing::warn!(
                        attempt,
                        backoff_ms = backoff.as_millis(),
                        "MangaDex 429, backing off"
                    );
                    tokio::time::sleep(backoff).await;
                    continue;
                }
                break resp;
            };

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!("MangaDex chapters failed: {} - {}", status, body);
            }

            let response_text = response.text().await?;
            let data: MangaDexChapterResponse = match serde_json::from_str(&response_text) {
                Ok(d) => d,
                Err(e) => {
                    tracing::error!(url=%url, error=%e, response=%response_text, "MangaDex JSON decode failed");
                    anyhow::bail!("MangaDex JSON decode failed: {}", e);
                }
            };
            if data.data.is_empty() {
                break;
            }

            for chapter in &data.data {
                chapters.push(chapter.attributes.clone());
            }

            if data.data.len() < LIMIT as usize {
                break;
            }
            offset += LIMIT;
            // Stay well under the ~5 rps limit across multi-page feeds.
            tokio::time::sleep(PAGE_DELAY).await;
        }

        Ok(chapters)
    }
}

#[derive(Debug, Deserialize)]
pub struct MangaDexSearchResponse {
    pub data: Vec<MangaDexSearchResult>,
}

#[derive(Debug, Deserialize)]
pub struct MangaDexSearchResult {
    pub id: String,
    pub attributes: MangaDexMangaAttributes,
}

#[derive(Debug, Deserialize)]
pub struct MangaDexMangaAttributes {
    pub title: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct MangaDexChapterResponse {
    pub result: String,
    pub response: String,
    pub data: Vec<MangaDexChapterWrapper>,
    pub limit: u32,
    pub offset: u32,
    pub total: u32,
}

#[derive(Debug, Deserialize)]
pub struct MangaDexChapterWrapper {
    pub id: String,
    #[serde(rename = "type")]
    pub chapter_type: String,
    pub attributes: MangaDexChapter,
    pub relationships: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MangaDexChapter {
    pub title: Option<String>,
    /// MangaDex returns `null` for chapters without a number (specials).
    /// Must be `Option` or a single such entry fails the whole page decode.
    #[serde(default)]
    pub chapter: Option<String>,
    pub volume: Option<String>,
    #[serde(rename = "translatedLanguage")]
    pub translated_language: String,
    #[serde(rename = "publishAt")]
    pub publish_at: Option<String>,
}

impl MangaDexChapter {
    /// Stored chapter number (scale x100): "10.5" -> 1050, "0.01" -> 1.
    /// Returns `None` when MangaDex sends `chapter: null` or an unparseable
    /// value, so the caller can skip the entry instead of failing the feed.
    ///
    /// NOTE: the `_10` suffix is historical; the stored scale is x100.
    pub fn chapter_number_10(&self) -> Option<i32> {
        crate::services::chapters::parse_chapter(self.chapter.as_deref()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chapter(number: Option<&str>) -> MangaDexChapter {
        MangaDexChapter {
            title: None,
            chapter: number.map(str::to_string),
            volume: None,
            translated_language: "en".to_string(),
            publish_at: None,
        }
    }

    /// Scale-agnostic: we compare against `parse_chapter` so this test holds
    /// regardless of whether the x100 scale change has landed in chapters.rs.
    #[test]
    fn chapter_number_uses_shared_parser() {
        use crate::services::chapters::parse_chapter;
        assert_eq!(chapter(Some("1")).chapter_number_10(), parse_chapter("1"));
        assert_eq!(
            chapter(Some("10.5")).chapter_number_10(),
            parse_chapter("10.5")
        );
        assert_eq!(chapter(None).chapter_number_10(), None);
        assert_eq!(chapter(Some("abc")).chapter_number_10(), None);
    }

    #[test]
    fn chapter_null_does_not_fail_deserialization() {
        let json = r#"{
            "result": "ok",
            "response": "collection",
            "data": [{
                "id": "abc",
                "type": "chapter",
                "attributes": {
                    "title": "Special",
                    "chapter": null,
                    "volume": "1",
                    "translatedLanguage": "en",
                    "publishAt": "2021-01-01T00:00:00+00:00"
                }
            }],
            "limit": 1,
            "offset": 0,
            "total": 1
        }"#;
        let response: MangaDexChapterResponse =
            serde_json::from_str(json).expect("chapter:null must deserialize");
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].attributes.chapter_number_10(), None);
    }
}
