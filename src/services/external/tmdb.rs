use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://api.themoviedb.org/3";

#[derive(Debug, Deserialize)]
#[expect(dead_code)]
struct TmdbGenre {
    id: i64,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbSeasonInfo {
    pub season_number: i32,
    pub name: String,
    pub episode_count: i32,
    pub air_date: Option<String>,
    pub overview: Option<String>,
    pub poster_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbEpisodeInfo {
    pub episode_number: i32,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub still_path: Option<String>,
    pub air_date: Option<String>,
    pub runtime: Option<i32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TmdbSeasonDetail {
    pub episodes: Vec<TmdbEpisodeInfo>,
}

#[derive(Debug, Deserialize)]
#[expect(dead_code)]
struct TmdbCompany {
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize)]
#[expect(dead_code)]
struct TmdbNetwork {
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct TmdbDetails {
    id: i64,
    title: Option<String>,
    name: Option<String>,
    original_title: Option<String>,
    original_name: Option<String>,
    poster_path: Option<String>,
    vote_average: Option<f64>,
    vote_count: Option<i64>,
    overview: Option<String>,
    number_of_seasons: Option<i32>,
    number_of_episodes: Option<i32>,
    status: Option<String>,
    #[serde(default)]
    genres: Vec<TmdbGenre>,
    #[serde(default)]
    production_companies: Vec<TmdbCompany>,
    #[serde(default)]
    networks: Vec<TmdbNetwork>,
    runtime: Option<i32>,
    #[serde(default)]
    episode_run_time: Vec<i32>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    last_air_date: Option<String>,
    original_language: Option<String>,
    #[serde(default)]
    seasons: Vec<TmdbSeasonInfo>,
}

fn parse_date(s: Option<&str>) -> Option<chrono::NaiveDate> {
    s.and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

fn poster_url_from(path: Option<&str>) -> Option<String> {
    path.map(|p| format!("/tmdb-image/{}", p.trim_start_matches('/')))
}

fn extract_genre_names(items: &[TmdbGenre]) -> Vec<String> {
    items.iter().map(|g| g.name.clone()).collect()
}

fn extract_company_names(items: &[TmdbCompany]) -> Vec<String> {
    items.iter().map(|c| c.name.clone()).collect()
}

fn extract_network_names(items: &[TmdbNetwork]) -> Vec<String> {
    items.iter().map(|n| n.name.clone()).collect()
}

fn map_details(r: TmdbDetails, media_type: &str) -> CreateMediaItem {
    let poster_url = poster_url_from(r.poster_path.as_deref());
    let title = r.title.clone().or_else(|| r.name.clone()).unwrap_or_default();
    let original_title = r.original_title.clone().or(r.original_name.clone());

    let aired_from = if media_type == "movie" {
        parse_date(r.release_date.as_deref())
    } else {
        parse_date(r.first_air_date.as_deref())
    };
    let aired_to = if media_type == "movie" {
        None
    } else {
        parse_date(r.last_air_date.as_deref())
    };

    // runtime: для movie — runtime, для tv — episode_run_time[0] или среднее
    let runtime_minutes = if media_type == "movie" {
        r.runtime
    } else {
        r.episode_run_time.first().copied().or(r.runtime)
    };

    let format_type = if media_type == "movie" {
        Some("Movie".to_string())
    } else {
        Some("TV".to_string())
    };

    let mut details = serde_json::Map::new();
    if let Some(lang) = r.original_language.as_ref() {
        details.insert(
            "original_language".to_string(),
            serde_json::Value::String(lang.clone()),
        );
    }

    CreateMediaItem {
        provider: "tmdb".to_string(),
        external_id: r.id.to_string(),
        media_type: media_type.to_string(),
        title: title.clone(),
        title_english: None,
        title_native: original_title,
        title_russian: None,
        poster_url,
        episodes: if media_type == "movie" {
            None
        } else {
            r.number_of_episodes.or(r.number_of_seasons)
        },
        description: r.overview,
        status: r.status,
        score: r.vote_average,
        is_tracked: false,
        mal_id: None,
        shikimori_id: None,
        comparison_key: Some(title),
        format_type,
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
        chapters: None,
        volumes: None,
        pages: None,
        runtime_minutes,
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
        duration: None,
        rating: None,
        rating_votes: r.vote_count.and_then(|v| i32::try_from(v).ok()),
        authors: Vec::new(),
        artists: Vec::new(),
        studios: Vec::new(),
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers: extract_company_names(&r.production_companies),
        serialized_in: Vec::new(),
        networks: extract_network_names(&r.networks),
        platforms: Vec::new(),
        genres: extract_genre_names(&r.genres),
        themes: Vec::new(),
        demographics: Vec::new(),
        categories: Vec::new(),
    }
}

fn map_search_result(r: &serde_json::Value, media_type: &str) -> Option<CreateMediaItem> {
    let id = r["id"].as_i64()?;
    let title = r["title"]
        .as_str()
        .or_else(|| r["name"].as_str())?
        .to_string();
    let poster_path = r["poster_path"].as_str();
    let poster_url = poster_url_from(poster_path);
    let score = r["vote_average"].as_f64();
    let description = r["overview"].as_str().map(String::from);

    Some(CreateMediaItem {
        provider: "tmdb".to_string(),
        external_id: id.to_string(),
        media_type: media_type.to_string(),
        title: title.clone(),
        title_english: None,
        title_native: r["original_title"]
            .as_str()
            .or_else(|| r["original_name"].as_str())
            .map(String::from),
        title_russian: None,
        poster_url,
        episodes: None,
        description,
        status: None,
        score,
        is_tracked: false,
        mal_id: None,
        shikimori_id: None,
        comparison_key: Some(title),
        format_type: None,
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
        rating_votes: r["vote_count"].as_i64().and_then(|v| i32::try_from(v).ok()),
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
    })
}

#[derive(Clone)]
pub struct TmdbService {
    client: Client,
    pub api_key: String,
}

impl TmdbService {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/search/multi", BASE_URL))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("query", query)
            .append_pair("language", "ru-RU");

        let response = self.client.get(url.as_str()).send().await?;
        let results: serde_json::Value = response.json().await?;

        let items: Vec<CreateMediaItem> = results["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                let media_type = r["media_type"].as_str()?;
                if media_type != "movie" && media_type != "tv" {
                    return None;
                }
                map_search_result(r, if media_type == "movie" { "movie" } else { "series" })
            })
            .collect();

        Ok(items)
    }

    pub async fn search_movies(
        &self,
        query: &str,
        genre_id: Option<i64>,
    ) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/search/movie", BASE_URL))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("query", query)
            .append_pair("language", "ru-RU");

        if let Some(gid) = genre_id {
            url.query_pairs_mut().append_pair("with_genres", &gid.to_string());
        }

        let response = self.client.get(url.as_str()).send().await?;
        let results: serde_json::Value = response.json().await?;

        let items: Vec<CreateMediaItem> = results["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                if let Some(gid) = genre_id
                    && let Some(ids) = r["genre_ids"].as_array()
                    && !ids.iter().any(|id| id.as_i64() == Some(gid))
                {
                    return None;
                }
                map_search_result(r, "movie")
            })
            .collect();

        Ok(items)
    }

