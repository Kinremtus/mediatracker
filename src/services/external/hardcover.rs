use reqwest::Client;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::models::media_item::CreateMediaItem;

const API_URL: &str = "https://api.hardcover.app/v1/graphql";
const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Search query. NOTE: the `fields` argument must NOT be added — when present
/// Hardcover returns `results: null` (live-verified 2026-09-27).
const SEARCH_QUERY: &str = r#"query SearchBooks($query: String!, $queryType: String!, $perPage: Int!) { search(query: $query, query_type: $queryType, per_page: $perPage) { results } }"#;

/// Detail lookup by slug. The `books` type has no `language` field and rejects
/// `_ilike`/`_like`/`_regex` (HTTP 403); plain `_eq` is all we need.
const DETAILS_BY_SLUG_QUERY: &str = r#"query BookBySlug($slug: String!) { books(where: {slug: {_eq: $slug}}, limit: 1) { id title slug description image { url } release_year ratings_count rating book_series { position series { name } } contributions { contribution author { name } } } }"#;

/// Fallback detail lookup when `external_id` is the numeric Hardcover id
/// instead of a slug.
const DETAILS_BY_ID_QUERY: &str = r#"query BookById($id: Int!) { books(where: {id: {_eq: $id}}, limit: 1) { id title slug description image { url } release_year ratings_count rating book_series { position series { name } } contributions { contribution author { name } } } }"#;

/// GraphQL envelope (`data` holds the query result, mirroring AniList).
#[derive(Debug, Deserialize)]
struct HardcoverResponse<T> {
    data: T,
}

#[derive(Debug, Deserialize)]
struct SearchData {
    search: SearchResults,
}

#[derive(Debug, Deserialize)]
struct SearchResults {
    /// Typesense envelope; `null` when the query is rejected.
    #[serde(default)]
    results: Option<TypesenseResults>,
}

#[derive(Debug, Deserialize)]
struct TypesenseResults {
    #[serde(default)]
    hits: Option<Vec<SearchHit>>,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    #[serde(default)]
    document: Option<SearchDocument>,
}

/// A Typesense `hit.document`. Only fields we actually map are declared;
/// unknown fields are ignored by serde.
#[derive(Debug, Deserialize)]
struct SearchDocument {
    id: String,
    title: String,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    image: Option<SearchImage>,
    #[serde(default)]
    author_names: Option<Vec<String>>,
    #[serde(default)]
    pages: Option<i32>,
    #[serde(default)]
    release_year: Option<i32>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(default)]
    ratings_count: Option<i32>,
    #[serde(default)]
    series_names: Option<Vec<String>>,
    #[serde(default)]
    genres: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct SearchImage {
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BooksData {
    #[serde(default)]
    books: Vec<BookDetail>,
}

#[derive(Debug, Deserialize)]
struct BookDetail {
    #[serde(default)]
    id: Option<i64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    image: Option<SearchImage>,
    #[serde(default)]
    release_year: Option<i32>,
    #[serde(default)]
    ratings_count: Option<i32>,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(default)]
    book_series: Vec<BookSeries>,
    #[serde(default)]
    contributions: Vec<Contribution>,
}

#[derive(Debug, Deserialize)]
struct BookSeries {
    #[serde(default)]
    series: Option<Series>,
}

#[derive(Debug, Deserialize)]
struct Series {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Contribution {
    #[serde(default)]
    author: Option<Author>,
}

#[derive(Debug, Deserialize)]
struct Author {
    #[serde(default)]
    name: Option<String>,
}

/// Trim whitespace, drop surrounding quotes and an optional `Bearer ` prefix
/// (mirrors bindery's NormalizeAPIToken).
fn normalize_token(raw: &str) -> String {
    let trimmed = raw.trim().trim_matches('"').trim();
    trimmed
        .strip_prefix("Bearer ")
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

/// `YYYY-MM-DD` -> `YYYY-MM` -> `YYYY` (same tolerance as google_books.rs).
fn parse_release_date(s: Option<&str>) -> Option<chrono::NaiveDate> {
    let s = s?;
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d") {
        return Some(d);
    }
    if let Ok(y) = s.parse::<i32>()
        && let Some(d) = chrono::NaiveDate::from_ymd_opt(y, 1, 1)
    {
        return Some(d);
    }
    None
}

/// `"Title — Author, Author"`, or just `"Title"` when there are no authors
/// (same convention as google_books.rs).
fn title_with_authors(title: &str, authors: &[String]) -> String {
    if authors.is_empty() {
        title.to_string()
    } else {
        format!("{title} — {}", authors.join(", "))
    }
}

/// Flatten the Typesense envelope into mapped items, tolerating missing
/// `results`, missing `hits` and hits without a `document`.
fn extract_hits(data: SearchData) -> Vec<CreateMediaItem> {
    data.search
        .results
        .and_then(|r| r.hits)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|hit| hit.document)
        .map(map_search_document)
        .collect()
}

