use anyhow::Result;
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Clone)]
pub struct MangaDexService {
    client: Client,
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
    pub async fn get_chapters(
        &self,
        manga_uuid: &str,
    ) -> Result<Vec<MangaDexChapter>> {
        let mut chapters = Vec::new();
        let mut offset = 0;
        const LIMIT: u32 = 100;

        loop {
            let url = format!(
                "https://api.mangadex.org/chapter?manga={}&limit={}&offset={}&order[chapter]=asc&translatedLanguage[]=en&translatedLanguage[]=ru",
                manga_uuid, LIMIT, offset
            );

            let response = self.client.get(&url).send().await?;
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
    pub chapter: String,
    pub volume: Option<String>,
    pub translated_language: String,
    pub publish_at: Option<String>,
}

impl MangaDexChapter {
    pub fn chapter_number_10(&self) -> Option<i32> {
        crate::services::chapters::parse_chapter(&self.chapter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chapter_number_from_mangadex() {
        let ch = MangaDexChapter {
            title: None,
            chapter: "1".to_string(),
            volume: None,
            translated_language: "en".to_string(),
            publish_at: None,
        };
        assert_eq!(ch.chapter_number_10(), Some(10));

        let ch = MangaDexChapter {
            title: None,
            chapter: "1.5".to_string(),
            volume: None,
            translated_language: "en".to_string(),
            publish_at: None,
        };
        assert_eq!(ch.chapter_number_10(), Some(15));

        let ch = MangaDexChapter {
            title: None,
            chapter: "10.5".to_string(),
            volume: None,
            translated_language: "en".to_string(),
            publish_at: None,
        };
        assert_eq!(ch.chapter_number_10(), Some(105));
    }
}