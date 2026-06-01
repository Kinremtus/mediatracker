use std::sync::Arc;
use std::time::{Duration, Instant};
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::Mutex;

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

#[derive(Debug, Deserialize)]
struct IgdbInvolvedCompany {
    developer: Option<bool>,
    publisher: Option<bool>,
    company: IgdbCompany,
}

#[derive(Debug, Deserialize)]
struct IgdbCompany {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct IgdbGame {
    id: i64,
    name: String,
    summary: Option<String>,
    rating: Option<f64>,
    total_rating: Option<f64>,
    total_rating_count: Option<i32>,
    cover: Option<IgdbCover>,
    genres: Option<Vec<IgdbGenre>>,
    platforms: Option<Vec<IgdbPlatform>>,
    first_release_date: Option<i64>,
    involved_companies: Option<Vec<IgdbInvolvedCompany>>,
    age_ratings: Option<Vec<IgdbAgeRating>>,
    game_modes: Option<Vec<IgdbName>>,
    themes: Option<Vec<IgdbName>>,
}

#[derive(Debug, Deserialize)]
struct IgdbAgeRating {
    rating: Option<IgdbName>,
}

#[derive(Debug, Deserialize)]
struct IgdbName {
    name: Option<String>,
}

fn cover_url(url: Option<String>) -> Option<String> {
    url.map(|url| {
        if url.starts_with("//") {
            format!("https:{}", url)
        } else if url.starts_with("http") {
            url
        } else {
            format!("{}/{}", COVER_BASE, url)
        }
    })
}

fn extract_genre_names(items: &Option<Vec<IgdbGenre>>) -> Vec<String> {
    items
        .as_ref()
        .map(|v| v.iter().filter_map(|g| g.name.clone()).collect())
        .unwrap_or_default()
}

fn extract_platform_names(items: &Option<Vec<IgdbPlatform>>) -> Vec<String> {
    items
        .as_ref()
        .map(|v| v.iter().filter_map(|p| p.name.clone()).collect())
        .unwrap_or_default()
}

fn extract_opt_names(items: &Option<Vec<IgdbName>>) -> Vec<String> {
    items
        .as_ref()
        .map(|v| v.iter().filter_map(|n| n.name.clone()).collect())
        .unwrap_or_default()
}

fn first_release_year(unix_ts: Option<i64>) -> Option<i16> {
    let ts = unix_ts?;
    let secs = i64::try_from(ts).ok()?;
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| i16::try_from(dt.format("%Y").to_string().parse::<i32>().unwrap_or(0)).ok())
        .flatten()
}

fn first_release_date(unix_ts: Option<i64>) -> Option<chrono::NaiveDate> {
    let ts = unix_ts?;
    let secs = i64::try_from(ts).ok()?;
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.date_naive())
}

fn map_game(g: IgdbGame) -> CreateMediaItem {
    let poster_url = cover_url(g.cover.and_then(|c| c.url));
    let score = g.total_rating.or(g.rating).map(|r| r / 10.0);
    let year = first_release_year(g.first_release_date);
    let aired_from = first_release_date(g.first_release_date);

    let (mut authors, mut publishers) = (Vec::new(), Vec::new());
    if let Some(involved) = g.involved_companies.as_ref() {
        for ic in involved {
            if let Some(name) = ic.company.name.clone() {
                if ic.publisher.unwrap_or(false) {
                    publishers.push(name);
                } else if ic.developer.unwrap_or(false) {
                    authors.push(name);
                } else {
                    authors.push(name);
                }
            }
        }
    }

    let themes = extract_opt_names(&g.themes);
    let genres = extract_genre_names(&g.genres);
    let platforms = extract_platform_names(&g.platforms);

    // Age rating — IGDB хранит в категориях (ESRB, PEGI, CERO). Берём первую name, если есть.
    let rating = g
        .age_ratings
        .as_ref()
        .and_then(|ars| ars.iter().filter_map(|a| a.rating.as_ref()?.name.clone()).next());

    let mut details = serde_json::Map::new();
    if let Some(c) = g.total_rating_count {
        details.insert("total_rating_count".to_string(), serde_json::Value::Number(c.into()));
    }
    let game_modes = extract_opt_names(&g.game_modes);
    if !game_modes.is_empty() {
        details.insert("game_modes".to_string(), serde_json::Value::Array(
            game_modes.into_iter().map(serde_json::Value::String).collect()
        ));
    }

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
        format_type: Some("Game".to_string()),
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
        chapters: None,
        volumes: None,
        pages: None,
        runtime_minutes: None,
        playtime_hours: None,
        year,
        aired_from,
        aired_to: None,
        premiered_season: None,
        premiered_year: year,
        broadcast: None,
        completed: None,
        licensed: None,
        source: None,
        duration: None,
        rating,
        rating_votes: g.total_rating_count,
        authors,
        artists: Vec::new(),
        studios: Vec::new(),
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers,
        serialized_in: Vec::new(),
        networks: Vec::new(),
        platforms,
        genres,
        themes,
        demographics: Vec::new(),
        categories: Vec::new(),
    }
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

        let mut url = url::Url::parse(TOKEN_URL)?;
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
            r#"search "{query}"; fields name,cover.url,summary,total_rating,first_release_date,genres.name,platforms.name,total_rating_count; limit 20;"#
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
        Ok(games.into_iter().map(map_game).collect())
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let token = self.ensure_token().await?;
        let body = format!(
            r#"fields name,cover.url,summary,total_rating,genres.name,first_release_date,platforms.name,involved_companies.company.name,involved_companies.developer,involved_companies.publisher,age_ratings.rating.name,total_rating_count,game_modes.name,themes.name; where id = {id};"#
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
        Ok(map_game(g))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_game_details() {
        let json = r#"{
            "id": 1942,
            "name": "The Witcher 3: Wild Hunt",
            "summary": "...",
            "total_rating": 92.5,
            "total_rating_count": 4000,
            "cover": {"url": "//images.igdb.com/...jpg"},
            "first_release_date": 1432080000,
            "genres": [{"name": "Role-playing (RPG)"}, {"name": "Adventure"}],
            "platforms": [{"name": "PC"}, {"name": "PlayStation 4"}],
            "involved_companies": [
                {"company": {"name": "CD Projekt Red"}, "developer": true},
                {"company": {"name": "CD Projekt"}, "publisher": true}
            ],
            "age_ratings": [{"rating": {"name": "M"}}]
        }"#;
        let g: IgdbGame = serde_json::from_str(json).unwrap();
        let item = map_game(g);
        assert_eq!(item.title, "The Witcher 3: Wild Hunt");
        assert_eq!(item.format_type.as_deref(), Some("Game"));
        assert!(item.genres.contains(&"Role-playing (RPG)".to_string()));
        assert!(item.platforms.contains(&"PC".to_string()));
        assert!(item.authors.contains(&"CD Projekt Red".to_string()));
        assert!(item.publishers.contains(&"CD Projekt".to_string()));
        assert_eq!(item.rating.as_deref(), Some("M"));
        assert!(item.score.is_some());
    }
}