fn map_search_document(doc: SearchDocument) -> CreateMediaItem {
    let authors = doc.author_names.unwrap_or_default();
    let title = title_with_authors(&doc.title, &authors);
    // D3: prefer the slug, fall back to the numeric id as a string. An empty
    // slug string must not win over the id.
    let external_id = doc.slug.filter(|s| !s.is_empty()).unwrap_or(doc.id);
    let aired_from = parse_release_date(doc.release_date.as_deref());
    let year = doc
        .release_year
        .and_then(|y| i16::try_from(y).ok())
        .filter(|y| *y > 0);

    CreateMediaItem {
        provider: "hardcover".to_string(),
        external_id,
        media_type: "book".to_string(),
        title,
        poster_url: doc.image.and_then(|i| i.url),
        description: doc.description,
        score: doc.rating,
        comparison_key: Some(doc.title),
        format_type: Some("Book".to_string()),
        pages: doc.pages,
        year,
        aired_from,
        rating_votes: doc.ratings_count,
        authors,
        serialized_in: doc.series_names.unwrap_or_default(),
        genres: doc.genres.unwrap_or_default(),
        ..Default::default()
    }
}

fn map_book_detail(book: BookDetail) -> CreateMediaItem {
    let authors: Vec<String> = book
        .contributions
        .iter()
        .filter_map(|c| c.author.as_ref())
        .filter_map(|a| a.name.clone())
        .collect();
    let title = title_with_authors(book.title.as_deref().unwrap_or_default(), &authors);
    let external_id = book
        .slug
        .filter(|s| !s.is_empty())
        .or_else(|| book.id.map(|id| id.to_string()))
        .unwrap_or_default();
    let serialized_in: Vec<String> = book
        .book_series
        .iter()
        .filter_map(|bs| bs.series.as_ref())
        .filter_map(|s| s.name.clone())
        .collect();

    CreateMediaItem {
        provider: "hardcover".to_string(),
        external_id,
        media_type: "book".to_string(),
        title,
        poster_url: book.image.and_then(|i| i.url),
        description: book.description,
        score: book.rating,
        comparison_key: book.title,
        format_type: Some("Book".to_string()),
        year: book.release_year.and_then(|y| i16::try_from(y).ok()),
        rating_votes: book.ratings_count,
        authors,
        serialized_in,
        ..Default::default()
    }
}

#[derive(Clone)]
pub struct HardcoverService {
    client: Client,
    api_token: String,
    last_request: Arc<Mutex<Option<Instant>>>,
}

