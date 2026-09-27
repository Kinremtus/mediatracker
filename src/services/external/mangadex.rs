use crate::models::media_item::CreateMediaItem;
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

    /// Unified-search entry point: find manga by title and map each hit into a
    /// normalized `CreateMediaItem` (provider `mangadex`).
    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let url = format!(
            "https://api.mangadex.org/manga?title={}&limit=10&includes[]=cover_art&includes[]=author&includes[]=artist",
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
        Ok(data.data.iter().map(map_manga).collect())
    }

    /// Fetch a single manga by MangaDex UUID and map it into a normalized
    /// `CreateMediaItem`. The response is wrapped in `{"data": {...}}`.
    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!(
            "https://api.mangadex.org/manga/{}?includes[]=cover_art",
            urlencoding::encode(id)
        );

        let response = self.client.get(&url).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("MangaDex details failed: {} - {}", status, body);
        }

        let response_text = response.text().await?;
        let data: MangaDexSingleResponse = match serde_json::from_str(&response_text) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(url=%url, error=%e, response=%response_text, "MangaDex details JSON decode failed");
                anyhow::bail!("MangaDex details JSON decode failed: {}", e);
            }
        };
        Ok(map_manga(&data.data))
    }
}

/// Map a raw MangaDex manga entity into the normalized provider item.
///
/// `media_type` is always `"manga"` here; the unified search layer normalizes
/// it further once the series is inspected (manga/manhwa/manhua/...).
fn map_manga(r: &MangaDexSearchResult) -> CreateMediaItem {
    let id = r.id.clone();
    let attrs = &r.attributes;

    // Prefer the English title, otherwise fall back to any language variant.
    let title = attrs
        .title
        .get("en")
        .or_else(|| attrs.title.values().next())
        .cloned()
        .unwrap_or_default();

    let poster_url = r
        .relationships
        .iter()
        .find(|rel| rel.rel_type == "cover_art")
        .and_then(|rel| rel.attributes.as_ref())
        .and_then(|a| a.file_name.as_ref())
        .map(|file_name| {
            format!(
                "https://uploads.mangadex.org/covers/{}/{}.256.jpg",
                id, file_name
            )
        });

    let description = attrs
        .description
        .get("en")
        .or_else(|| attrs.description.values().next())
        .cloned();
    let description = crate::utils::clean_description(description);

    // MangaDex reports fractional chapters ("12.5"); truncate to a whole count
    // for the metadata field rather than dropping the value entirely.
    let chapters = attrs
        .last_chapter
        .as_deref()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|f| f as i32);
    let volumes = attrs
        .last_volume
        .as_deref()
        .and_then(|s| s.parse::<i32>().ok());
    let year = attrs.year.map(|y| y as i16);

    CreateMediaItem {
        provider: "mangadex".to_string(),
        external_id: id,
        media_type: "manga".to_string(),
        title: title.clone(),
        poster_url,
        description,
        status: attrs.status.clone(),
        chapters,
        volumes,
        year,
        comparison_key: Some(title),
        ..Default::default()
    }
}

#[derive(Debug, Deserialize)]
pub struct MangaDexSearchResponse {
    pub data: Vec<MangaDexSearchResult>,
}

/// `GET /manga/{id}` returns a single entity wrapped in `{"data": {...}}`.
#[derive(Debug, Deserialize)]
pub struct MangaDexSingleResponse {
    pub data: MangaDexSearchResult,
}

#[derive(Debug, Deserialize)]
pub struct MangaDexSearchResult {
    pub id: String,
    pub attributes: MangaDexMangaAttributes,
    /// `includes[]=cover_art` surfaces relationships; kept defaulted so the
    /// chapter-enrichment path (which omits `includes`) keeps deserializing.
    #[serde(default)]
    pub relationships: Vec<MangaDexRelationship>,
}

#[derive(Debug, Deserialize)]
pub struct MangaDexRelationship {
    #[serde(rename = "type")]
    pub rel_type: String,
    #[serde(default)]
    pub attributes: Option<MangaDexRelationshipAttributes>,
}

#[derive(Debug, Deserialize)]
pub struct MangaDexRelationshipAttributes {
    #[serde(rename = "fileName")]
    pub file_name: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MangaDexMangaAttributes {
    pub title: HashMap<String, String>,
    /// One language→title map per alternate title. Some works carry their exact
    /// English/romaji name only here (the primary `title` map may hold just
    /// `ko-ro`), so the title matcher must consider these too.
    #[serde(rename = "altTitles", default)]
    pub alt_titles: Vec<HashMap<String, String>>,
    /// Localized descriptions; used by the unified search mapper (English preferred).
    #[serde(default)]
    pub description: HashMap<String, String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(rename = "lastChapter", default)]
    pub last_chapter: Option<String>,
    #[serde(rename = "lastVolume", default)]
    pub last_volume: Option<String>,
}

