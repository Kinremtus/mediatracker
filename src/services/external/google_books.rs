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
#[allow(dead_code)]
struct GoogleBooksVolumeInfo {
    title: String,
    authors: Option<Vec<String>>,
    publisher: Option<String>,
    published_date: Option<String>,
    description: Option<String>,
    page_count: Option<i32>,
    image_links: Option<GoogleBooksImageLinks>,
    average_rating: Option<f64>,
    ratings_count: Option<i32>,
    categories: Option<Vec<String>>,
    language: Option<String>,
    print_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleBooksImageLinks {
    thumbnail: Option<String>,
    small_thumbnail: Option<String>,
}

fn parse_year(s: Option<&str>) -> Option<i16> {
    s.and_then(|s| {
        // "1999" или "1999-05-15" — берём первые 4 цифры
        let year_str: String = s.chars().take_while(|c| c.is_ascii_digit()).take(4).collect();
        year_str.parse::<i16>().ok()
    })
}

fn parse_published_date(s: Option<&str>) -> Option<chrono::NaiveDate> {
    let s = s?;
    // YYYY-MM-DD
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    // YYYY-MM
    if let Ok(d) = chrono::NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d") {
        return Some(d);
    }
    // YYYY
    if let Ok(y) = s.parse::<i32>() {
        if let Some(d) = chrono::NaiveDate::from_ymd_opt(y, 1, 1) {
            return Some(d);
        }
    }
    None
}

fn map_item(r: GoogleBooksItem) -> CreateMediaItem {
    let authors = r.volume_info.authors.clone().unwrap_or_default();
    let title_with_author = if authors.is_empty() {
        r.volume_info.title.clone()
    } else {
        format!("{} — {}", r.volume_info.title, authors.join(", "))
    };

    let poster_url = r
        .volume_info
        .image_links
        .as_ref()
        .and_then(|img| img.thumbnail.clone().or(img.small_thumbnail.clone()));

    let year = parse_year(r.volume_info.published_date.as_deref());
    let aired_from = parse_published_date(r.volume_info.published_date.as_deref());

    let mut details = serde_json::Map::new();
    if let Some(lang) = r.volume_info.language.as_ref() {
        details.insert("language".to_string(), serde_json::Value::String(lang.clone()));
    }
    if let Some(pt) = r.volume_info.print_type.as_ref() {
        details.insert("print_type".to_string(), serde_json::Value::String(pt.clone()));
    }

    CreateMediaItem {
        provider: "google_books".to_string(),
        external_id: r.id,
        media_type: "book".to_string(),
        title: title_with_author,
        title_english: None,
        title_native: None,
        title_russian: None,
        poster_url,
        episodes: None,
        description: r.volume_info.description,
        status: None,
        score: r.volume_info.average_rating,
        is_tracked: false,
        mal_id: None,
        comparison_key: Some(r.volume_info.title),
        format_type: r.volume_info.print_type.or(Some("Book".to_string())),
        details: if details.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(details))
        },
        chapters: None,
        volumes: None,
        pages: r.volume_info.page_count,
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
        rating_votes: r.volume_info.ratings_count,
        authors,
        artists: Vec::new(),
        studios: Vec::new(),
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers: r.volume_info.publisher.map(|p| vec![p]).unwrap_or_default(),
        serialized_in: Vec::new(),
        networks: Vec::new(),
        platforms: Vec::new(),
        genres: r.volume_info.categories.unwrap_or_default(),
        themes: Vec::new(),
        demographics: Vec::new(),
        categories: Vec::new(),
    }
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
        Ok(results.items.unwrap_or_default().into_iter().map(map_item).collect())
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let url = format!("{}/volumes/{}", BASE_URL, id);
        let response = self.client.get(&url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Google Books details failed: {}", response.status());
        }
        let r: GoogleBooksItem = response.json().await?;
        Ok(map_item(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_book_details() {
        let json = r#"{
            "id": "abc123",
            "volume_info": {
                "title": "Clean Code",
                "authors": ["Robert C. Martin"],
                "publisher": "Prentice Hall",
                "published_date": "2008-08-01",
                "description": "...",
                "page_count": 464,
                "image_links": {"thumbnail": "https://..."},
                "average_rating": 4.2,
                "ratings_count": 5000,
                "categories": ["Computers"],
                "language": "en",
                "print_type": "BOOK"
            }
        }"#;
        let item: GoogleBooksItem = serde_json::from_str(json).unwrap();
        let mapped = map_item(item);
        assert!(mapped.title.starts_with("Clean Code"));
        assert_eq!(mapped.pages, Some(464));
        assert_eq!(mapped.year, Some(2008));
        assert!(mapped.authors.contains(&"Robert C. Martin".to_string()));
        assert!(mapped.publishers.contains(&"Prentice Hall".to_string()));
        assert!(mapped.genres.contains(&"Computers".to_string()));
        assert_eq!(mapped.rating_votes, Some(5000));
        let details = mapped.details.expect("details");
        assert_eq!(details.get("language").and_then(|v| v.as_str()), Some("en"));
    }
}
