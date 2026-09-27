use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://comicvine.gamespot.com/api";
const MIN_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Deserialize)]
struct ComicVineSearchResponse {
    #[serde(default)]
    results: Vec<ComicVineVolume>,
}

#[derive(Debug, Deserialize)]
struct ComicVineDetailResponse {
    results: ComicVineVolume,
}

#[derive(Debug, Deserialize)]
#[expect(dead_code)]
struct ComicVineVolume {
    id: i64,
    name: Option<String>,
    start_year: Option<String>,
    image: Option<ComicVineImage>,
    publisher: Option<ComicVinePublisher>,
    count_of_issues: Option<i32>,
    description: Option<String>,
    deck: Option<String>,
    site_detail_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ComicVineImage {
    medium_url: Option<String>,
    small_url: Option<String>,
    original_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ComicVinePublisher {
    name: Option<String>,
}

fn map_volume(v: ComicVineVolume) -> CreateMediaItem {
    let title = v.name.unwrap_or_else(|| format!("Comic #{}", v.id));
    let poster_url = v
        .image
        .and_then(|i| i.medium_url.or(i.small_url).or(i.original_url));
    let description = crate::utils::clean_description(v.deck.or(v.description));
    let year = v
        .start_year
        .and_then(|s| s.parse::<i16>().ok())
        .filter(|y| *y > 0);
    let publishers = v.publisher.and_then(|p| p.name).into_iter().collect();

    CreateMediaItem {
        provider: "comicvine".to_string(),
        external_id: v.id.to_string(),
        media_type: "comic".to_string(),
        title: title.clone(),
        comparison_key: Some(title),
        poster_url,
        description,
        chapters: v.count_of_issues,
        volumes: None,
        year,
        publishers,
        format_type: Some("Comic".to_string()),
        rating: None,
        ..Default::default()
    }
}

#[derive(Clone)]
pub struct ComicVineService {
    client: Client,
    api_key: String,
    last_request: Arc<Mutex<Instant>>,
}

impl ComicVineService {
    pub fn new(api_key: String) -> Self {
        Self {
            client: super::http_client(),
            api_key,
            last_request: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(10))),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Comic Vine allows roughly one request per second; serialize callers so
    /// bursts never trip the rate limit. Mirrors the IGDB token Mutex pattern.
    async fn throttle(&self) {
        let mut last = self.last_request.lock().await;
        let elapsed = last.elapsed();
        if elapsed < MIN_INTERVAL {
            tokio::time::sleep(MIN_INTERVAL - elapsed).await;
        }
        *last = Instant::now();
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        if !self.is_configured() {
            return Ok(Vec::new());
        }
        self.throttle().await;

        let mut url = url::Url::parse(&format!("{BASE_URL}/search/"))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("resources", "volume")
            .append_pair("query", query)
            .append_pair("format", "json");

        let resp = self.client.get(url).send().await?;
        let parsed: ComicVineSearchResponse = resp.json().await?;
        Ok(parsed.results.into_iter().map(map_volume).collect())
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        if !self.is_configured() {
            anyhow::bail!("Comic Vine API key not configured");
        }
        self.throttle().await;

        let mut url = url::Url::parse(&format!("{BASE_URL}/volume/4050-{id}/"))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("format", "json");

        let resp = self.client.get(url).send().await?;
        let parsed: ComicVineDetailResponse = resp.json().await?;
        Ok(map_volume(parsed.results))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_search_result() {
        let json = r#"{
            "results": [
                {
                    "id": 12345,
                    "name": "Saga",
                    "start_year": "2012",
                    "image": {
                        "medium_url": "https://example.com/medium.jpg",
                        "small_url": "https://example.com/small.jpg",
                        "original_url": "https://example.com/orig.jpg"
                    },
                    "publisher": {"name": "Image Comics"},
                    "count_of_issues": 54,
                    "description": "<p>Desc</p>",
                    "deck": "A space opera",
                    "site_detail_url": "https://comicvine.gamespot.com/saga/4050-12345/"
                }
            ]
        }"#;
        let resp: ComicVineSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.results.len(), 1);
        let item = map_volume(resp.results.into_iter().next().unwrap());
        assert_eq!(item.provider, "comicvine");
        assert_eq!(item.external_id, "12345");
        assert_eq!(item.media_type, "comic");
        assert_eq!(item.title, "Saga");
        assert_eq!(item.comparison_key.as_deref(), Some("Saga"));
        assert_eq!(item.chapters, Some(54));
        assert_eq!(
            item.poster_url.as_deref(),
            Some("https://example.com/medium.jpg")
        );
        assert_eq!(item.year, Some(2012));
        assert_eq!(item.publishers, vec!["Image Comics".to_string()]);
        assert_eq!(item.format_type.as_deref(), Some("Comic"));
    }

    #[test]
    fn maps_detail_result() {
        let json = r#"{
            "results": {
                "id": 999,
                "name": null,
                "start_year": "0",
                "image": {"small_url": "https://example.com/small.jpg"},
                "publisher": null,
                "count_of_issues": 12,
                "description": "Plain text",
                "deck": null,
                "site_detail_url": null
            }
        }"#;
        let resp: ComicVineDetailResponse = serde_json::from_str(json).unwrap();
        let item = map_volume(resp.results);
        assert_eq!(item.external_id, "999");
        assert_eq!(item.title, "Comic #999");
        assert_eq!(item.chapters, Some(12));
        assert_eq!(
            item.poster_url.as_deref(),
            Some("https://example.com/small.jpg")
        );
        assert_eq!(item.year, None);
        assert!(item.publishers.is_empty());
    }
}
