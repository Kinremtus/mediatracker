use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::models::media_item::CreateMediaItem;
use crate::utils::clean_description;

const BASE_URL: &str = "https://shikimori.one/api";
const USER_AGENT: &str = "MediaTracker/0.1 (+https://github.com/Kinremtus/mediatracker)";

#[derive(Debug, Deserialize)]
struct ShikimoriSearchResult {
    id: i64,
    name: String,
    name_en: Option<String>,
    russian: Option<String>,
    image: Option<ShikimoriImage>,
    kind: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_f64")]
    score: Option<f64>,
    status: Option<String>,
    episodes: Option<i32>,
    episodes_aired: Option<i32>,
    description: Option<String>,
    mal_id: Option<i64>,
    aired_on: Option<chrono::NaiveDate>,
    released_on: Option<chrono::NaiveDate>,
    rating: Option<String>,
    duration: Option<i32>,
    #[serde(default)]
    studios: Vec<ShikimoriStudio>,
    #[serde(default)]
    genres: Vec<ShikimoriGenre>,
    #[serde(default)]
    demographics: Vec<ShikimoriDemographic>,
}

fn deserialize_optional_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_f64()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("invalid number")),
        Some(serde_json::Value::String(s)) => s
            .parse()
            .map(Some)
            .map_err(serde::de::Error::custom),
        _ => Err(serde::de::Error::custom("expected number or string")),
    }
}

