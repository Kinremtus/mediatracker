use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::models::media_item::CreateMediaItem;
use crate::utils::clean_description;

/// MyAnimeList data via Jikan v4 (https://jikan.moe) — no API key required.
const BASE_URL: &str = "https://api.jikan.moe/v4";
const USER_AGENT: &str = "MediaTracker/0.1 (+https://github.com/Kinremtus/mediatracker)";
const SEARCH_LIMIT: u32 = 25;

#[derive(Debug, Deserialize)]
struct MalAnimeSearchResponse {
    data: Vec<MalAnimeSearchItem>,
}

#[derive(Debug, Deserialize)]
struct MalAnimeResponse {
    data: MalAnimeFull,
}

#[derive(Debug, Deserialize)]
struct MalAnimeSearchItem {
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
    genres: Option<Vec<MalNamed>>,
    themes: Option<Vec<MalNamed>>,
    demographics: Option<Vec<MalNamed>>,
}

#[derive(Debug, Deserialize)]
struct MalAnimeFull {
    mal_id: i64,
    title: String,
    title_english: Option<String>,
    title_japanese: Option<String>,
    images: Option<MalImages>,
    episodes: Option<i32>,
    synopsis: Option<String>,
    score: Option<f64>,
    scored_by: Option<i64>,
    status: Option<String>,
    #[serde(rename = "type")]
    anime_type: Option<String>,
    source: Option<String>,
    aired: Option<MalAired>,
    duration: Option<String>,
    rating: Option<String>,
    season: Option<String>,
    year: Option<i32>,
    broadcast: Option<MalBroadcast>,
    producers: Option<Vec<MalNamed>>,
    licensors: Option<Vec<MalNamed>>,
    studios: Option<Vec<MalNamed>>,
    genres: Option<Vec<MalNamed>>,
    themes: Option<Vec<MalNamed>>,
    demographics: Option<Vec<MalNamed>>,
}

#[derive(Debug, Deserialize)]
struct MalNamed {
    name: String,
}

