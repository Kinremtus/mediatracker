use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://api.rawg.io/api";

#[derive(Debug, Deserialize)]
struct RawgName {
    name: String,
}

#[derive(Debug, Deserialize)]
struct RawgEsrb {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawgPlatformWrap {
    platform: RawgName,
}

#[derive(Debug, Deserialize)]
#[expect(dead_code)]
struct RawgSearchResult {
    id: i64,
    name: String,
    background_image: Option<String>,
    rating: Option<f64>,
    description: Option<String>,
    released: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct RawgDetails {
    id: i64,
    name: String,
    background_image: Option<String>,
    rating: Option<f64>,
    ratings_count: Option<i32>,
    metacritic: Option<i32>,
    description_raw: Option<String>,
    released: Option<String>,
    playtime: Option<i32>,
    status: Option<String>,
    esrb_rating: Option<RawgEsrb>,
    #[serde(default)]
    genres: Vec<RawgName>,
    #[serde(default)]
    platforms: Vec<RawgPlatformWrap>,
    #[serde(default)]
    developers: Vec<RawgName>,
    #[serde(default)]
    publishers: Vec<RawgName>,
}

fn parse_date(s: Option<&str>) -> Option<chrono::NaiveDate> {
    s.and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

fn extract_names(items: &[RawgName]) -> Vec<String> {
    items.iter().map(|n| n.name.clone()).collect()
}

fn extract_platform_names(items: &[RawgPlatformWrap]) -> Vec<String> {
    items.iter().map(|p| p.platform.name.clone()).collect()
}

fn map_details(r: RawgDetails) -> CreateMediaItem {
    let year = parse_date(r.released.as_deref())
        .and_then(|d| i16::try_from(d.format("%Y").to_string().parse::<i32>().unwrap_or(0)).ok());

    let mut details = serde_json::Map::new();
    if let Some(m) = r.metacritic {
        details.insert("metacritic".to_string(), serde_json::Value::Number(m.into()));
    }

    CreateMediaItem {
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
        status: r.status,
        score: r.rating,
        is_tracked: false,
        mal_id: None,
        shikimori_id: None,
        comparison_key: Some(r.name),
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
        playtime_hours: r.playtime,
        year,
        aired_from: parse_date(r.released.as_deref()),
        aired_to: None,
        premiered_season: None,
        premiered_year: year,
        broadcast: None,
        completed: None,
        licensed: None,
        source: None,
        duration: None,
        rating: r.esrb_rating.and_then(|e| e.name),
        rating_votes: r.ratings_count,
        authors: extract_names(&r.developers),
        artists: Vec::new(),
        studios: Vec::new(),
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers: extract_names(&r.publishers),
        serialized_in: Vec::new(),
        networks: Vec::new(),
        platforms: extract_platform_names(&r.platforms),
        genres: extract_names(&r.genres),
        themes: Vec::new(),
        demographics: Vec::new(),
        categories: Vec::new(),
    }
}

fn map_search(r: &RawgSearchResult) -> CreateMediaItem {
    CreateMediaItem {
        provider: "rawg".to_string(),
        external_id: r.id.to_string(),
        media_type: "game".to_string(),
        title: r.name.clone(),
        title_english: None,
        title_native: None,
        title_russian: None,
        poster_url: r.background_image.clone(),
        episodes: None,
        description: r.description.clone(),
        status: None,
        score: r.rating,
        is_tracked: false,
        mal_id: None,
        shikimori_id: None,
        comparison_key: Some(r.name.clone()),
        format_type: Some("Game".to_string()),
        details: None,
        chapters: None,
        volumes: None,
        pages: None,
        runtime_minutes: None,
        playtime_hours: None,
        year: None,
        aired_from: None,
        aired_to: None,
        premiered_season: None,
        premiered_year: None,
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
        genres: Vec::new(),
        themes: Vec::new(),
        demographics: Vec::new(),
        categories: Vec::new(),
    }
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

        let items: Vec<CreateMediaItem> = results["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                let id = r["id"].as_i64()?;
                let name = r["name"].as_str()?.to_string();
                Some(map_search(&RawgSearchResult {
                    id,
                    name: name.clone(),
                    background_image: r["background_image"].as_str().map(String::from),
                    rating: r["rating"].as_f64(),
                    description: r["description"].as_str().map(String::from),
                    released: r["released"].as_str().map(String::from),
                }))
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/games/{}", BASE_URL, id))?;
        url.query_pairs_mut().append_pair("key", &self.api_key);

        let response = self.client.get(url.as_str()).send().await?;
        let r: RawgDetails = response.json().await?;
        Ok(map_details(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_game_details() {
        let json = r#"{
            "id": 3498,
            "name": "GTA V",
            "background_image": "https://...",
            "rating": 4.47,
            "ratings_count": 5000,
            "metacritic": 96,
            "description_raw": "...",
            "released": "2013-09-17",
            "playtime": 32,
            "esrb_rating": {"name": "Mature"},
            "genres": [{"name": "Action"}, {"name": "Adventure"}],
            "platforms": [{"platform": {"name": "PC"}}, {"platform": {"name": "PlayStation 5"}}],
            "developers": [{"name": "Rockstar North"}],
            "publishers": [{"name": "Rockstar Games"}]
        }"#;
        let details: RawgDetails = serde_json::from_str(json).unwrap();
        let item = map_details(details);
        assert_eq!(item.title, "GTA V");
        assert_eq!(item.playtime_hours, Some(32));
        assert_eq!(item.year, Some(2013));
        assert_eq!(item.rating.as_deref(), Some("Mature"));
        assert_eq!(item.rating_votes, Some(5000));
        assert!(item.genres.contains(&"Action".to_string()));
        assert!(item.authors.contains(&"Rockstar North".to_string()));
        assert!(item.publishers.contains(&"Rockstar Games".to_string()));
        assert!(item.platforms.contains(&"PC".to_string()));
    }
}
