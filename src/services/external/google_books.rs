use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://www.googleapis.com/books/v1";

#[derive(Debug, Deserialize)]
struct GoogleBooksResponse {
    items: Option<Vec<GoogleBooksItem>>,
}

#[derive(Debug, Deserialize)]
struct GoogleBooksItem {
    id: String,
    volume_info: GoogleBooksVolumeInfo,
}

#[derive(Debug, Deserialize)]
struct GoogleBooksVolumeInfo {
    title: String,
    authors: Option<Vec<String>>,
    publisher: Option<String>,
    published_date: Option<String>,
    description: Option<String>,
    page_count: Option<i32>,
    image_links: Option<GoogleBooksImageLinks>,
    average_rating: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct GoogleBooksImageLinks {
    thumbnail: Option<String>,
}

#[derive(Clone)]
pub struct GoogleBooksService {
    client: Client,
}

impl GoogleBooksService {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/volumes", BASE_URL))?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("maxResults", "20");

        let response = self.client.get(url.as_str()).send().await?;
        let results: GoogleBooksResponse = response.json().await?;

        let items = results
            .items
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                let authors = r.volume_info.authors.unwrap_or_default();
                let author_str = if authors.is_empty() {
                    None
                } else {
                    Some(authors.join(", "))
                };

                let mut title = r.volume_info.title.clone();
                if let Some(ref author) = author_str {
                    title = format!("{} — {}", title, author);
                }

                CreateMediaItem {
                    provider: "google_books".to_string(),
                    external_id: r.id,
                    media_type: "book".to_string(),
                    title,
                    title_english: author_str,
                    title_native: None,
                    title_russian: None,
                    poster_url: r.volume_info.image_links.and_then(|img| img.thumbnail),
                    episodes: r.volume_info.page_count,
                    description: r.volume_info.description,
                    status: None,
                    score: r.volume_info.average_rating,
                    is_tracked: false,
                    mal_id: None,
                }
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/volumes/{}", BASE_URL, id);
        let response = self.client.get(&url).send().await?;
        let r: GoogleBooksItem = response.json().await?;

        let authors = r.volume_info.authors.unwrap_or_default();
        let author_str = if authors.is_empty() {
            None
        } else {
            Some(authors.join(", "))
        };

        let mut title = r.volume_info.title.clone();
        if let Some(ref author) = author_str {
            title = format!("{} — {}", title, author);
        }

        Ok(CreateMediaItem {
            provider: "google_books".to_string(),
            external_id: r.id,
            media_type: "book".to_string(),
            title,
            title_english: author_str,
            title_native: None,
            title_russian: None,
            poster_url: r.volume_info.image_links.and_then(|img| img.thumbnail),
            episodes: r.volume_info.page_count,
            description: r.volume_info.description,
            status: None,
            score: r.volume_info.average_rating,
            is_tracked: false,
            mal_id: None,
        })
    }
}
