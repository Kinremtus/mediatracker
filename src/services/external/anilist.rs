use reqwest::Client;
use serde::Deserialize;

use crate::models::media_item::CreateMediaItem;

const API_URL: &str = "https://graphql.anilist.co";

const SEARCH_QUERY: &str = r#"query ($search: String) { Page(perPage: 10) { media(type: MANGA, search: $search) { id title { romaji english native } coverImage { large } chapters volumes description averageScore genres status startDate { year } format } } }"#;

const DETAILS_QUERY: &str = r#"query ($id: Int) { Media(id: $id) { id title { romaji english native } coverImage { large } chapters volumes description averageScore genres status startDate { year } format } }"#;

#[derive(Debug, Deserialize)]
struct AniListResponse<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct AniListPageData {
    #[serde(rename = "Page")]
    page: AniListPage,
}

#[derive(Debug, Deserialize)]
struct AniListPage {
    media: Vec<AniListMedia>,
}

#[derive(Debug, Deserialize)]
struct AniListMediaData {
    #[serde(rename = "Media")]
    media: AniListMedia,
}

#[derive(Debug, Deserialize)]
struct AniListMedia {
    id: i64,
    title: AniListTitle,
    #[serde(rename = "coverImage")]
    cover_image: Option<AniListCover>,
    chapters: Option<i32>,
    volumes: Option<i32>,
    description: Option<String>,
    #[serde(rename = "averageScore")]
    average_score: Option<f64>,
    genres: Option<Vec<String>>,
    status: Option<String>,
    #[serde(rename = "startDate")]
    start_date: Option<AniListDate>,
    format: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AniListTitle {
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AniListCover {
    large: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AniListDate {
    year: Option<i32>,
}

fn map_media(m: AniListMedia) -> CreateMediaItem {
    let title = m
        .title
        .romaji
        .clone()
        .or_else(|| m.title.english.clone())
        .or_else(|| m.title.native.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let comparison_key = m.title.english.clone().or_else(|| m.title.romaji.clone());

    let year = m
        .start_date
        .and_then(|d| d.year)
        .and_then(|y| i16::try_from(y).ok());

    let score = m.average_score.map(|s| s / 10.0);

    CreateMediaItem {
        provider: "anilist".to_string(),
        external_id: m.id.to_string(),
        media_type: "manga".to_string(),
        title,
        title_english: m.title.english.clone(),
        title_native: m.title.native.clone(),
        title_russian: None,
        poster_url: m.cover_image.and_then(|c| c.large),
        episodes: None,
        description: crate::utils::clean_description(m.description),
        status: m.status,
        score,
        is_tracked: false,
        mal_id: None,
        shikimori_id: None,
        comparison_key,
        format_type: m.format,
        details: None,
        chapters: m.chapters,
        volumes: m.volumes,
        pages: None,
        runtime_minutes: None,
        playtime_hours: None,
        year,
        aired_from: None,
        aired_to: None,
        premiered_season: None,
        premiered_year: year,
        broadcast: None,
        completed: None,
        licensed: None,
        source: None,
        duration: None,
        rating: None,
        rating_votes: None,
        authors: Vec::new(),
        artists: Vec::new(),
        studios: Vec::new(),
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers: Vec::new(),
        serialized_in: Vec::new(),
        networks: Vec::new(),
        platforms: Vec::new(),
        genres: m.genres.unwrap_or_default(),
        themes: Vec::new(),
        demographics: Vec::new(),
        categories: Vec::new(),
    }
}

#[derive(Clone)]
pub struct AniListService {
    client: Client,
}

impl Default for AniListService {
    fn default() -> Self {
        Self::new()
    }
}

impl AniListService {
    pub fn new() -> Self {
        Self {
            client: super::http_client(),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let body = serde_json::json!({
            "query": SEARCH_QUERY,
            "variables": { "search": query }
        });

        let resp = self.client.post(API_URL).json(&body).send().await?;
        let parsed: AniListResponse<AniListPageData> = resp.json().await?;
        Ok(parsed.data.page.media.into_iter().map(map_media).collect())
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let numeric_id = id.parse::<i64>()?;

        let body = serde_json::json!({
            "query": DETAILS_QUERY,
            "variables": { "id": numeric_id }
        });

        let resp = self.client.post(API_URL).json(&body).send().await?;
        let parsed: AniListResponse<AniListMediaData> = resp.json().await?;
        Ok(map_media(parsed.data.media))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_response() {
        let json = r#"{
            "data": {
                "Page": {
                    "media": [
                        {
                            "id": 30002,
                            "title": {
                                "romaji": "Berserk",
                                "english": "Berserk",
                                "native": "ベルセルク"
                            },
                            "coverImage": { "large": "https://s4.anilist.co/file/berserk.jpg" },
                            "chapters": 380,
                            "volumes": 42,
                            "description": "<b>Guts</b> is a lone mercenary.",
                            "averageScore": 92,
                            "genres": ["Action", "Adventure"],
                            "status": "RELEASING",
                            "startDate": { "year": 1989 },
                            "format": "MANGA"
                        }
                    ]
                }
            }
        }"#;

        let parsed: AniListResponse<AniListPageData> = serde_json::from_str(json).unwrap();
        let items: Vec<CreateMediaItem> =
            parsed.data.page.media.into_iter().map(map_media).collect();

        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.provider, "anilist");
        assert_eq!(item.external_id, "30002");
        assert_eq!(item.media_type, "manga");
        assert_eq!(item.title, "Berserk");
        assert_eq!(
            item.poster_url.as_deref(),
            Some("https://s4.anilist.co/file/berserk.jpg")
        );
        assert_eq!(item.chapters, Some(380));
        assert_eq!(item.volumes, Some(42));
        assert_eq!(item.score, Some(9.2));
        assert_eq!(item.year, Some(1989));
        assert_eq!(item.premiered_year, Some(1989));
        assert!(item.genres.contains(&"Action".to_string()));
    }

    #[test]
    fn parses_details_response() {
        let json = r#"{
            "data": {
                "Media": {
                    "id": 105778,
                    "title": {
                        "romaji": "Chainsaw Man",
                        "english": "Chainsaw Man",
                        "native": "チェンソーマン"
                    },
                    "coverImage": { "large": "https://s4.anilist.co/file/csm.jpg" },
                    "chapters": null,
                    "volumes": null,
                    "description": null,
                    "averageScore": 85,
                    "genres": null,
                    "status": "RELEASING",
                    "startDate": { "year": 2018 },
                    "format": "MANGA"
                }
            }
        }"#;

        let parsed: AniListResponse<AniListMediaData> = serde_json::from_str(json).unwrap();
        let item = map_media(parsed.data.media);

        assert_eq!(item.external_id, "105778");
        assert_eq!(item.title, "Chainsaw Man");
        assert_eq!(
            item.poster_url.as_deref(),
            Some("https://s4.anilist.co/file/csm.jpg")
        );
        assert_eq!(item.score, Some(8.5));
        assert_eq!(item.chapters, None);
        assert!(item.genres.is_empty());
    }
}