    pub async fn search_tv(
        &self,
        query: &str,
        genre_id: Option<i64>,
    ) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/search/tv", BASE_URL))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("query", query)
            .append_pair("language", "ru-RU");

        if let Some(gid) = genre_id {
            url.query_pairs_mut().append_pair("with_genres", &gid.to_string());
        }

        let response = self.client.get(url.as_str()).send().await?;
        let results: serde_json::Value = response.json().await?;

        let items: Vec<CreateMediaItem> = results["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                if let Some(gid) = genre_id
                    && let Some(ids) = r["genre_ids"].as_array()
                    && !ids.iter().any(|id| id.as_i64() == Some(gid))
                {
                    return None;
                }
                map_search_result(r, "series")
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(
        &self,
        id: &str,
        media_type: &str,
    ) -> Result<CreateMediaItem, anyhow::Error> {
        let tmdb_type = match media_type {
            "movie" | "animated-movies" => "movie",
            _ => "tv",
        };
        let mut url = Url::parse(&format!("{}/{}/{}", BASE_URL, tmdb_type, id))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("language", "ru-RU");

        let response = self.client.get(url.as_str()).send().await?;
        let r: TmdbDetails = response.json().await?;
        Ok(map_details(r, media_type))
    }

    pub async fn fetch_seasons(
        &self,
        id: &str,
    ) -> Result<Vec<TmdbSeasonInfo>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/tv/{}", BASE_URL, id))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("language", "ru-RU");

        let response = self.client.get(url.as_str()).send().await?;
        let details: TmdbDetails = response.json().await?;
        Ok(details.seasons)
    }

