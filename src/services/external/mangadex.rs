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
        Self {
            client: Client::new(),
        }
    }

    /// Get chapter list for a manga by MangaDex ID.
    /// Returns (title_en, title_ru, chapter_number, volume) for each chapter.
    pub async fn get_chapters(
        &self,
        manga_id: &str,
    ) -> Result<Vec<MangaDexChapter>> {
        let mut chapters = Vec::new();
        let mut offset = 0;
        const LIMIT: u32 = 100;

        loop {
            let url = format!(
                "https://api.mangadex.org/manga/{}/feed?limit={}&offset={}&order[chapter]=asc&translatedLanguage[]=en&translatedLanguage[]=ru",
                manga_id, LIMIT, offset
            );

            let response = self.client.get(&url).send().await?;
            if !response.status().is_success() {
                anyhow::bail!("MangaDex chapters failed: {}", response.status());
            }

            let data: MangaDexFeedResponse = response.json().await?;
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

    /// Search manga by title to find MangaDex ID.
    pub async fn search_manga(&self, query: &str) -> Result<Vec<MangaDexSearchResult>> {
        let url = format!(
            "https://api.mangadex.org/manga?title={}&limit=10",
            urlencoding::encode(query)
        );

        let response = self.client.get(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("MangaDex search failed: {}", response.status());
        }

        let data: MangaDexSearchResponse = response.json().await?;
        Ok(data.data)
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
pub struct MangaDexFeedResponse {
    pub data: Vec<MangaDexChapterWrapper>,
}

#[derive(Debug, Deserialize)]
pub struct MangaDexChapterWrapper {
    pub id: String,
    pub attributes: MangaDexChapter,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MangaDexChapter {
    pub title: Option<String>,
    pub chapter: String,
    pub volume: Option<String>,
    pub translated_language: String,
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
        // MangaDex returns chapter as string, e.g. "1", "1.5", "10", "100"
        let ch = MangaDexChapter {
            title: None,
            chapter: "1".to_string(),
            volume: None,
            translated_language: "en".to_string(),
        };
        assert_eq!(ch.chapter_number_10(), Some(10));

        let ch = MangaDexChapter {
            title: None,
            chapter: "1.5".to_string(),
            volume: None,
            translated_language: "en".to_string(),
        };
        assert_eq!(ch.chapter_number_10(), Some(15));

        let ch = MangaDexChapter {
            title: None,
            chapter: "10.5".to_string(),
            volume: None,
            translated_language: "en".to_string(),
        };
        assert_eq!(ch.chapter_number_10(), Some(105));
    }
}