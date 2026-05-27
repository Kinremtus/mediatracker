use reqwest::Client;
use serde::Deserialize;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://api.mangaupdates.com/v1";

#[derive(Debug, Deserialize)]
struct MangaUpdatesSearchResponse {
    results: Vec<MangaUpdatesSearchResult>,
}

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
    url: Option<MangaUpdatesImageUrl>,
}

#[derive(Debug, Deserialize)]
struct MangaUpdatesImageUrl {
    original: Option<String>,
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
        self.search_by_type(query, &[]).await
    }

    pub async fn search_by_type(
        &self,
        query: &str,
        allowed_types: &[&str],
    ) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
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

        let body: MangaUpdatesSearchResponse = response.json().await?;
        let results = body.results;

        let items = results
            .into_iter()
            .filter_map(|r| {
                let series_type = r.record.series_type.as_deref().unwrap_or("");

                // If allowed_types is specified, filter by exact match (case-insensitive)
                if !allowed_types.is_empty() {
                    let type_lower = series_type.to_lowercase();
                    let matches = allowed_types.iter().any(|t| type_lower == t.to_lowercase());
                    if !matches {
                        return None;
                    }
                }

                let media_type = match series_type {
                    "Manga" => "manga".to_string(),
                    "Manhwa" => "manhwa".to_string(),
                    "Manhua" => "manhua".to_string(),
                    "Novel" => "novel".to_string(),
                    "OEL" | "Doujinshi" | "Filipino" | "Indonesian" | "Thai" | "Vietnamese" | "Malaysian" => "other-comics".to_string(),
                    _ => "manga".to_string(),
                };

                let poster_url = r.record.image
                    .and_then(|img| img.url)
                    .and_then(|u| u.original);

                Some(CreateMediaItem {
                    provider: "mangaupdates".to_string(),
                    external_id: r.record.series_id.to_string(),
                    media_type,
                    title: r.record.title,
                    title_english: None,
                    title_native: None,
                    title_russian: None,
                    poster_url,
                    episodes: None,
                    description: r.record.description,
                    status: None,
                    score: r.record.bayesian_rating,
                    is_tracked: false,
                    mal_id: None,
                })
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/series/{}", BASE_URL, id);
        let response = self.client.get(&url).send().await?;
        let r: MangaUpdatesRecord = response.json().await?;

        let poster_url = r.image
            .and_then(|img| img.url)
            .and_then(|u| u.original);

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
            poster_url,
            episodes: None,
            description: r.description,
            status: None,
            score: r.bayesian_rating,
            is_tracked: false,
            mal_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_response_wrapper() {
        let json = r#"{"total_hits":1,"page":1,"per_page":1,"results":[{"record":{"series_id":1,"title":"One Piece","type":"Manga","bayesian_rating":8.5}}]}"#;
        let body: MangaUpdatesSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(body.results.len(), 1);
        assert_eq!(body.results[0].record.title, "One Piece");
    }
}
