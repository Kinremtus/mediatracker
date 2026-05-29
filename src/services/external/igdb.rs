use std::sync::Arc;
use std::time::{Duration, Instant};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;
use url::Url;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://api.igdb.com/v4";
const TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const COVER_BASE: &str = "https://images.igdb.com/igdb/image/upload/t_cover_big";

#[derive(Debug, Deserialize)]
struct TwitchTokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct IgdbGame {
    id: i64,
    name: String,
    summary: Option<String>,
    rating: Option<f64>,
    total_rating: Option<f64>,
    cover: Option<IgdbCover>,
    genres: Option<Vec<IgdbGenre>>,
    first_release_date: Option<i64>,
    platforms: Option<Vec<IgdbPlatform>>,
}

#[derive(Debug, Deserialize)]
struct IgdbCover {
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IgdbGenre {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IgdbPlatform {
    name: Option<String>,
}

#[derive(Clone)]
pub struct IgdbService {
    client: Client,
    pub client_id: String,
    client_secret: String,
    token: Arc<Mutex<String>>,
    token_expires_at: Arc<Mutex<Instant>>,
}

impl IgdbService {
    pub fn new(client_id: String, client_secret: String) -> Self {
        Self {
            client: Client::new(),
            client_id,
            client_secret,
            token: Arc::new(Mutex::new(String::new())),
            token_expires_at: Arc::new(Mutex::new(Instant::now())),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }

    async fn ensure_token(&self) -> Result<String, anyhow::Error> {
        {
            let (expires_at, token) = tokio::join!(
                self.token_expires_at.lock(),
                self.token.lock(),
            );
            if !token.is_empty() && Instant::now() < *expires_at {
                return Ok(token.clone());
            }
        }

        let mut url = Url::parse(TOKEN_URL)?;
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("client_secret", &self.client_secret)
            .append_pair("grant_type", "client_credentials");

        let resp = self
            .client
            .post(url.as_str())
            .send()
            .await?
            .json::<TwitchTokenResponse>()
            .await?;

        {
            let (mut expires_at, mut token) = tokio::join!(
                self.token_expires_at.lock(),
                self.token.lock(),
            );
            *token = resp.access_token.clone();
            *expires_at = Instant::now() + Duration::from_secs(resp.expires_in.saturating_sub(60));
        }

        Ok(resp.access_token)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        if !self.is_configured() {
            return Ok(Vec::new());
        }

        let token = self.ensure_token().await?;
        let body = format!(
            r#"search "{query}"; fields name,cover.url,summary,total_rating,first_release_date,genres.name; limit 20;"#
        );

        let resp = self
            .client
            .post(format!("{}/games", BASE_URL))
            .header("Client-ID", &self.client_id)
            .header("Authorization", format!("Bearer {}", token))
            .body(body)
            .send()
            .await?;

        let games: Vec<IgdbGame> = resp.json().await?;

        let items = games
            .into_iter()
            .map(|g| {
                let poster_url = g.cover.and_then(|c| c.url).map(|url| {
                    if url.starts_with("//") {
                        format!("https:{}", url)
                    } else if url.starts_with("http") {
                        url
                    } else {
                        format!("{}/{}", COVER_BASE, url)
                    }
                });

                let score = g.total_rating.or(g.rating).map(|r| r / 10.0);

                CreateMediaItem {
                    provider: "igdb".to_string(),
                    external_id: g.id.to_string(),
                    media_type: "game".to_string(),
                    title: g.name.clone(),
                    title_english: None,
                    title_native: None,
                    title_russian: None,
                    poster_url,
                    episodes: None,
                    description: g.summary,
                    status: None,
                    score,
                    is_tracked: false,
                    mal_id: None,
                    comparison_key: Some(g.name),
                }
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let token = self.ensure_token().await?;
        let body = format!(
            r#"fields name,cover.url,summary,total_rating,genres.name,first_release_date,platforms.name,involved_companies.company.name; where id = {id};"#
        );

        let resp = self
            .client
            .post(format!("{}/games", BASE_URL))
            .header("Client-ID", &self.client_id)
            .header("Authorization", format!("Bearer {}", token))
            .body(body)
            .send()
            .await?;

        let games: Vec<IgdbGame> = resp.json().await?;

        let g = games
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Game not found"))?;

        let poster_url = g.cover.and_then(|c| c.url).map(|url| {
            if url.starts_with("//") {
                format!("https:{}", url)
            } else if url.starts_with("http") {
                url
            } else {
                format!("{}/{}", COVER_BASE, url)
            }
        });

        let score = g.total_rating.or(g.rating).map(|r| r / 10.0);

        Ok(CreateMediaItem {
            provider: "igdb".to_string(),
            external_id: g.id.to_string(),
            media_type: "game".to_string(),
            title: g.name.clone(),
            title_english: None,
            title_native: None,
            title_russian: None,
            poster_url,
            episodes: None,
            description: g.summary,
            status: None,
            score,
            is_tracked: false,
            mal_id: None,
            comparison_key: Some(g.name),
        })
    }
}
