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
    publisher: Option<Vec<String>>,
    subject: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OpenLibraryWork {
    title: Option<String>,
    description: Option<OpenLibraryDescription>,
    covers: Option<Vec<i64>>,
    subjects: Option<Vec<String>>,
    subject_people: Option<Vec<String>>,
    subject_places: Option<Vec<String>>,
    first_publish_date: Option<String>,
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

fn parse_year(s: Option<&str>) -> Option<i16> {
    s.and_then(|s| {
        let year_str: String = s.chars().take_while(|c| c.is_ascii_digit()).take(4).collect();
        year_str.parse::<i16>().ok()
    })
}

fn parse_published_date(s: Option<&str>) -> Option<chrono::NaiveDate> {
    let s = s?;
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    if let Ok(y) = s.parse::<i32>()
        && let Some(d) = chrono::NaiveDate::from_ymd_opt(y, 1, 1) {
            return Some(d);
        }
    None
}

fn map_search_doc(doc: OpenLibrarySearchDoc) -> Option<CreateMediaItem> {
    let key = doc.key?;
    let title = doc.title?;

    let olid = key.strip_prefix("/works/").unwrap_or(&key).to_string();

    let authors = doc.author_name.clone().unwrap_or_default();
    let title_with_author = if authors.is_empty() {
        title.clone()
    } else {
        format!("{} — {}", title, authors.join(", "))
    };

    let poster_url = doc.cover_i.map(|id| format!("{}/{}-M.jpg", COVERS_URL, id));
    let year = doc.first_publish_year.and_then(|y| i16::try_from(y).ok());

    Some(CreateMediaItem {
        provider: "openlibrary".to_string(),
        external_id: olid,
        media_type: "book".to_string(),
        title: title_with_author,
        title_english: None,
        title_native: None,
        title_russian: None,
        poster_url,
        episodes: None,
        description: None,
        status: None,
        score: None,
        is_tracked: false,
        mal_id: None,
        shikimori_id: None,
        comparison_key: Some(title),
        format_type: Some("Book".to_string()),
        details: None,
        chapters: None,
        volumes: None,
        pages: doc.number_of_pages_median,
        runtime_minutes: None,
        playtime_hours: None,
        year,
        aired_from: year.and_then(|y| chrono::NaiveDate::from_ymd_opt(y as i32, 1, 1)),
        aired_to: None,
        premiered_season: None,
        premiered_year: year,
        broadcast: None,
        completed: None,
        licensed: None,
        source: None,
        duration: None,
        rating: None,
        rating_votes: None,
        authors,
        artists: Vec::new(),
        studios: Vec::new(),
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers: doc.publisher.unwrap_or_default(),
        serialized_in: Vec::new(),
        networks: Vec::new(),
        platforms: Vec::new(),
        genres: doc.subject.unwrap_or_default(),
        themes: Vec::new(),
        demographics: Vec::new(),
        categories: Vec::new(),
    })
}

fn map_work(id: &str, work: OpenLibraryWork) -> Result<CreateMediaItem, anyhow::Error> {
    let title = work
        .title
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Work has no title"))?;
    let description = work.description.and_then(OpenLibraryDescription::into_string);

    let poster_url = work
        .covers
        .as_ref()
        .and_then(|c| c.first().map(|id| format!("{}/{}-M.jpg", COVERS_URL, id)));

    let year = parse_year(work.first_publish_date.as_deref());
    let aired_from = parse_published_date(work.first_publish_date.as_deref());

    let mut genres = work.subjects.clone().unwrap_or_default();
    if genres.is_empty() {
        genres = Vec::new();
    }
    let themes: Vec<String> = work.subject_people.clone().unwrap_or_default();
    let categories: Vec<String> = work.subject_places.clone().unwrap_or_default();

    Ok(CreateMediaItem {
        provider: "openlibrary".to_string(),
        external_id: id.to_string(),
        media_type: "book".to_string(),
        title,
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
        shikimori_id: None,
        comparison_key: None,
        format_type: Some("Book".to_string()),
        details: None,
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
        genres,
        themes,
        demographics: Vec::new(),
        categories,
    })
}

#[derive(Clone)]
pub struct OpenLibraryService {
    client: Client,
}

impl Default for OpenLibraryService {
    fn default() -> Self {
        Self::new()
    }
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
            .append_pair(
                "fields",
                "key,title,author_name,cover_i,number_of_pages_median,first_publish_year,publisher,subject",
            )
            .append_pair("limit", "20");

        let resp = self.client.get(url.as_str()).send().await?;
        let results: OpenLibrarySearchResponse = resp.json().await?;
        Ok(results
            .docs
            .unwrap_or_default()
            .into_iter()
            .filter_map(map_search_doc)
            .collect())
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/works/{}.json", BASE_URL, id);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("OpenLibrary details failed: {}", resp.status());
        }
        let work: OpenLibraryWork = resp.json().await?;
        map_work(id, work)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_doc() {
        let json = r#"{
            "key": "/works/OL45804W",
            "title": "The Lord of the Rings",
            "author_name": ["J. R. R. Tolkien"],
            "cover_i": 12345,
            "number_of_pages_median": 1200,
            "first_publish_year": 1954,
            "publisher": ["Allen & Unwin"],
            "subject": ["Fantasy", "Fiction"]
        }"#;
        let doc: OpenLibrarySearchDoc = serde_json::from_str(json).unwrap();
        let item = map_search_doc(doc).unwrap();
        assert!(item.title.contains("Lord of the Rings"));
        assert_eq!(item.pages, Some(1200));
        assert_eq!(item.year, Some(1954));
        assert!(item.publishers.contains(&"Allen & Unwin".to_string()));
        assert!(item.genres.contains(&"Fantasy".to_string()));
    }
}
