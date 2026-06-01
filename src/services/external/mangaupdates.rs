use reqwest::Client;
use serde::Deserialize;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://api.mangaupdates.com/v1";

#[derive(Debug, Deserialize)]
struct MangaUpdatesSearchResponse {
    results: Vec<MangaUpdatesSearchHit>,
}

#[derive(Debug, Deserialize)]
struct MangaUpdatesSearchHit {
    record: MangaUpdatesSeries,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
struct MangaUpdatesSeries {
    series_id: i64,
    title: String,
    url: Option<String>,
    associated: Option<Vec<MangaUpdatesAssociated>>,
    description: Option<String>,
    image: Option<MangaUpdatesImage>,
    #[serde(rename = "type")]
    series_type: Option<String>,
    year: Option<String>,
    bayesian_rating: Option<f64>,
    rating_votes: Option<i32>,
    genres: Option<Vec<MangaUpdatesGenre>>,
    categories: Option<Vec<MangaUpdatesCategory>>,
    latest_chapter: Option<i32>,
    forum_id: Option<i64>,
    status: Option<String>,
    licensed: Option<bool>,
    completed: Option<bool>,
    anime: Option<MangaUpdatesAnime>,
    #[serde(default)]
    related_series: Vec<MangaUpdatesRelatedSeries>,
    #[serde(default)]
    authors: Vec<MangaUpdatesAuthor>,
    #[serde(default)]
    publishers: Vec<MangaUpdatesPublisher>,
    #[serde(default)]
    publications: Vec<MangaUpdatesPublication>,
    last_updated: Option<MangaUpdatesTime>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesAssociated {
    title: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesImage {
    url: Option<MangaUpdatesImageUrl>,
    height: Option<i32>,
    width: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesImageUrl {
    original: Option<String>,
    thumb: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesGenre {
    genre: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
struct MangaUpdatesCategory {
    series_id: Option<i64>,
    category: String,
    votes: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesAnime {
    start: Option<String>,
    end: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[allow(dead_code)]
struct MangaUpdatesRelatedSeries {
    relation_id: Option<i64>,
    relation_type: Option<String>,
    related_series_id: Option<i64>,
    related_series_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesAuthor {
    name: String,
    #[serde(rename = "type")]
    author_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesPublisher {
    publisher_name: String,
    #[serde(rename = "type")]
    publisher_type: Option<String>,
    notes: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesPublication {
    publication_name: String,
    publisher_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MangaUpdatesTime {
    as_rfc3339: Option<String>,
}

fn media_type_for(series_type: Option<&str>) -> String {
    match series_type {
        Some("Manga") => "manga".to_string(),
        Some("Manhwa") => "manhwa".to_string(),
        Some("Manhua") => "manhua".to_string(),
        Some("Novel") => "novel".to_string(),
        Some("OEL")
        | Some("Doujinshi")
        | Some("Filipino")
        | Some("Indonesian")
        | Some("Thai")
        | Some("Vietnamese")
        | Some("Malaysian")
        | Some("Nordic")
        | Some("French")
        | Some("Spanish")
        | Some("German")
        | Some("Drama CD")
        | Some("Artbook") => "other-comics".to_string(),
        _ => "manga".to_string(),
    }
}

fn poster_url_from(image: Option<MangaUpdatesImage>) -> Option<String> {
    image.and_then(|img| img.url).and_then(|u| u.original)
}

fn extract_names<T, F>(items: Option<Vec<T>>, name_fn: F) -> Vec<String>
where
    F: Fn(T) -> String,
{
    items
        .map(|v| v.into_iter().map(name_fn).collect())
        .unwrap_or_default()
}

/// MangaUpdates возвращает `status` в формате:
///   "N Volumes (StatusWord)  \nM SpecialVolumes (StatusWord)" — для завершённых
///   "Ongoing" / " hiatus" / "Discontinued" / "Cancelled" — для продолжающихся
/// Эта функция отделяет количество томов от статусного слова.
fn parse_mu_status(raw: Option<String>) -> (Option<String>, Option<i32>) {
    let s = match raw {
        Some(s) if !s.is_empty() => s,
        _ => return (None, None),
    };
    if let Some(volumes) = extract_first_volume_count(&s) {
        let status = extract_parenthesized_after_volumes(&s);
        (status, Some(volumes))
    } else {
        (Some(s), None)
    }
}

fn extract_first_volume_count(s: &str) -> Option<i32> {
    let idx = s.find("Volume")?;
    let before = s[..idx].trim_end();
    let num_start = before
        .rfind(|c: char| !c.is_ascii_digit())
        .map(|i| i + 1)
        .unwrap_or(0);
    before[num_start..].parse().ok()
}

fn extract_parenthesized_after_volumes(s: &str) -> Option<String> {
    let idx = s.find("Volume")?;
    let rest = &s[idx..];
    let open = rest.find('(')?;
    let close_rel = rest[open..].find(')')?;
    let content = rest[open + 1..open + close_rel].trim();
    if content.is_empty() {
        None
    } else {
        Some(content.to_string())
    }
}

fn map_series(series: MangaUpdatesSeries) -> CreateMediaItem {
    let media_type = media_type_for(series.series_type.as_deref());
    let poster_url = poster_url_from(series.image);

    let title_for_key = series.title.clone();

    let genres: Vec<String> = extract_names(series.genres.clone(), |g| g.genre);
    let categories: Vec<String> = extract_names(series.categories.clone(), |c| c.category);

    let (mut authors, mut artists) = (Vec::new(), Vec::new());
    for a in series.authors.iter() {
        match a.author_type.as_deref() {
            Some("Artist") => artists.push(a.name.clone()),
            _ => authors.push(a.name.clone()),
        }
    }

    let mut publishers: Vec<String> = Vec::new();
    for p in series.publishers.iter() {
        publishers.push(p.publisher_name.clone());
    }
    let serialized_in: Vec<String> = series
        .publications
        .iter()
        .map(|p| p.publication_name.clone())
        .collect();

    let year_parsed = series
        .year
        .as_ref()
        .and_then(|y| y.parse::<i16>().ok());

    let (status, volumes) = parse_mu_status(series.status);

    let mut details = serde_json::Map::new();
    if let Some(start) = series.anime.as_ref().and_then(|a| a.start.clone()) {
        details.insert("anime_start_chapter".to_string(), serde_json::Value::String(start));
    }
    if let Some(end) = series.anime.as_ref().and_then(|a| a.end.clone()) {
        details.insert("anime_end_chapter".to_string(), serde_json::Value::String(end));
    }

    CreateMediaItem {
        provider: "mangaupdates".to_string(),
        external_id: series.series_id.to_string(),
        media_type,
        title: series.title.clone(),
        title_english: None,
        title_native: None,
        title_russian: None,
        poster_url,
        episodes: None,
        description: series.description,
        status,
        score: series.bayesian_rating,
        is_tracked: false,
        mal_id: None,
        comparison_key: Some(title_for_key),
        format_type: series.series_type,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
        chapters: series.latest_chapter,
        volumes,
        pages: None,
        runtime_minutes: None,
        playtime_hours: None,
        year: year_parsed,
        aired_from: None,
        aired_to: None,
        premiered_season: None,
        premiered_year: None,
        broadcast: None,
        completed: series.completed,
        licensed: series.licensed,
        source: None,
        duration: None,
        rating: None,
        rating_votes: series.rating_votes,
        authors,
        artists,
        studios: Vec::new(),
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers,
        serialized_in,
        networks: Vec::new(),
        platforms: Vec::new(),
        genres,
        themes: Vec::new(),
        demographics: Vec::new(),
        categories,
    }
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

        let items: Vec<CreateMediaItem> = results
            .into_iter()
            .filter_map(|hit| {
                let series_type = hit.record.series_type.as_deref().unwrap_or("");

                if !allowed_types.is_empty() {
                    let type_lower = series_type.to_lowercase();
                    let matches = allowed_types.iter().any(|t| type_lower == t.to_lowercase());
                    if !matches {
                        return None;
                    }
                }

                Some(map_series(hit.record))
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/series/{}", BASE_URL, id);
        let response = self.client.get(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!(
                "MangaUpdates details failed: {} for id={}",
                response.status(),
                id
            );
        }
        let series: MangaUpdatesSeries = response.json().await?;
        Ok(map_series(series))
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

    #[test]
    fn parses_full_series_response() {
        let json = r#"{
            "series_id": 112,
            "title": "Naruto",
            "url": "/series.html?id=112",
            "description": "Twelve years ago...",
            "image": {"url": {"original": "https://...", "thumb": "https://..."}, "height": 300, "width": 200},
            "type": "Manga",
            "year": "1999",
            "bayesian_rating": 7.7,
            "rating_votes": 5580,
            "genres": [{"genre": "Action"}, {"genre": "Adventure"}],
            "categories": [{"category": "Adapted to Anime", "votes": 100}],
            "latest_chapter": 700,
            "status": "72 Volumes (Complete)",
            "licensed": true,
            "completed": true,
            "anime": {"start": "Vol 1, Chap 1", "end": "Vol 28, Chap 245"},
            "authors": [{"name": "Kishimoto Masashi", "type": "Author"}],
            "publishers": [{"publisher_name": "Shueisha", "type": "Original"}],
            "publications": [{"publication_name": "Shukan Shounen Jump", "publisher_name": "Shueisha"}]
        }"#;
        let series: MangaUpdatesSeries = serde_json::from_str(json).unwrap();
        let item = map_series(series);
        assert_eq!(item.title, "Naruto");
        assert_eq!(item.media_type, "manga");
        assert_eq!(item.format_type.as_deref(), Some("Manga"));
        assert_eq!(item.chapters, Some(700));
        assert_eq!(item.year, Some(1999));
        assert_eq!(item.completed, Some(true));
        assert_eq!(item.licensed, Some(true));
        assert_eq!(item.score, Some(7.7));
        assert_eq!(item.rating_votes, Some(5580));
        assert!(item.genres.contains(&"Action".to_string()));
        assert!(item.genres.contains(&"Adventure".to_string()));
        assert!(item.authors.contains(&"Kishimoto Masashi".to_string()));
        assert!(item.publishers.contains(&"Shueisha".to_string()));
        assert!(item.serialized_in.contains(&"Shukan Shounen Jump".to_string()));
        assert!(item.categories.contains(&"Adapted to Anime".to_string()));
        let details = item.details.expect("details should exist");
        assert_eq!(
            details.get("anime_start_chapter").and_then(|v| v.as_str()),
            Some("Vol 1, Chap 1")
        );
    }

    #[test]
    fn parse_mu_status_extracts_combini_ban() {
        let (s, v) = parse_mu_status(Some(
            "72 Volumes (Complete)  \n24 Combini-ban Volumes (Complete)".to_string(),
        ));
        assert_eq!(s.as_deref(), Some("Complete"));
        assert_eq!(v, Some(72));
    }

    #[test]
    fn parse_mu_status_ongoing_no_volumes() {
        let (s, v) = parse_mu_status(Some("Ongoing".to_string()));
        assert_eq!(s.as_deref(), Some("Ongoing"));
        assert_eq!(v, None);
    }

    #[test]
    fn parse_mu_status_one_piece() {
        let (s, v) = parse_mu_status(Some("110 Volumes (Ongoing)".to_string()));
        assert_eq!(s.as_deref(), Some("Ongoing"));
        assert_eq!(v, Some(110));
    }

    #[test]
    fn parse_mu_status_short_completed() {
        let (s, v) = parse_mu_status(Some("4 Volumes (Complete)".to_string()));
        assert_eq!(s.as_deref(), Some("Complete"));
        assert_eq!(v, Some(4));
    }

    #[test]
    fn parse_mu_status_empty() {
        let (s, v) = parse_mu_status(None);
        assert_eq!(s, None);
        assert_eq!(v, None);
    }
}
