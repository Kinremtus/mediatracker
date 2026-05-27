use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://api.themoviedb.org/3";

#[derive(Debug, Deserialize)]
struct TmdbSearchResult {
    id: i64,
    title: Option<String>,
    name: Option<String>,
    poster_path: Option<String>,
    media_type: Option<String>,
    vote_average: Option<f64>,
    overview: Option<String>,
    genre_ids: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
struct TmdbDetails {
    id: i64,
    title: Option<String>,
    name: Option<String>,
    poster_path: Option<String>,
    vote_average: Option<f64>,
    overview: Option<String>,
    number_of_seasons: Option<i32>,
    number_of_episodes: Option<i32>,
    status: Option<String>,
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
        let items = results["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                let media_type = r["media_type"].as_str()?;
                if media_type != "movie" && media_type != "tv" {
                    return None;
                }

                let id = r["id"].as_i64()?;
                let title = r["title"].as_str().or(r["name"].as_str())?.to_string();
                let poster_path = r["poster_path"].as_str();
                let poster_url = poster_path.map(|p| format!("/tmdb-image/{}", p.trim_start_matches('/')));
                let score = r["vote_average"].as_f64();
                let description = r["overview"].as_str().map(String::from);

                Some(CreateMediaItem {
                    provider: "tmdb".to_string(),
                    external_id: id.to_string(),
                    media_type: if media_type == "movie" {
                        "movie".to_string()
                    } else {
                        "series".to_string()
                    },
                    title,
                    title_english: None,
                    title_native: None,
                    title_russian: None,
                    poster_url,
                    episodes: None,
                    description,
                    status: None,
                    score,
                    is_tracked: false,
                    mal_id: None,
                })
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

        let items = results["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                // Post-filter by genre_ids if genre_id was specified
                if let Some(gid) = genre_id {
                    let genre_ids = r["genre_ids"].as_array();
                    if let Some(ids) = genre_ids {
                        if !ids.iter().any(|id| id.as_i64() == Some(gid)) {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }

                let id = r["id"].as_i64()?;
                let title = r["title"].as_str()?.to_string();
                let poster_path = r["poster_path"].as_str();
                let poster_url = poster_path.map(|p| format!("/tmdb-image/{}", p.trim_start_matches('/')));
                let score = r["vote_average"].as_f64();
                let description = r["overview"].as_str().map(String::from);

                Some(CreateMediaItem {
                    provider: "tmdb".to_string(),
                    external_id: id.to_string(),
                    media_type: "movie".to_string(),
                    title,
                    title_english: None,
                    title_native: None,
                    title_russian: None,
                    poster_url,
                    episodes: None,
                    description,
                    status: None,
                    score,
                    is_tracked: false,
                    mal_id: None,
                })
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

        let items = results["results"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|r| {
                // Post-filter by genre_ids if genre_id was specified
                if let Some(gid) = genre_id {
                    let genre_ids = r["genre_ids"].as_array();
                    if let Some(ids) = genre_ids {
                        if !ids.iter().any(|id| id.as_i64() == Some(gid)) {
                            return None;
                        }
                    } else {
                        return None;
                    }
                }

                let id = r["id"].as_i64()?;
                let title = r["name"].as_str()?.to_string();
                let poster_path = r["poster_path"].as_str();
                let poster_url = poster_path.map(|p| format!("/tmdb-image/{}", p.trim_start_matches('/')));
                let score = r["vote_average"].as_f64();
                let description = r["overview"].as_str().map(String::from);

                Some(CreateMediaItem {
                    provider: "tmdb".to_string(),
                    external_id: id.to_string(),
                    media_type: "series".to_string(),
                    title,
                    title_english: None,
                    title_native: None,
                    title_russian: None,
                    poster_url,
                    episodes: None,
                    description,
                    status: None,
                    score,
                    is_tracked: false,
                    mal_id: None,
                })
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(
        &self,
        id: &str,
        media_type: &str,
    ) -> Result<CreateMediaItem, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/{}/{}", BASE_URL, media_type, id))?;
        url.query_pairs_mut()
            .append_pair("api_key", &self.api_key)
            .append_pair("language", "ru-RU");

        let response = self.client.get(url.as_str()).send().await?;

        let r: TmdbDetails = response.json().await?;
        let poster_url = r.poster_path.map(|p| format!("/tmdb-image/{}", p.trim_start_matches('/')));
        let title = r.title.unwrap_or(r.name.unwrap_or_default());

        Ok(CreateMediaItem {
            provider: "tmdb".to_string(),
            external_id: r.id.to_string(),
            media_type: media_type.to_string(),
            title,
            title_english: None,
            title_native: None,
            title_russian: None,
            poster_url,
            episodes: r.number_of_episodes.or(r.number_of_seasons),
            description: r.overview,
            status: r.status,
            score: r.vote_average,
            is_tracked: false,
            mal_id: None,
        })
    }
}