impl MangaDexMangaAttributes {
    /// Every title variant MangaDex returns: the values of the primary `title`
    /// map plus every value of every entry in `altTitles`.
    pub fn title_candidates(&self) -> impl Iterator<Item = &str> {
        self.title.values().map(String::as_str).chain(
            self.alt_titles
                .iter()
                .flat_map(|entry| entry.values())
                .map(String::as_str),
        )
    }
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

    #[test]
    fn manga_attributes_parse_alt_titles_and_collect_candidates() {
        let json = r#"{
            "title": {"ko-ro": "Academy eseo Saranamgi"},
            "altTitles": [
                {"en": "The Extra's Academy Survival Guide"},
                {"ja": "Academy eseo Saranamgi"}
            ]
        }"#;
        let attrs: MangaDexMangaAttributes =
            serde_json::from_str(json).expect("altTitles must deserialize");
        assert_eq!(attrs.alt_titles.len(), 2);

        let candidates: Vec<&str> = attrs.title_candidates().collect();
        assert!(candidates.contains(&"Academy eseo Saranamgi"));
        assert!(candidates.contains(&"The Extra's Academy Survival Guide"));
    }

    #[test]
    fn manga_attributes_alt_titles_default_to_empty() {
        let attrs: MangaDexMangaAttributes =
            serde_json::from_str(r#"{"title":{"en":"Berserk"}}"#).expect("altTitles is optional");
        assert!(attrs.alt_titles.is_empty());
        assert_eq!(attrs.title_candidates().count(), 1);
    }

    #[test]
    fn map_manga_builds_item_from_search_hit() {
        let json = r#"{
            "id": "801513ba-a712-498c-8f57-cae55b38cc92",
            "type": "manga",
            "attributes": {
                "title": {"en": "Goodbye, Dragon Life"},
                "altTitles": [{"ja": "さようなら竜生、こんにちは人生"}],
                "description": {"en": "A long time ago, the dragon retired."},
                "status": "completed",
                "year": 2015,
                "lastChapter": "12.5",
                "lastVolume": "3"
            },
            "relationships": [
                {"id": "cover-id", "type": "cover_art", "attributes": {"fileName": "cover-file.jpg"}},
                {"id": "author-id", "type": "author", "attributes": {"name": "Someone"}}
            ]
        }"#;
        let result: MangaDexSearchResult =
            serde_json::from_str(json).expect("search hit must deserialize");
        let item = map_manga(&result);

        assert_eq!(item.provider, "mangadex");
        assert_eq!(item.external_id, "801513ba-a712-498c-8f57-cae55b38cc92");
        assert_eq!(item.media_type, "manga");
        assert_eq!(item.title, "Goodbye, Dragon Life");
        assert_eq!(
            item.poster_url.as_deref(),
            Some(
                "https://uploads.mangadex.org/covers/801513ba-a712-498c-8f57-cae55b38cc92/cover-file.jpg.256.jpg"
            )
        );
        assert_eq!(item.chapters, Some(12));
        assert_eq!(item.volumes, Some(3));
        assert_eq!(item.year, Some(2015));
        assert_eq!(item.status.as_deref(), Some("completed"));
        assert!(
            item.description
                .as_deref()
                .is_some_and(|d| d.contains("dragon"))
        );
        assert_eq!(item.comparison_key.as_deref(), Some("Goodbye, Dragon Life"));
    }

    #[test]
    fn single_response_maps_via_same_mapper() {
        let json = r#"{
            "result": "ok",
            "response": "entity",
            "data": {
                "id": "single-id",
                "type": "manga",
                "attributes": {
                    "title": {"en": "Solo Leveling"},
                    "status": "ongoing",
                    "year": 2018
                },
                "relationships": [
                    {"id": "cover-id", "type": "cover_art", "attributes": {"fileName": "solo.png"}}
                ]
            }
        }"#;
        let response: MangaDexSingleResponse =
            serde_json::from_str(json).expect("single response must deserialize");
        let item = map_manga(&response.data);

        assert_eq!(item.external_id, "single-id");
        assert_eq!(item.title, "Solo Leveling");
        assert_eq!(item.chapters, None);
        assert_eq!(item.volumes, None);
        assert_eq!(item.year, Some(2018));
        assert_eq!(
            item.poster_url.as_deref(),
            Some("https://uploads.mangadex.org/covers/single-id/solo.png.256.jpg")
        );
    }

    #[test]
    fn search_hit_without_relationships_falls_back_to_any_title() {
        let json = r#"{
            "id": "no-rel",
            "attributes": {
                "title": {"ja": "ワンピース"}
            }
        }"#;
        let result: MangaDexSearchResult =
            serde_json::from_str(json).expect("relationships must be optional");
        let item = map_manga(&result);

        assert_eq!(item.title, "ワンピース");
        assert_eq!(item.poster_url, None);
        assert_eq!(item.description, None);
        assert_eq!(item.status, None);
        assert_eq!(item.chapters, None);
    }
}