#[derive(Debug, Deserialize)]
pub struct ShikimoriImage {
    pub original: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ShikiCalendarEntry {
    pub next_episode: i32,
    pub next_episode_at: chrono::DateTime<chrono::Utc>,
    pub anime: ShikiCalendarAnime,
}

#[derive(Debug, Deserialize)]
pub struct ShikiCalendarAnime {
    pub id: i64,
    pub name: String,
    pub russian: Option<String>,
    pub image: ShikimoriImage,
}

#[derive(Debug, Deserialize)]
struct ShikimoriStudio {
    name: String,
}

#[derive(Debug, Deserialize)]
struct ShikimoriGenre {
    name: String,
    russian: Option<String>,
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ShikimoriDemographic {
    name: String,
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

fn extract_studio_names(items: &[ShikimoriStudio]) -> Vec<String> {
    items.iter().map(|s| s.name.clone()).collect()
}

fn extract_genre_names(items: &[ShikimoriGenre]) -> (Vec<String>, Vec<String>) {
    // Shikimori: kind = "theme" → themes, иначе → genres
    let mut genres = Vec::new();
    let mut themes = Vec::new();
    for g in items {
        match g.kind.as_deref() {
            Some("theme") => themes.push(g.name.clone()),
            _ => genres.push(g.name.clone()),
        }
    }
    (genres, themes)
}

fn extract_demographic_names(items: &[ShikimoriDemographic]) -> Vec<String> {
    items.iter().map(|d| d.name.clone()).collect()
}

fn map_anime(r: ShikimoriSearchResult) -> CreateMediaItem {
    let comparison_key = r.name_en.clone().unwrap_or_else(|| r.name.clone());
    let (genres, themes) = extract_genre_names(&r.genres);
    let demographics = extract_demographic_names(&r.demographics);
    let studios = extract_studio_names(&r.studios);

    // aired_on / released_on могут быть одинаковыми для аниме; используем aired_on
    let (aired_from, aired_to) = (r.aired_on, r.released_on);

    let format_type = r.kind.as_ref().map(|k| match k.as_str() {
        "tv" => "TV",
        "movie" => "Movie",
        "ova" => "OVA",
        "ona" => "ONA",
        "special" => "Special",
        "music" => "Music",
        other => other,
    }.to_string());

    let duration_text = r.duration.map(|m| format!("{m} min."));

    CreateMediaItem {
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
        title_russian: r.russian,
        poster_url: poster_url(r.image.and_then(|img| img.original)),
        episodes: r.episodes,
        description: clean_description(r.description),
        status: r.status,
        score: r.score,
        is_tracked: false,
        mal_id: r.mal_id,
        shikimori_id: Some(r.id),
        comparison_key: Some(comparison_key),
        format_type,
        details: None,
        chapters: None,
        volumes: None,
        pages: None,
        runtime_minutes: r.duration,
        playtime_hours: None,
        year: None,
        aired_from,
        aired_to,
        premiered_season: None,
        premiered_year: None,
        broadcast: None,
        completed: None,
        licensed: None,
        source: None,
        duration: duration_text,
        rating: r.rating,
        rating_votes: None,
        authors: Vec::new(),
        artists: Vec::new(),
        studios,
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers: Vec::new(),
        serialized_in: Vec::new(),
        networks: Vec::new(),
        platforms: Vec::new(),
        genres,
        themes,
        demographics,
        categories: Vec::new(),
    }
}

#[derive(Clone)]
pub struct ShikimoriService {
    client: Client,
}

impl ShikimoriService {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent(USER_AGENT)
                .build()
                .expect("reqwest client"),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/animes", BASE_URL))?;
        url.query_pairs_mut()
            .append_pair("search", query)
            .append_pair("limit", "50");
        let response = self.client.get(url).send().await?;
        let results: Vec<ShikimoriSearchResult> = response.json().await?;

        let items = results.into_iter().map(map_anime).collect();

        Ok(items)
    }

    pub async fn fetch_calendar(&self) -> Result<Vec<ShikiCalendarEntry>, anyhow::Error> {
        let url = format!("{}/calendar", BASE_URL);
        let response = self.client.get(&url).send().await?;
        let entries: Vec<ShikiCalendarEntry> = response.json().await?;
        Ok(entries)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/animes/{}", BASE_URL, id);
        let response = self.client.get(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Shikimori details failed: {}", response.status());
        }
        let r: ShikimoriSearchResult = response.json().await?;
        Ok(map_anime(r))
    }

    /// Fetch the full episode list for an anime from Shikimori.
    /// **Currently broken in production**: Shikimori's REST endpoint
    /// `/api/animes/{id}/episodes` returns 301 → HTML 404 on both
    /// shikimori.one and shikimori.io. We use Jikan v4 for episodes
    /// instead (see `MalService::fetch_episodes` in `mal.rs`). This
    /// method is kept for when Shikimori restores the endpoint.
    pub async fn fetch_episodes(&self, shikimori_id: i64) -> Result<Vec<ShikimoriEpisode>, anyhow::Error> {
        let url = format!("{}/animes/{}/episodes", BASE_URL, shikimori_id);
        let response = self.client.get(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Shikimori episodes failed: {}", response.status());
        }
        let episodes: Vec<ShikimoriEpisode> = response.json().await?;
        Ok(episodes)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShikimoriEpisode {
    pub number: i32,
    #[serde(default)]
    pub name_en: Option<String>,
    #[serde(default)]
    pub name_ru: Option<String>,
    #[serde(default)]
    pub airdate: Option<chrono::NaiveDate>,
    #[serde(default)]
    pub duration: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_response_with_string_score() {
        let json = r#"[{"id":20,"name":"Naruto","russian":"Наруто","kind":"tv","score":"8.02","status":"released","episodes":220}]"#;
        let results: Vec<ShikimoriSearchResult> = serde_json::from_str(json).unwrap();
        assert_eq!(results[0].score, Some(8.02));
        assert_eq!(results[0].russian.as_deref(), Some("Наруто"));
    }

    #[test]
    fn parses_genres_themes_demographics() {
        // Note: no `score` field on purpose — exercises the
        // `#[serde(default)]` on ShikimoriSearchResult.score, which
        // is what keeps us alive when Shikimori omits it (older
        // entries, ranking not yet computed, etc.).
        let json = r#"{
            "id": 20,
            "name": "Naruto",
            "kind": "tv",
            "episodes": 220,
            "status": "released",
            "studios": [{"name": "Studio Pierrot"}],
            "genres": [
                {"name": "Action", "kind": "genre"},
                {"name": "Martial Arts", "kind": "theme"}
            ],
            "demographics": [{"name": "Shounen"}]
        }"#;
        let r: ShikimoriSearchResult = serde_json::from_str(json).unwrap();
        let item = map_anime(r);
        assert_eq!(item.format_type.as_deref(), Some("TV"));
        assert!(item.studios.contains(&"Studio Pierrot".to_string()));
        assert!(item.genres.contains(&"Action".to_string()));
        assert!(item.themes.contains(&"Martial Arts".to_string()));
        assert!(item.demographics.contains(&"Shounen".to_string()));
        assert!(item.score.is_none(), "missing score must default to None");
    }
}