    pub async fn fetch_season_episodes(
        &self,
        id: &str,
        season_number: i32,
    ) -> Result<TmdbSeasonDetail, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/tv/{}/season/{}", BASE_URL, id, season_number))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("language", "ru-RU");

        let response = self.client.get(url.as_str()).send().await?;
        let detail: TmdbSeasonDetail = response.json().await?;
        Ok(detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_movie_details() {
        let json = r#"{
            "id": 27205,
            "title": "Inception",
            "original_title": "Inception",
            "poster_path": "/abc.jpg",
            "vote_average": 8.367,
            "vote_count": 30000,
            "overview": "Cobb...",
            "runtime": 148,
            "release_date": "2010-07-15",
            "status": "Released",
            "genres": [{"id": 28, "name": "Action"}, {"id": 878, "name": "Science Fiction"}],
            "production_companies": [{"id": 923, "name": "Legendary Pictures"}]
        }"#;
        let details: TmdbDetails = serde_json::from_str(json).unwrap();
        let item = map_details(details, "movie");
        assert_eq!(item.title, "Inception");
        assert_eq!(item.format_type.as_deref(), Some("Movie"));
        assert_eq!(item.runtime_minutes, Some(148));
        assert_eq!(item.rating_votes, Some(30000));
        assert_eq!(
            item.aired_from,
            Some(chrono::NaiveDate::from_ymd_opt(2010, 7, 15).unwrap())
        );
        assert!(item.genres.contains(&"Action".to_string()));
        assert!(item.publishers.contains(&"Legendary Pictures".to_string()));
    }

