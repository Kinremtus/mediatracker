use reqwest::Client;
use serde::Deserialize;
use url::Url;

use crate::models::media_item::CreateMediaItem;

const BASE_URL: &str = "https://openlibrary.org";
const COVERS_URL: &str = "https://covers.openlibrary.org/b/id";

#[derive(Debug, Deserialize)]
struct OpenLibrarySearchResponse {
    docs: Option<Vec<OpenLibrarySearchDoc>>,
}

#[derive(Debug, Deserialize)]
struct OpenLibrarySearchDoc {
    key: Option<String>,
    title: Option<String>,
    author_name: Option<Vec<String>>,
    cover_i: Option<i64>,
    number_of_pages_median: Option<i32>,
    first_publish_year: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct OpenLibraryWork {
    title: Option<String>,
    description: Option<OpenLibraryDescription>,
    covers: Option<Vec<i64>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenLibraryDescription {
    Text(String),
    Obj(OpenLibraryDescObj),
}

#[derive(Debug, Deserialize)]
struct OpenLibraryDescObj {
    value: Option<String>,
}

impl OpenLibraryDescription {
    fn into_string(self) -> Option<String> {
        match self {
            OpenLibraryDescription::Text(s) => Some(s),
            OpenLibraryDescription::Obj(o) => o.value,
        }
    }
}

#[derive(Clone)]
pub struct OpenLibraryService {
    client: Client,
}

impl OpenLibraryService {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("MediaTracker/0.1 (https://github.com/Kinremtus/mediatracker)")
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse(&format!("{}/search.json", BASE_URL))?;
        url.query_pairs_mut()
            .append_pair("q", query)
            .append_pair("fields", "key,title,author_name,cover_i,number_of_pages_median,first_publish_year")
            .append_pair("limit", "20");

        let resp = self.client.get(url.as_str()).send().await?;
        let results: OpenLibrarySearchResponse = resp.json().await?;

        let items = results
            .docs
            .unwrap_or_default()
            .into_iter()
            .filter_map(|doc| {
                let key = doc.key?;
                let title = doc.title?;

                let olid = key.strip_prefix("/works/").unwrap_or(&key);

                let authors = doc.author_name.unwrap_or_default();
                let author_str = if authors.is_empty() {
                    None
                } else {
                    Some(authors.join(", "))
                };

                let mut full_title = title.clone();
                if let Some(ref author) = author_str {
                    full_title = format!("{} — {}", title, author);
                }

                let poster_url = doc.cover_i.map(|id| format!("{}/{}-M.jpg", COVERS_URL, id));

                Some(CreateMediaItem {
                    provider: "openlibrary".to_string(),
                    external_id: olid.to_string(),
                    media_type: "book".to_string(),
                    title: full_title,
                    title_english: author_str,
                    title_native: None,
                    title_russian: None,
                    poster_url,
                    episodes: doc.number_of_pages_median,
                    description: None,
                    status: None,
                    score: None,
                    is_tracked: false,
                    mal_id: None,
                    comparison_key: Some(title),
                })
            })
            .collect();

        Ok(items)
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/works/{}.json", BASE_URL, id);
        let resp = self.client.get(&url).send().await?;
        let work: OpenLibraryWork = resp.json().await?;

        let title = work.title.ok_or_else(|| anyhow::anyhow!("Work has no title"))?;

        let description = work.description.and_then(OpenLibraryDescription::into_string);

        let poster_url = work
            .covers
            .and_then(|c| c.first().map(|id| format!("{}/{}-M.jpg", COVERS_URL, id)));

        Ok(CreateMediaItem {
            provider: "openlibrary".to_string(),
            external_id: id.to_string(),
            media_type: "book".to_string(),
            title: title.clone(),
            title_english: None,
            title_native: None,
            title_russian: None,
            poster_url,
            episodes: None,
            description,
            status: None,
            score: None,
            is_tracked: false,
            mal_id: None,
            comparison_key: Some(title),
        })
    }
}
