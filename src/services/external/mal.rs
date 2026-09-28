use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::models::media_item::CreateMediaItem;
use crate::utils::clean_description;

/// MyAnimeList data. Tenrai is a Jikan-v4-compatible proxy and is the
/// primary host; Jikan is kept as a fallback while it is still online.
/// Both return the same schema, so parsing is shared.
const TENRAI_BASE_URL: &str = "https://api.tenrai.org/v1";
const JIKAN_BASE_URL: &str = "https://api.jikan.moe/v4";
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
struct MalMangaSearchResponse {
    data: Vec<MalManga>,
}

#[derive(Debug, Deserialize)]
struct MalMangaResponse {
    data: MalMangaFull,
}

#[derive(Debug, Deserialize)]
struct MalManga {
    mal_id: i64,
    title: String,
    #[serde(default)]
    title_english: Option<String>,
    #[serde(default)]
    title_japanese: Option<String>,
    #[serde(default)]
    images: Option<MalImages>,
    #[serde(default)]
    chapters: Option<i32>,
    #[serde(default)]
    volumes: Option<i32>,
    #[serde(default)]
    synopsis: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(rename = "type", default)]
    manga_type: Option<String>,
    #[serde(default)]
    genres: Option<Vec<MalNamed>>,
    #[serde(default)]
    themes: Option<Vec<MalNamed>>,
    #[serde(default)]
    demographics: Option<Vec<MalNamed>>,
}

#[derive(Debug, Deserialize)]
struct MalMangaFull {
    mal_id: i64,
    title: String,
    #[serde(default)]
    title_english: Option<String>,
    #[serde(default)]
    title_japanese: Option<String>,
    #[serde(default)]
    images: Option<MalImages>,
    #[serde(default)]
    chapters: Option<i32>,
    #[serde(default)]
    volumes: Option<i32>,
    #[serde(default)]
    synopsis: Option<String>,
    #[serde(default)]
    score: Option<f64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(rename = "type", default)]
    manga_type: Option<String>,
    #[serde(default)]
    genres: Option<Vec<MalNamed>>,
    #[serde(default)]
    themes: Option<Vec<MalNamed>>,
    #[serde(default)]
    demographics: Option<Vec<MalNamed>>,
    #[serde(default)]
    published: Option<MalAired>,
    #[serde(default)]
    authors: Option<Vec<MalNamed>>,
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
    /// Human-readable duration, e.g. "24 min. per ep.".
    ///
    /// Tenrai returns an integer number of seconds (`1440`), Jikan omits the
    /// field, and older payloads used a string. `deserialize_duration_lenient`
    /// accepts all three; numbers become `"{n} sec"`, which
    /// `parse_duration_to_minutes()` already understands (ceil to minutes).
    #[serde(default, deserialize_with = "deserialize_duration_lenient")]
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

/// Accepts `duration` as a JSON string, an integer number of seconds, or null.
/// Integer seconds are normalized to a `"{n} sec"` string so the existing
/// `parse_duration_to_minutes()` parser is reused unchanged.
fn deserialize_duration_lenient<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s),
        Some(serde_json::Value::Number(n)) => n.as_i64().map(|secs| format!("{secs} sec")),
        Some(other) => {
            return Err(D::Error::custom(format!(
                "unexpected `duration` value: {other}"
            )));
        }
    })
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

    if found { Some(total) } else { None }
}

/// Берёт целое число, стоящее непосредственно перед `unit` в строке.
/// Например: "1 hr. 52 min." + "hr" → 1, "1 hr. 52 min." + "min" → 52.
fn extract_number_before(s: &str, unit: &str) -> Option<i32> {
    let pos = s.find(unit)?;
    let before = &s[..pos];
    // Разбиваем по не-цифровым символам и берём последний непустой кусок.
    let last = before
        .split(|c: char| !c.is_ascii_digit())
        .rfind(|s| !s.is_empty())?;
    last.parse().ok()
}

