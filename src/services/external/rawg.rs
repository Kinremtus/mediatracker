use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://api.rawg.io/api";

#[derive(Debug, Deserialize)]
struct RawgSearchResult {
    id: i64,
    name: String,
    background_image: Option<String>,
    rating: Option<f64>,
    description: Option<String>,
    released: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawgDetails {
    id: i64,
    name: String,
    background_image: Option<String>,
    rating: Option<f64>,
    description_raw: Option<String>,
    released: Option<String>,
    platforms: Option<Vec<RawgPlatform>>,
}

#[derive(Debug, Deserialize)]
struct RawgPlatform {
    platform: RawgPlatformInfo,
}

#[derive(Debug, Deserialize)]
struct RawgPlatformInfo {
    name: String,
}

#[derive(Clone)]
pub struct RawgService {
    client: Client,
    pub api_key: String,
}

impl RawgService {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/games", BASE_URL))?;
        url.query_pairs_mut()
            .append_pair("key", &self.api_key)
            .append_pair("search", query)
            .append_pair("page_size", "20");

        let response = self.client.get(url.as_str()).send().await?;

        let results: serde_json::Value = response.json().await?;
        let items = results["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                let id = r["id"].as_i64()?;
                let name = r["name"].as_str()?.to_string();
                let background_image = r["background_image"].as_str().map(String::from);
                let rating = r["rating"].as_f64();
                let description = r["description_raw"].as_str().map(String::from);

                Some(CreateMediaItem {
                    provider: "rawg".to_string(),
                    external_id: id.to_string(),
                    media_type: "game".to_string(),
                    title: name.clone(),
                    title_english: None,
                    title_native: None,
                    title_russian: None,
                    poster_url: background_image,
                    episodes: None,
                    description,
                    status: None,
                    score: rating,
                    is_tracked: false,
                    mal_id: None,
                    comparison_key: Some(name),
                })
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/games/{}", BASE_URL, id))?;
        url.query_pairs_mut().append_pair("key", &self.api_key);

        let response = self.client.get(url.as_str()).send().await?;

        let r: RawgDetails = response.json().await?;

        Ok(CreateMediaItem {
            provider: "rawg".to_string(),
            external_id: r.id.to_string(),
            media_type: "game".to_string(),
            title: r.name.clone(),
            title_english: None,
            title_native: None,
            title_russian: None,
            poster_url: r.background_image,
            episodes: None,
            description: r.description_raw,
            status: None,
            score: r.rating,
            is_tracked: false,
            mal_id: None,
            comparison_key: Some(r.name),
        })
    }
}