    #[test]
    fn parses_tv_details() {
        let json = r#"{
            "id": 1399,
            "name": "Game of Thrones",
            "original_name": "Game of Thrones",
            "poster_path": "/abc.jpg",
            "vote_average": 8.4,
            "vote_count": 10000,
            "overview": "...",
            "number_of_seasons": 8,
            "number_of_episodes": 73,
            "status": "Ended",
            "first_air_date": "2011-04-17",
            "last_air_date": "2019-05-19",
            "episode_run_time": [60],
            "genres": [{"id": 10765, "name": "Sci-Fi & Fantasy"}],
            "networks": [{"id": 1, "name": "HBO"}]
        }"#;
        let details: TmdbDetails = serde_json::from_str(json).unwrap();
        let item = map_details(details, "series");
        assert_eq!(item.format_type.as_deref(), Some("TV"));
        assert_eq!(item.episodes, Some(73));
        assert_eq!(item.runtime_minutes, Some(60));
        assert!(item.networks.contains(&"HBO".to_string()));
    }

    #[test]
    fn maps_movie_search_result() {
        let json = serde_json::json!({
            "id": 27205,
            "title": "Inception",
            "original_title": "Inception",
            "poster_path": "/abc.jpg",
            "vote_average": 8.367,
            "vote_count": 30000,
            "overview": "A thief who steals corporate secrets.",
            "genre_ids": [28, 878],
            "media_type": "movie"
        });
        let item = map_search_result(&json, "movie").unwrap();
        assert_eq!(item.title, "Inception");
        assert_eq!(item.external_id, "27205");
        assert_eq!(item.score, Some(8.367));
        assert_eq!(
            item.poster_url,
            Some("/tmdb-image/abc.jpg".to_string())
        );
    }

    #[test]
    fn maps_tv_search_result() {
        let json = serde_json::json!({
            "id": 1399,
            "name": "Game of Thrones",
            "original_name": "Game of Thrones",
            "poster_path": "/poster.jpg",
            "vote_average": 8.4,
            "vote_count": 10000,
            "overview": "Nine noble families fight for control.",
        });
        let item = map_search_result(&json, "series").unwrap();
        assert_eq!(item.title, "Game of Thrones");
        assert_eq!(item.external_id, "1399");
        assert_eq!(item.score, Some(8.4));
    }

    #[test]
    fn maps_search_result_without_poster() {
        let json = serde_json::json!({
            "id": 123,
            "name": "No Poster Show",
            "vote_average": 5.0,
            "vote_count": 100,
        });
        let item = map_search_result(&json, "series").unwrap();
        assert_eq!(item.title, "No Poster Show");
        assert_eq!(item.poster_url, None);
    }

    #[test]
    fn maps_search_result_uses_title_over_name() {
        let json = serde_json::json!({
            "id": 456,
            "title": "The Movie",
            "name": "The Movie (TV)",
            "overview": "A test movie.",
        });
        let item = map_search_result(&json, "movie").unwrap();
        assert_eq!(item.title, "The Movie");
    }

    #[test]
    fn parses_tv_seasons_list() {
        let json = r#"{
            "id": 1399,
            "name": "Game of Thrones",
            "number_of_seasons": 8,
            "number_of_episodes": 73,
            "seasons": [
                {
                    "season_number": 1,
                    "name": "Season 1",
                    "episode_count": 10,
                    "air_date": "2011-04-17",
                    "overview": "First season",
                    "poster_path": "/poster1.jpg"
                },
                {
                    "season_number": 2,
                    "name": "Season 2",
                    "episode_count": 10,
                    "air_date": null,
                    "overview": "",
                    "poster_path": null
                }
            ]
        }"#;
        let details: TmdbDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.seasons.len(), 2);
        assert_eq!(details.seasons[0].season_number, 1);
        assert_eq!(details.seasons[0].name, "Season 1");
        assert_eq!(details.seasons[0].episode_count, 10);
        assert_eq!(details.seasons[0].air_date.as_deref(), Some("2011-04-17"));
        assert_eq!(details.seasons[0].overview.as_deref(), Some("First season"));
        assert!(details.seasons[0].poster_path.is_some());
        assert_eq!(details.seasons[1].season_number, 2);
        assert!(details.seasons[1].air_date.is_none());
        assert!(details.seasons[1].poster_path.is_none());
    }

    #[test]
    fn parses_season_episodes() {
        let json = r#"{
            "_id": "5256c89f19c2956ff6046e77",
            "air_date": "2011-04-17",
            "episodes": [
                {
                    "air_date": "2011-04-17",
                    "episode_number": 1,
                    "id": 63056,
                    "name": "Winter Is Coming",
                    "overview": "Prologue.",
                    "runtime": 62,
                    "season_number": 1,
                    "still_path": "/still1.jpg",
                    "vote_average": 8.1
                },
                {
                    "air_date": "2011-04-24",
                    "episode_number": 2,
                    "id": 63057,
                    "name": "The Kingsroad",
                    "overview": "Catelyn receives a message.",
                    "runtime": 56,
                    "season_number": 1,
                    "still_path": null,
                    "vote_average": 8.0
                }
            ],
            "name": "Season 1",
            "season_number": 1
        }"#;
        let detail: TmdbSeasonDetail = serde_json::from_str(json).unwrap();
        assert_eq!(detail.episodes.len(), 2);
        assert_eq!(detail.episodes[0].episode_number, 1);
        assert_eq!(detail.episodes[0].name.as_deref(), Some("Winter Is Coming"));
        assert!(detail.episodes[0].still_path.is_some());
        assert_eq!(detail.episodes[0].runtime, Some(62));
        assert_eq!(detail.episodes[1].episode_number, 2);
        assert!(detail.episodes[1].still_path.is_none());
        assert_eq!(detail.episodes[1].runtime, Some(56));
    }

    #[test]
    fn parses_episode_with_minimal_fields() {
        let json = r#"{
            "episode_number": 1,
            "name": null,
            "overview": null,
            "still_path": null,
            "air_date": null,
            "runtime": null
        }"#;
        let ep: TmdbEpisodeInfo = serde_json::from_str(json).unwrap();
        assert_eq!(ep.episode_number, 1);
        assert!(ep.name.is_none());
        assert!(ep.overview.is_none());
        assert!(ep.still_path.is_none());
        assert!(ep.air_date.is_none());
        assert!(ep.runtime.is_none());
    }

    #[test]
    fn parses_season_episodes_empty() {
        let json = r#"{"episodes": []}"#;
        let detail: TmdbSeasonDetail = serde_json::from_str(json).unwrap();
        assert!(detail.episodes.is_empty());
    }
}