fn map_full(anime: MalAnimeFull) -> CreateMediaItem {
    let comparison_key = anime
        .title_english
        .clone()
        .unwrap_or_else(|| anime.title.clone());

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
    if let Some(a) = anime.aired.as_ref()
        && let Some(s) = a.string.as_ref()
    {
        details.insert(
            "aired_string".to_string(),
            serde_json::Value::String(s.clone()),
        );
    }
    if let Some(b) = anime.broadcast.as_ref()
        && let Some(s) = b.string.as_ref()
    {
        details.insert(
            "broadcast".to_string(),
            serde_json::Value::String(s.clone()),
        );
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
        seasons: None,
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
        runtime_minutes: anime
            .duration
            .as_deref()
            .and_then(parse_duration_to_minutes),
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
    let comparison_key = item
        .title_english
        .clone()
        .unwrap_or_else(|| item.title.clone());
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
        seasons: None,
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

fn map_manga_search(manga: MalManga) -> CreateMediaItem {
    let comparison_key = manga
        .title_english
        .clone()
        .unwrap_or_else(|| manga.title.clone());

    CreateMediaItem {
        provider: "mal".to_string(),
        external_id: manga.mal_id.to_string(),
        media_type: "manga".to_string(),
        title: manga.title,
        title_english: manga.title_english,
        title_native: manga.title_japanese,
        title_russian: None,
        poster_url: poster_url(&manga.images),
        episodes: None,
        seasons: None,
        description: clean_description(manga.synopsis),
        status: manga.status,
        score: manga.score,
        is_tracked: false,
        mal_id: Some(manga.mal_id),
        shikimori_id: None,
        comparison_key: Some(comparison_key),
        format_type: manga.manga_type,
        details: None,
        chapters: manga.chapters,
        volumes: manga.volumes,
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
        genres: extract_names(&manga.genres),
        themes: extract_names(&manga.themes),
        demographics: extract_names(&manga.demographics),
        categories: Vec::new(),
    }
}

fn map_manga_full(manga: MalMangaFull) -> CreateMediaItem {
    let comparison_key = manga
        .title_english
        .clone()
        .unwrap_or_else(|| manga.title.clone());

    let aired_from = manga
        .published
        .as_ref()
        .and_then(|p| p.from.as_deref())
        .and_then(parse_date);
    let aired_to = manga
        .published
        .as_ref()
        .and_then(|p| p.to.as_deref())
        .and_then(parse_date);

    let year_i16: Option<i16> =
        aired_from.and_then(|d| d.format("%Y").to_string().parse::<i16>().ok());
    let premiered_year_i16 = year_i16;

    CreateMediaItem {
        provider: "mal".to_string(),
        external_id: manga.mal_id.to_string(),
        media_type: "manga".to_string(),
        title: manga.title,
        title_english: manga.title_english,
        title_native: manga.title_japanese,
        title_russian: None,
        poster_url: poster_url(&manga.images),
        episodes: None,
        seasons: None,
        description: clean_description(manga.synopsis),
        status: manga.status,
        score: manga.score,
        is_tracked: false,
        mal_id: Some(manga.mal_id),
        shikimori_id: None,
        comparison_key: Some(comparison_key),
        format_type: manga.manga_type,
        details: None,
        chapters: manga.chapters,
        volumes: manga.volumes,
        pages: None,
        runtime_minutes: None,
        playtime_hours: None,
        year: year_i16,
        aired_from,
        aired_to,
        premiered_season: None,
        premiered_year: premiered_year_i16,
        broadcast: None,
        completed: None,
        licensed: None,
        source: None,
        duration: None,
        rating: None,
        rating_votes: None,
        authors: extract_names(&manga.authors),
        artists: Vec::new(),
        studios: Vec::new(),
        producers: Vec::new(),
        licensors: Vec::new(),
        publishers: Vec::new(),
        serialized_in: Vec::new(),
        networks: Vec::new(),
        platforms: Vec::new(),
        genres: extract_names(&manga.genres),
        themes: extract_names(&manga.themes),
        demographics: extract_names(&manga.demographics),
        categories: Vec::new(),
    }
}

#[derive(Clone)]
pub struct MalService {
    client: Client,
    /// Ordered by priority: primary first, fallback second.
    base_urls: [&'static str; 2],
}

impl Default for MalService {
    fn default() -> Self {
        Self::new()
    }
}

/// True when an episode row carries real data. Tenrai's `/episodes` endpoint
/// sometimes appends placeholder rows: a repeated title with no `aired` date
/// (e.g. mal_id 27899). Such rows must not enter the catalog.
fn is_real_episode(ep: &JikanEpisode) -> bool {
    ep.title.is_some() || ep.aired.is_some()
}

/// Outcome of trying every host for one logical request.
enum HostAttempt {
    /// A terminal response from some host: success, or a non-retryable 4xx
    /// (e.g. 404). Returned to the caller untouched.
    Response(reqwest::Response),
    /// Every host failed transiently (network error / 429 / 5xx). Carries a
    /// per-host description so the final error lists all statuses.
    AllTransient(String),
}

impl MalService {
    pub fn new() -> Self {
        Self {
            client: super::http_client(),
            base_urls: [TENRAI_BASE_URL, JIKAN_BASE_URL],
        }
    }

    /// Sends `GET {host}{path}` against each host in priority order and
    /// returns the first *terminal* response. Only a network error, 429 or
    /// 5xx advances to the next host. When every host fails transiently the
    /// returned summary lists each host's status.
    async fn try_hosts(&self, hosts: &[&str], path: &str) -> HostAttempt {
        let mut failures: Vec<String> = Vec::with_capacity(hosts.len());
        for host in hosts {
            let url = format!("{host}{path}");
            match self.client.get(&url).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.as_u16() == 429 || status.is_server_error() {
                        failures.push(format!("{host}: {status}"));
                        continue;
                    }
                    return HostAttempt::Response(resp);
                }
                Err(e) => {
                    failures.push(format!("{host}: {e}"));
                    continue;
                }
            }
        }
        HostAttempt::AllTransient(failures.join("; "))
    }

    /// Terminal response or an error whose message contains every host's
    /// failure, used by the non-paginated methods.
    async fn get_with_failover(
        &self,
        hosts: &[&str],
        path: &str,
    ) -> Result<reqwest::Response, anyhow::Error> {
        match self.try_hosts(hosts, path).await {
            HostAttempt::Response(resp) => Ok(resp),
            HostAttempt::AllTransient(msg) => {
                anyhow::bail!("MAL request failed on all hosts: {msg}")
            }
        }
    }

    pub async fn search(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        // Build the query once with `Url` (correct percent-encoding), then
        // reuse the path across hosts.
        let mut url = Url::parse("https://mal.invalid/anime")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", query);
            pairs.append_pair("limit", &SEARCH_LIMIT.to_string());
        }
        let path = match url.query() {
            Some(q) => format!("{}?{}", url.path(), q),
            None => url.path().to_string(),
        };

        let response = self.get_with_failover(&self.base_urls, &path).await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(
                crate::services::external::NotFoundError(format!("mal search: {query}")).into(),
            );
        }
        if !status.is_success() {
            anyhow::bail!("MAL search failed: {}", status);
        }

        let body: MalAnimeSearchResponse = response.json().await?;
        Ok(body.data.into_iter().map(map_search).collect())
    }

    pub async fn get_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let path = format!("/anime/{id}/full");
        let response = self.get_with_failover(&self.base_urls, &path).await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(crate::services::external::NotFoundError(format!("mal {id}")).into());
        }
        if !status.is_success() {
            anyhow::bail!("MAL details failed: {}", status);
        }

        let body: MalAnimeResponse = response.json().await?;
        Ok(map_full(body.data))
    }

    /// Search manga via `GET /manga?q=...&limit=N` (Tenrai, Jikan fallback).
    pub async fn search_manga(&self, query: &str) -> Result<Vec<CreateMediaItem>, anyhow::Error> {
        let mut url = Url::parse("https://mal.invalid/manga")?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("q", query);
            pairs.append_pair("limit", &SEARCH_LIMIT.to_string());
        }
        let path = match url.query() {
            Some(q) => format!("{}?{}", url.path(), q),
            None => url.path().to_string(),
        };

        let response = self.get_with_failover(&self.base_urls, &path).await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(crate::services::external::NotFoundError(format!(
                "mal manga search: {query}"
            ))
            .into());
        }
        if !status.is_success() {
            anyhow::bail!("MAL manga search failed: {}", status);
        }

        let body: MalMangaSearchResponse = response.json().await?;
        Ok(body.data.into_iter().map(map_manga_search).collect())
    }

    /// Fetch full manga details via `GET /manga/{id}/full` (Tenrai, Jikan fallback).
    pub async fn get_manga_details(&self, id: &str) -> Result<CreateMediaItem, anyhow::Error> {
        let path = format!("/manga/{id}/full");
        let response = self.get_with_failover(&self.base_urls, &path).await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(crate::services::external::NotFoundError(format!("mal manga {id}")).into());
        }
        if !status.is_success() {
            anyhow::bail!("MAL manga details failed: {}", status);
        }

        let body: MalMangaResponse = response.json().await?;
        Ok(map_manga_full(body.data))
    }

    /// Fetch the full episode list for an anime.
    /// Endpoint: `GET /anime/{mal_id}/episodes?page=N`, paginated until
    /// `pagination.has_next_page == false`.
    ///
    /// Resilience: each page is requested with host failover (Tenrai first,
    /// Jikan second) and retried on transient failure of *both* hosts
    /// (network / 429 / 5xx, up to `MAX_ATTEMPTS` with linear backoff).
    /// A JSON parse error or a non-retryable 4xx stops the whole loop.
    ///
    /// Placeholder rows (no title AND no air date) are dropped: Tenrai's
    /// endpoint occasionally repeats the list (e.g. mal_id 27899 returns 24
    /// rows for a 12-episode show), which would otherwise double the drawer
    /// and inflate the `media_items.episodes` denominator.
    pub async fn fetch_episodes(&self, mal_id: i64) -> Result<Vec<JikanEpisode>, anyhow::Error> {
        const MAX_ATTEMPTS: u32 = 3;
        const RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(2000);
        const PAGE_PAUSE: std::time::Duration = std::time::Duration::from_millis(250);

        let mut all = Vec::new();
        let mut page: i32 = 1;
        loop {
            let path = format!("/anime/{mal_id}/episodes?page={page}");

            let body: JikanEpisodesResponse = match self
                .fetch_page_with_retry(
                    &self.base_urls,
                    &path,
                    mal_id,
                    page,
                    MAX_ATTEMPTS,
                    RETRY_BACKOFF,
                )
                .await
            {
                Ok(b) => b,
                Err(()) => break,
            };

            let got = body.data.len();
            tracing::info!(
                mal_id,
                page,
                got,
                total_so_far = all.len() + got,
                "mal episodes page (tenrai/jikan)"
            );
            all.extend(body.data);
            if got == 0
                || !body.pagination.has_next_page
                || page >= body.pagination.last_visible_page
            {
                break;
            }
            page += 1;
            // Jikan rate limit: 3 req/sec, 60 req/min. 250 ms keeps us
            // under the per-second cap with small margin.
            tokio::time::sleep(PAGE_PAUSE).await;
        }

        // Drop Tenrai placeholder rows (see doc comment above).
        all.retain(is_real_episode);
        Ok(all)
    }

    /// GET one page with host failover, retrying while *every* host fails
    /// transiently. Returns `Err(())` to signal "stop the whole pagination
    /// loop" (non-recoverable 4xx, JSON parse failure, or retries exhausted).
    async fn fetch_page_with_retry(
        &self,
        hosts: &[&str],
        path: &str,
        mal_id: i64,
        page: i32,
        max_attempts: u32,
        base_backoff: std::time::Duration,
    ) -> Result<JikanEpisodesResponse, ()> {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            match self.try_hosts(hosts, path).await {
                HostAttempt::Response(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        return match resp.json().await {
                            Ok(b) => Ok(b),
                            Err(e) => {
                                tracing::warn!(mal_id, page, error = %e, "mal episodes: json parse failed, stopping");
                                Err(())
                            }
                        };
                    }
                    // 404 / other 4xx — won't self-heal, and 404 never fails over.
                    tracing::warn!(mal_id, page, %status, "mal episodes: non-retryable status, stopping");
                    return Err(());
                }
                HostAttempt::AllTransient(msg) => {
                    if attempt < max_attempts {
                        let backoff = base_backoff * attempt;
                        tracing::warn!(
                            mal_id,
                            page,
                            attempt,
                            error = %msg,
                            backoff_ms = backoff.as_millis() as u64,
                            "mal episodes: all hosts transient, retrying"
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    tracing::warn!(mal_id, page, error = %msg, "mal episodes: retries exhausted, stopping");
                    return Err(());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Minimal one-shot HTTP/1.1 server for failover tests.
    /// Replies to every request with `status` + `body` and counts how many
    /// requests actually arrived. Returns `(base_url, request_counter)`.
    /// No external mock crate: tokio is already a dependency.
    async fn spawn_stub(status: u16, body: &'static str) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let count_task = count.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let count = count_task.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf).await;
                    count.fetch_add(1, Ordering::SeqCst);
                    let reason = match status {
                        200 => "OK",
                        404 => "Not Found",
                        429 => "Too Many Requests",
                        500 => "Internal Server Error",
                        503 => "Service Unavailable",
                        _ => "Status",
                    };
                    let resp = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        (format!("http://{addr}"), count)
    }

    #[tokio::test]
    async fn failover_returns_first_host_without_contacting_second() {
        let (a, ca) = spawn_stub(200, "{}").await;
        let (b, cb) = spawn_stub(200, "{}").await;
        let svc = MalService::new();
        let resp = svc
            .get_with_failover(&[a.as_str(), b.as_str()], "/anime")
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(ca.load(Ordering::SeqCst), 1);
        assert_eq!(
            cb.load(Ordering::SeqCst),
            0,
            "second host must not be called"
        );
    }

    #[tokio::test]
    async fn failover_falls_back_on_5xx() {
        let (a, ca) = spawn_stub(500, "{}").await;
        let (b, cb) = spawn_stub(200, "{}").await;
        let svc = MalService::new();
        let resp = svc
            .get_with_failover(&[a.as_str(), b.as_str()], "/anime/1/full")
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(ca.load(Ordering::SeqCst), 1);
        assert_eq!(cb.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failover_reports_both_statuses_when_all_fail() {
        let (a, _ca) = spawn_stub(500, "{}").await;
        let (b, _cb) = spawn_stub(503, "{}").await;
        let svc = MalService::new();
        let err = svc
            .get_with_failover(&[a.as_str(), b.as_str()], "/anime")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("500"), "missing first status: {msg}");
        assert!(msg.contains("503"), "missing second status: {msg}");
    }

    #[tokio::test]
    async fn failover_does_not_retry_on_404() {
        let (a, ca) = spawn_stub(404, "{}").await;
        let (b, cb) = spawn_stub(200, "{}").await;
        let svc = MalService::new();
        let resp = svc
            .get_with_failover(&[a.as_str(), b.as_str()], "/anime/999999/full")
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
        assert_eq!(ca.load(Ordering::SeqCst), 1);
        assert_eq!(cb.load(Ordering::SeqCst), 0, "404 must not fail over");
    }

    #[tokio::test]
    async fn failover_falls_back_on_429() {
        let (a, _ca) = spawn_stub(429, "{}").await;
        let (b, cb) = spawn_stub(200, "{}").await;
        let svc = MalService::new();
        let resp = svc
            .get_with_failover(&[a.as_str(), b.as_str()], "/anime")
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(cb.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failover_falls_back_on_network_error() {
        // Reserve a port, then drop the listener so the connect is refused.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let dead = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let (b, cb) = spawn_stub(200, "{}").await;
        let svc = MalService::new();
        let resp = svc
            .get_with_failover(&[dead.as_str(), b.as_str()], "/anime")
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(cb.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn parses_tenrai_episode_integer_duration() {
        // Live Tenrai shape: `duration` is seconds as a number.
        let json = r#"{"data":[{"mal_id":1,"title":"New Surge","duration":1440,"aired":"2015-01-09T00:00:00+00:00"}],"pagination":{"last_visible_page":1,"has_next_page":false}}"#;
        let body: JikanEpisodesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(body.data[0].duration.as_deref(), Some("1440 sec"));
        assert_eq!(
            parse_duration_to_minutes(body.data[0].duration.as_deref().unwrap()),
            Some(24)
        );
    }

    #[test]
    fn parses_jikan_episode_without_duration() {
        // Jikan omits `duration` entirely; must stay a clean None.
        let json = r#"{"data":[{"mal_id":1,"title":"x","aired":null}],"pagination":{"last_visible_page":1,"has_next_page":false}}"#;
        let body: JikanEpisodesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(body.data[0].duration, None);
    }

    #[test]
    fn drops_tenrai_placeholder_episodes() {
        let real = JikanEpisode {
            mal_id: 1,
            title: Some("New Surge".to_string()),
            title_japanese: Some("新洸".to_string()),
            aired: Some("2015-01-09T00:00:00+00:00".to_string()),
            duration: Some("1440 sec".to_string()),
        };
        let placeholder = JikanEpisode {
            mal_id: 13,
            title: None,
            title_japanese: None,
            aired: None,
            duration: None,
        };
        assert!(is_real_episode(&real));
        assert!(!is_real_episode(&placeholder));
    }

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
        assert_eq!(
            item.aired_from,
            Some(chrono::NaiveDate::from_ymd_opt(2002, 10, 3).unwrap())
        );
        assert_eq!(
            item.aired_to,
            Some(chrono::NaiveDate::from_ymd_opt(2007, 2, 8).unwrap())
        );
        assert_eq!(item.duration.as_deref(), Some("23 min. per ep."));
        assert_eq!(item.rating.as_deref(), Some("PG-13 - Teens 13 or older"));
        assert!(item.studios.contains(&"Studio Pierrot".to_string()));
        assert!(item.producers.contains(&"TV Tokyo".to_string()));
        assert!(item.licensors.contains(&"VIZ Media".to_string()));
        assert!(item.genres.contains(&"Action".to_string()));
        assert!(item.themes.contains(&"Martial Arts".to_string()));
        assert!(item.demographics.contains(&"Shounen".to_string()));
        let details = item.details.expect("details should exist");
        assert_eq!(
            details.get("aired_string").and_then(|v| v.as_str()),
            Some("Oct 3, 2002 to Feb 8, 2007")
        );
    }

    #[test]
    fn parses_manga_search_response() {
        let json = r#"{"data":[{"mal_id":13,"title":"One Piece","title_english":"One Piece","title_japanese":"ワンピース","chapters":1100,"volumes":105,"score":9.22,"status":"Publishing","type":"Manga","genres":[{"name":"Action"},{"name":"Adventure"}],"themes":[{"name":"Pirates"}]}]}"#;
        let body: MalMangaSearchResponse = serde_json::from_str(json).unwrap();
        let item = map_manga_search(body.data.into_iter().next().unwrap());
        assert_eq!(item.title, "One Piece");
        assert_eq!(item.external_id, "13");
        assert_eq!(item.media_type, "manga");
        assert_eq!(item.chapters, Some(1100));
        assert_eq!(item.volumes, Some(105));
        assert_eq!(item.format_type.as_deref(), Some("Manga"));
        assert!(item.genres.contains(&"Action".to_string()));
        assert!(item.themes.contains(&"Pirates".to_string()));
    }

    #[test]
    fn parses_full_manga_response() {
        let json = r#"{
            "data": {
                "mal_id": 13,
                "title": "One Piece",
                "title_english": "One Piece",
                "title_japanese": "ワンピース",
                "chapters": 1100,
                "volumes": 105,
                "synopsis": "...",
                "score": 9.22,
                "status": "Publishing",
                "type": "Manga",
                "published": {"from": "1997-07-22T00:00:00+00:00", "to": null, "string": "Jul 22, 1997 to ?"},
                "authors": [{"name": "Oda Eiichiro"}],
                "genres": [{"name": "Action"}, {"name": "Adventure"}],
                "themes": [{"name": "Pirates"}],
                "demographics": [{"name": "Shounen"}]
            }
        }"#;
        let body: MalMangaResponse = serde_json::from_str(json).unwrap();
        let item = map_manga_full(body.data);
        assert_eq!(item.title, "One Piece");
        assert_eq!(item.external_id, "13");
        assert_eq!(item.media_type, "manga");
        assert_eq!(item.chapters, Some(1100));
        assert_eq!(item.volumes, Some(105));
        assert_eq!(item.format_type.as_deref(), Some("Manga"));
        assert_eq!(item.year, Some(1997));
        assert_eq!(item.premiered_year, Some(1997));
        assert_eq!(
            item.aired_from,
            Some(chrono::NaiveDate::from_ymd_opt(1997, 7, 22).unwrap())
        );
        assert_eq!(item.aired_to, None);
        assert!(item.authors.contains(&"Oda Eiichiro".to_string()));
        assert!(item.demographics.contains(&"Shounen".to_string()));
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