impl Default for HardcoverService {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl HardcoverService {
    pub fn new(api_token: String) -> Self {
        Self {
            client: super::http_client(),
            api_token: normalize_token(&api_token),
            last_request: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.api_token.is_empty()
    }

    /// Hardcover allows ~60 req/min; keep at least 1s between requests.
    async fn throttle(&self) {
        let mut last = self.last_request.lock().await;
        if let Some(prev) = *last {
            let elapsed = prev.elapsed();
            if elapsed < MIN_INTERVAL {
                tokio::time::sleep(MIN_INTERVAL - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    /// POST a GraphQL document with the given `variables`; non-2xx (401/403/
    /// 429) becomes an error via `error_for_status`. A 404 is classified as a
    /// [`super::NotFoundError`] so callers can tell "gone" from "unavailable".
    async fn post_graphql<T: serde::de::DeserializeOwned>(
        &self,
        query: &str,
        variables: serde_json::Value,
        not_found_context: &str,
    ) -> Result<HardcoverResponse<T>, anyhow::Error> {
        let response = self
            .client
            .post(API_URL)
            .bearer_auth(&self.api_token)
            .json(&serde_json::json!({
                "query": query,
                "variables": variables,
            }))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(
                crate::services::external::NotFoundError(not_found_context.to_string()).into(),
            );
        }
        let resp = response.error_for_status()?;
        let parsed: HardcoverResponse<T> = resp.json().await?;
        Ok(parsed)
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        if !self.is_configured() {
            tracing::warn!("Hardcover API key not configured; skipping search");
            return Ok(Vec::new());
        }

        self.throttle().await;
        let parsed: HardcoverResponse<SearchData> = self
            .post_graphql(
                SEARCH_QUERY,
                serde_json::json!({
                    "query": query,
                    "queryType": "Book",
                    "perPage": 10,
                }),
                "hardcover search",
            )
            .await?;

        Ok(extract_hits(parsed.data))
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        if !self.is_configured() {
            anyhow::bail!("Hardcover API key not configured");
        }

        self.throttle().await;
        let not_found_context = format!("hardcover {id}");
        let parsed: HardcoverResponse<BooksData> = self
            .post_graphql(
                DETAILS_BY_SLUG_QUERY,
                serde_json::json!({ "slug": id }),
                &not_found_context,
            )
            .await?;
        if let Some(book) = parsed.data.books.into_iter().next() {
            return Ok(map_book_detail(book));
        }

        // Fallback: the caller may have passed the numeric Hardcover id.
        if let Ok(numeric_id) = id.parse::<i32>() {
            self.throttle().await;
            let parsed: HardcoverResponse<BooksData> = self
                .post_graphql(
                    DETAILS_BY_ID_QUERY,
                    serde_json::json!({ "id": numeric_id }),
                    &not_found_context,
                )
                .await?;
            if let Some(book) = parsed.data.books.into_iter().next() {
                return Ok(map_book_detail(book));
            }
        }

        Err(
            crate::services::external::NotFoundError(format!("Hardcover book not found: {id}"))
                .into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_bearer_token() {
        assert_eq!(normalize_token("abc123"), "abc123");
        assert_eq!(normalize_token("  abc123  "), "abc123");
        assert_eq!(normalize_token("Bearer abc123"), "abc123");
        assert_eq!(normalize_token("\"Bearer abc123\""), "abc123");
        assert_eq!(normalize_token(""), "");
    }

    #[test]
    fn parses_search_envelope_and_maps_fields() {
        let json = r#"{
            "data": {
                "search": {
                    "results": {
                        "found": 1,
                        "hits": [
                            {
                                "document": {
                                    "id": "312460",
                                    "title": "Dune",
                                    "slug": "dune",
                                    "subtitle": "Dune Chronicles, Book 1",
                                    "description": "Set on the desert planet Arrakis.",
                                    "image": {"url": "https://assets.hardcover.app/dune.jpg"},
                                    "author_names": ["Frank Herbert"],
                                    "pages": 658,
                                    "release_year": 1965,
                                    "release_date": "1965-06-01",
                                    "rating": 4.3,
                                    "ratings_count": 12345,
                                    "series_names": ["Dune"],
                                    "genres": ["Science Fiction"]
                                }
                            }
                        ]
                    }
                }
            }
        }"#;
        let parsed: HardcoverResponse<SearchData> = serde_json::from_str(json).unwrap();
        let items = extract_hits(parsed.data);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.provider, "hardcover");
        assert_eq!(item.external_id, "dune");
        assert_eq!(item.media_type, "book");
        assert_eq!(item.title, "Dune — Frank Herbert");
        assert_eq!(item.comparison_key.as_deref(), Some("Dune"));
        assert_eq!(
            item.poster_url.as_deref(),
            Some("https://assets.hardcover.app/dune.jpg")
        );
        assert_eq!(item.score, Some(4.3));
        assert_eq!(item.rating_votes, Some(12345));
        assert_eq!(item.pages, Some(658));
        assert_eq!(item.year, Some(1965));
        assert_eq!(item.aired_from, chrono::NaiveDate::from_ymd_opt(1965, 6, 1));
        assert_eq!(item.authors, vec!["Frank Herbert".to_string()]);
        assert_eq!(item.serialized_in, vec!["Dune".to_string()]);
        assert_eq!(item.genres, vec!["Science Fiction".to_string()]);
        assert_eq!(item.format_type.as_deref(), Some("Book"));
    }

    #[test]
    fn null_results_yields_empty() {
        let json = r#"{"data":{"search":{"results":null}}}"#;
        let parsed: HardcoverResponse<SearchData> = serde_json::from_str(json).unwrap();
        assert!(extract_hits(parsed.data).is_empty());
    }

    #[test]
    fn missing_hits_yields_empty() {
        let json = r#"{"data":{"search":{"results":{"found":0}}}}"#;
        let parsed: HardcoverResponse<SearchData> = serde_json::from_str(json).unwrap();
        assert!(extract_hits(parsed.data).is_empty());
    }

    #[test]
    fn search_document_without_slug_falls_back_to_id() {
        let json = r#"{
            "data": {
                "search": {
                    "results": {
                        "hits": [
                            {"document": {"id": "99", "title": "Solo"}}
                        ]
                    }
                }
            }
        }"#;
        let parsed: HardcoverResponse<SearchData> = serde_json::from_str(json).unwrap();
        let items = extract_hits(parsed.data);
        assert_eq!(items[0].external_id, "99");
        assert_eq!(items[0].title, "Solo");
        assert_eq!(items[0].comparison_key.as_deref(), Some("Solo"));
    }

    #[test]
    fn parses_details_and_maps_authors_and_series() {
        let json = r#"{
            "data": {
                "books": [
                    {
                        "id": 312460,
                        "title": "Dune",
                        "slug": "dune",
                        "description": "Set on Arrakis.",
                        "image": {"url": "https://assets.hardcover.app/dune.jpg"},
                        "release_year": 1965,
                        "ratings_count": 12345,
                        "rating": 4.3,
                        "book_series": [{"position": 1, "series": {"name": "Dune"}}],
                        "contributions": [
                            {"contribution": "Author", "author": {"name": "Frank Herbert"}}
                        ]
                    }
                ]
            }
        }"#;
        let parsed: HardcoverResponse<BooksData> = serde_json::from_str(json).unwrap();
        let book = parsed.data.books.into_iter().next().unwrap();
        let item = map_book_detail(book);
        assert_eq!(item.provider, "hardcover");
        assert_eq!(item.external_id, "dune");
        assert_eq!(item.title, "Dune — Frank Herbert");
        assert_eq!(item.year, Some(1965));
        assert_eq!(item.rating_votes, Some(12345));
        assert_eq!(item.authors, vec!["Frank Herbert".to_string()]);
        assert_eq!(item.serialized_in, vec!["Dune".to_string()]);
    }
}
