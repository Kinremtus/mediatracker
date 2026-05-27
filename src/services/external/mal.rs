use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::media_item::CreateMediaItem;

/// MyAnimeList data via Jikan v4 (https://jikan.moe) — no API key required.
const BASE_URL: &str = "https://api.jikan.moe/v4";
const USER_AGENT: &str = "MediaTracker/0.1 (+https://github.com/Kinremtus/mediatracker)";
const SEARCH_LIMIT: u32 = 25;

#[derive(Debug, Deserialize)]
struct MalAnimeSearchResponse {
    data: Vec<MalAnime>,
}

#[derive(Debug, Deserialize)]
struct MalAnimeResponse {
    data: MalAnime,
}

#[derive(Debug, Deserialize)]
struct MalAnime {
    mal_id: i64,
    title: String,
    title_english: Option<String>,
    title_japanese: Option<String>,
    images: Option<MalImages>,
    episodes: Option<i32>,
    synopsis: Option<String>,
    score: Option<f64>,
    status: Option<String>,
    #[serde(rename = "type")]
    anime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MalImages {
    jpg: Option<MalImageSet>,
}

#[derive(Debug, Deserialize)]
struct MalImageSet {
    image_url: Option<String>,
}

fn poster_url(images: &Option<MalImages>) -> Option<String> {
    images
        .as_ref()
        .and_then(|i| i.jpg.as_ref())
        .and_then(|j| j.image_url.clone())
}

fn map_anime(r: MalAnime) -> CreateMediaItem {
    CreateMediaItem {
        provider: "mal".to_string(),
        external_id: r.mal_id.to_string(),
        media_type: "anime".to_string(),
        title: r.title,
        title_english: r.title_english,
        title_native: r.title_japanese,
        title_russian: None,
        poster_url: poster_url(&r.images),
        episodes: r.episodes,
        description: r.synopsis,
        status: r.status,
        score: r.score,
        is_tracked: false,
        mal_id: Some(r.mal_id),
    }
}

#[derive(Clone)]
pub struct MalService {
    client: Client,
}

impl MalService {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/anime", BASE_URL))?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", query);
            pairs.append_pair("limit", &SEARCH_LIMIT.to_string());
        }

        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("MAL/Jikan search failed: {}", response.status());
        }

        let body: MalAnimeSearchResponse = response.json().await?;
        Ok(body.data.into_iter().map(map_anime).collect())
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/anime/{id}/full", BASE_URL);
        let response = self.client.get(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("MAL/Jikan details failed: {}", response.status());
        }

        let body: MalAnimeResponse = response.json().await?;
        Ok(map_anime(body.data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_response() {
        let json = r#"{"data":[{"mal_id":20,"title":"Naruto","title_english":"Naruto","title_japanese":"ナルト","score":8.02,"episodes":220,"status":"Finished Airing","type":"TV"}]}"#;
        let body: MalAnimeSearchResponse = serde_json::from_str(json).unwrap();
        assert_eq!(body.data[0].mal_id, 20);
    }
}