#[derive(Debug, Deserialize)]
struct MalAired {
    from: Option<String>,
    to: Option<String>,
    string: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MalBroadcast {
    string: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JikanEpisode {
    pub mal_id: i32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub title_japanese: Option<String>,
    /// Jikan returns RFC3339 (e.g. "2002-10-03T00:00:00+00:00").
    /// Stored as `String` and parsed at the persistence layer so
    /// we can re-use the same `parse_date()` helper that detail
    /// responses already use.
    #[serde(default)]
    pub aired: Option<String>,
    /// Human-readable duration, e.g. "24 min. per ep.". Parsed
    /// into minutes by `parse_duration_to_minutes()` at persistence.
    #[serde(default)]
    pub duration: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JikanEpisodesResponse {
    data: Vec<JikanEpisode>,
    pagination: JikanPagination,
}

#[derive(Debug, Deserialize)]
struct JikanPagination {
    last_visible_page: i32,
    has_next_page: bool,
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

fn extract_names(items: &Option<Vec<MalNamed>>) -> Vec<String> {
    items
        .as_ref()
        .map(|v| v.iter().map(|n| n.name.clone()).collect())
        .unwrap_or_default()
}

fn parse_date(s: &str) -> Option<chrono::NaiveDate> {
    // Jikan returns RFC3339 like "2002-10-03T00:00:00+00:00"
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.naive_utc().date())
}

/// Парсит строку `duration` от MAL в минуты (длительность одного эпизода).
/// Поддерживает форматы:
///   "1 hr. 52 min."   → 112
///   "2 hr."           → 120
///   "23 min. per ep." → 23
///   "59 min."         → 59
///   "45 sec. per ep." → 1   (округление вверх — минимальная единица)
///   "" / мусор        → None
pub fn parse_duration_to_minutes(s: &str) -> Option<i32> {
    let lower = s.to_lowercase();
    let mut total: i32 = 0;
    let mut found = false;

    if let Some(hours) = extract_number_before(&lower, "hr") {
        total += hours * 60;
        found = true;
    }
    if let Some(minutes) = extract_number_before(&lower, "min") {
        total += minutes;
        found = true;
    } else if let Some(seconds) = extract_number_before(&lower, "sec") {
        // Секунды округляем вверх до 1 минуты (минимальная единица в БД).
        total += (seconds + 59) / 60;
        found = true;
    }

    if found {
        Some(total)
    } else {
        None
    }
}

/// Берёт целое число, стоящее непосредственно перед `unit` в строке.
/// Например: "1 hr. 52 min." + "hr" → 1, "1 hr. 52 min." + "min" → 52.
fn extract_number_before(s: &str, unit: &str) -> Option<i32> {
    let pos = s.find(unit)?;
    let before = &s[..pos];
    // Разбиваем по не-цифровым символам и берём последний непустой кусок.
    let last = before
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .last()?;
    last.parse().ok()
}

fn map_full(anime: MalAnimeFull) -> CreateMediaItem {
    let comparison_key = anime.title_english.clone().unwrap_or_else(|| anime.title.clone());

    let aired_from = anime
        .aired
        .as_ref()
        .and_then(|a| a.from.as_deref())
        .and_then(parse_date);
    let aired_to = anime
        .aired
        .as_ref()
        .and_then(|a| a.to.as_deref())
        .and_then(parse_date);

    let mut details = serde_json::Map::new();
    if let Some(a) = anime.aired.as_ref() {
        if let Some(s) = a.string.as_ref() {
            details.insert("aired_string".to_string(), serde_json::Value::String(s.clone()));
        }
    }
    if let Some(b) = anime.broadcast.as_ref() {
        if let Some(s) = b.string.as_ref() {
            details.insert("broadcast".to_string(), serde_json::Value::String(s.clone()));
        }
    }

    let year_i16: Option<i16> = anime.year.and_then(|y| i16::try_from(y).ok());
    let premiered_year_i16 = year_i16;

    CreateMediaItem {
        provider: "mal".to_string(),
        external_id: anime.mal_id.to_string(),
        media_type: "anime".to_string(),
        title: anime.title,
        title_english: anime.title_english,
        title_native: anime.title_japanese,
        title_russian: None,
        poster_url: poster_url(&anime.images),
        episodes: anime.episodes,
        description: clean_description(anime.synopsis),
        status: anime.status,
        score: anime.score,
        is_tracked: false,
        mal_id: Some(anime.mal_id),
        shikimori_id: None,
        comparison_key: Some(comparison_key),
        format_type: anime.anime_type,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
        chapters: None,
        volumes: None,
        pages: None,
        runtime_minutes: anime.duration.as_deref().and_then(parse_duration_to_minutes),
        playtime_hours: None,
        year: year_i16,
        aired_from,
        aired_to,
        premiered_season: anime.season,
        premiered_year: premiered_year_i16,
        broadcast: anime.broadcast.as_ref().and_then(|b| b.string.clone()),
        completed: None,
        licensed: None,
        source: anime.source,
        duration: anime.duration,
        rating: anime.rating,
        rating_votes: anime.scored_by.and_then(|v| i32::try_from(v).ok()),
        authors: Vec::new(),
        artists: Vec::new(),
        studios: extract_names(&anime.studios),
        producers: extract_names(&anime.producers),
        licensors: extract_names(&anime.licensors),
        publishers: Vec::new(),
        serialized_in: Vec::new(),
        networks: Vec::new(),
        platforms: Vec::new(),
        genres: extract_names(&anime.genres),
        themes: extract_names(&anime.themes),
        demographics: extract_names(&anime.demographics),
        categories: Vec::new(),
    }
}

fn map_search(item: MalAnimeSearchItem) -> CreateMediaItem {
    let comparison_key = item.title_english.clone().unwrap_or_else(|| item.title.clone());
    CreateMediaItem {
        provider: "mal".to_string(),
        external_id: item.mal_id.to_string(),
        media_type: "anime".to_string(),
        title: item.title,
        title_english: item.title_english,
        title_native: item.title_japanese,
        title_russian: None,
        poster_url: poster_url(&item.images),
        episodes: item.episodes,
        description: clean_description(item.synopsis),
        status: item.status,
        score: item.score,
        is_tracked: false,
        mal_id: Some(item.mal_id),
        shikimori_id: None,
        comparison_key: Some(comparison_key),
        format_type: item.anime_type,
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
        genres: extract_names(&item.genres),
        themes: extract_names(&item.themes),
        demographics: extract_names(&item.demographics),
        categories: Vec::new(),
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
        Ok(body.data.into_iter().map(map_search).collect())
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/anime/{id}/full", BASE_URL);
        let response = self.client.get(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("MAL/Jikan details failed: {}", response.status());
        }

        let body: MalAnimeResponse = response.json().await?;
        Ok(map_full(body.data))
    }

    /// Fetch the full episode list for an anime from Jikan v4.
    /// Endpoint: `GET /v4/anime/{mal_id}/episodes?page=N` — paginated
    /// (100 per page, rate-limited to ~3 req/sec and ~60 req/min).
    /// Iterates pages until `pagination.has_next_page == false`.
    pub async fn fetch_episodes(&self, mal_id: i64) -> Result<Vec<JikanEpisode>, anyhow::Error> {
        let mut all = Vec::new();
        let mut page = 1;
        loop {
            let url = format!("{}/anime/{}/episodes?page={}", BASE_URL, mal_id, page);
            let response = self.client.get(&url).send().await?;
            if !response.status().is_success() {
                anyhow::bail!(
                    "Jikan episodes failed for mal_id={} page={}: {}",
                    mal_id,
                    page,
                    response.status()
                );
            }
            let body: JikanEpisodesResponse = response.json().await?;
            let got = body.data.len();
            all.extend(body.data);
            if got == 0
                || !body.pagination.has_next_page
                || page >= body.pagination.last_visible_page
            {
                break;
            }
            page += 1;
            // Jikan rate limit: 3 req/sec, 60 req/min. 350 ms keeps us
            // under the per-second cap with margin.
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
        }
        Ok(all)
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

    #[test]
    fn parses_full_anime_response() {
        let json = r#"{
            "data": {
                "mal_id": 20,
                "title": "Naruto",
                "title_english": "Naruto",
                "title_japanese": "ナルト",
                "episodes": 220,
                "synopsis": "...",
                "score": 7.99,
                "scored_by": 250000,
                "status": "Finished Airing",
                "type": "TV",
                "source": "Manga",
                "aired": {"from": "2002-10-03T00:00:00+00:00", "to": "2007-02-08T00:00:00+00:00", "string": "Oct 3, 2002 to Feb 8, 2007"},
                "duration": "23 min. per ep.",
                "rating": "PG-13 - Teens 13 or older",
                "season": "fall",
                "year": 2002,
                "broadcast": {"string": "Thursdays at 19:30 (JST)"},
                "producers": [{"name": "TV Tokyo"}, {"name": "Aniplex"}],
                "licensors": [{"name": "VIZ Media"}],
                "studios": [{"name": "Studio Pierrot"}],
                "genres": [{"name": "Action"}, {"name": "Adventure"}],
                "themes": [{"name": "Martial Arts"}],
                "demographics": [{"name": "Shounen"}]
            }
        }"#;
        let body: MalAnimeResponse = serde_json::from_str(json).unwrap();
        let item = map_full(body.data);
        assert_eq!(item.title, "Naruto");
        assert_eq!(item.format_type.as_deref(), Some("TV"));
        assert_eq!(item.source.as_deref(), Some("Manga"));
        assert_eq!(item.premiered_season.as_deref(), Some("fall"));
        assert_eq!(item.premiered_year, Some(2002));
        assert_eq!(item.aired_from, Some(chrono::NaiveDate::from_ymd_opt(2002, 10, 3).unwrap()));
        assert_eq!(item.aired_to, Some(chrono::NaiveDate::from_ymd_opt(2007, 2, 8).unwrap()));
        assert_eq!(item.duration.as_deref(), Some("23 min. per ep."));
        assert_eq!(item.rating.as_deref(), Some("PG-13 - Teens 13 or older"));
        assert!(item.studios.contains(&"Studio Pierrot".to_string()));
        assert!(item.producers.contains(&"TV Tokyo".to_string()));
        assert!(item.licensors.contains(&"VIZ Media".to_string()));
        assert!(item.genres.contains(&"Action".to_string()));
        assert!(item.themes.contains(&"Martial Arts".to_string()));
        assert!(item.demographics.contains(&"Shounen".to_string()));
        let details = item.details.expect("details should exist");
        assert_eq!(details.get("aired_string").and_then(|v| v.as_str()), Some("Oct 3, 2002 to Feb 8, 2007"));
    }

    #[test]
    fn parses_duration_hours_and_minutes() {
        assert_eq!(parse_duration_to_minutes("1 hr. 52 min."), Some(112));
    }

    #[test]
    fn parses_duration_only_minutes() {
        assert_eq!(parse_duration_to_minutes("23 min. per ep."), Some(23));
    }

    #[test]
    fn parses_duration_only_hours() {
        assert_eq!(parse_duration_to_minutes("2 hr."), Some(120));
    }

    #[test]
    fn parses_duration_seconds_rounds_up() {
        // 45 секунд → 1 минута (округление вверх)
        assert_eq!(parse_duration_to_minutes("45 sec. per ep."), Some(1));
    }

    #[test]
    fn parses_duration_empty_returns_none() {
        assert_eq!(parse_duration_to_minutes(""), None);
    }

    #[test]
    fn parses_duration_garbage_returns_none() {
        assert_eq!(parse_duration_to_minutes("Unknown"), None);
    }
}
