use reqwest::Client;
use serde::Deserialize;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://api.mangaupdates.com/v1";

#[derive(Debug, Deserialize)]
struct MangaUpdatesSearchResult {
    record: MangaUpdatesRecord,
}

#[derive(Debug, Deserialize)]
struct MangaUpdatesRecord {
    series_id: i64,
    title: String,
    image: Option<MangaUpdatesImage>,
    #[serde(rename = "type")]
    series_type: Option<String>,
    description: Option<String>,
    bayesian_rating: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct MangaUpdatesImage {
    url: Option<String>,
}

#[derive(Clone)]
pub struct MangaUpdatesService {
    client: Client,
}

impl MangaUpdatesService {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let url = format!("{}/series/search", BASE_URL);
        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "search": query,
                "perpage": 20
            }))
            .send()
            .await?;

        let results: Vec<MangaUpdatesSearchResult> = response.json().await?;

        let items = results
            .into_iter()
            .map(|r| CreateMediaItem {
                provider: "mangaupdates".to_string(),
                external_id: r.record.series_id.to_string(),
                media_type: match r.record.series_type.as_deref() {
                    Some("Manga") => "manga".to_string(),
                    Some("Manhwa") => "manhwa".to_string(),
                    Some("Manhua") => "manhua".to_string(),
                    Some("Novel") => "novel".to_string(),
                    _ => "manga".to_string(),
                },
                title: r.record.title,
                title_english: None,
                title_native: None,
                title_russian: None,
                poster_url: r.record.image.and_then(|img| img.url),
                episodes: None,
                description: r.record.description,
                status: None,
                score: r.record.bayesian_rating,
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/series/{}", BASE_URL, id);
        let response = self.client.get(&url).send().await?;
        let r: MangaUpdatesRecord = response.json().await?;

        Ok(CreateMediaItem {
            provider: "mangaupdates".to_string(),
            external_id: r.series_id.to_string(),
            media_type: match r.series_type.as_deref() {
                Some("Manga") => "manga".to_string(),
                Some("Manhwa") => "manhwa".to_string(),
                Some("Manhua") => "manhua".to_string(),
                Some("Novel") => "novel".to_string(),
                _ => "manga".to_string(),
            },
            title: r.title,
            title_english: None,
            title_native: None,
            title_russian: None,
            poster_url: r.image.and_then(|img| img.url),
            episodes: None,
            description: r.description,
            status: None,
            score: r.bayesian_rating,
        })
    }
}
