use reqwest::Client;
use serde::Deserialize;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://shikimori.one/api";

#[derive(Debug, Deserialize)]
struct ShikimoriSearchResult {
    id: i64,
    name: String,
    name_en: Option<String>,
    image: Option<ShikimoriImage>,
    kind: Option<String>,
    score: Option<f64>,
    status: Option<String>,
    episodes: Option<i32>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ShikimoriImage {
    original: Option<String>,
}

fn poster_url(original: Option<String>) -> Option<String> {
    original.map(|url| {
        if url.starts_with("http") {
            url
        } else {
            format!("https://shikimori.one{}", url)
        }
    })
}

#[derive(Clone)]
pub struct ShikimoriService {
    client: Client,
}

impl ShikimoriService {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let url = format!("{}/animes?search={}", BASE_URL, query);
        let response = self.client.get(&url).send().await?;
        let results: Vec<ShikimoriSearchResult> = response.json().await?;

        let items = results
            .into_iter()
            .map(|r| CreateMediaItem {
                provider: "shikimori".to_string(),
                external_id: r.id.to_string(),
                media_type: match r.kind.as_deref() {
                    Some("anime") => "anime".to_string(),
                    Some("manga") => "manga".to_string(),
                    _ => "anime".to_string(),
                },
                title: r.name,
                title_english: r.name_en,
                title_native: None,
                title_russian: None,
                poster_url: poster_url(r.image.and_then(|img| img.original)),
                episodes: r.episodes,
                description: r.description,
                status: r.status,
                score: r.score,
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/animes/{}", BASE_URL, id);
        let response = self.client.get(&url).send().await?;
        let r: ShikimoriSearchResult = response.json().await?;

        Ok(CreateMediaItem {
            provider: "shikimori".to_string(),
            external_id: r.id.to_string(),
            media_type: match r.kind.as_deref() {
                Some("anime") => "anime".to_string(),
                Some("manga") => "manga".to_string(),
                _ => "anime".to_string(),
            },
            title: r.name,
            title_english: r.name_en,
            title_native: None,
            title_russian: None,
            poster_url: poster_url(r.image.and_then(|img| img.original)),
            episodes: r.episodes,
            description: r.description,
            status: r.status,
            score: r.score,
        })
    }
}
