//! Tappytoon (tappytoon.com) official-source title search + chapter metadata.
//!
//! The site is a Next.js SPA, but every server-rendered page embeds its Redux
//! state in `__NEXT_DATA__`: the search page carries an anonymous API token
//! (`props.initialState.axios.headers.Authorization`) together with the matching
//! comics (`props.initialState.entities.comics`). That token authorises the
//! public `api-global.tappytoon.com` JSON API, whose `/comics/{id}/chapters`
//! returns the complete chapter list (titles + `createdAt`) without a login.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::services::external::chapter_title::{self, RawRow};
use crate::services::external::official_meta::{self, OfficialMeta};
use crate::services::official_types::OfficialHit;

const SEARCH_BASE: &str = "https://www.tappytoon.com/en/search/all";
const API_BASE: &str = "https://api-global.tappytoon.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
/// The anonymous API token comes from the SSR state of any page; it is reused
/// until it ages out or the API answers 401.
const TOKEN_TTL: Duration = Duration::from_secs(30 * 60);
/// Query used to obtain a fresh token when none is cached (hits are discarded).
const TOKEN_REFRESH_QUERY: &str = "a";
/// Defensive cap on rows taken from one answer.
const MAX_ROWS: usize = 5000;
/// Longest stored episode title (a hostile answer must not bloat the DB).
const MAX_TITLE_CHARS: usize = 512;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "tappytoon".to_string(),
        ..OfficialMeta::default()
    }
}

fn token_cell() -> &'static Mutex<Option<(String, Instant)>> {
    static CELL: OnceLock<Mutex<Option<(String, Instant)>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

fn cached_token(now: Instant) -> Option<String> {
    let guard = token_cell().lock().ok()?;
    guard
        .as_ref()
        .and_then(|(token, at)| (now.duration_since(*at) < TOKEN_TTL).then(|| token.clone()))
}

fn store_token(token: &str) {
    if let Ok(mut guard) = token_cell().lock() {
        *guard = Some((token.to_string(), Instant::now()));
    }
}

/// Drop the cached token so the next refresh cannot hand back a rejected one.
fn clear_token() {
    if let Ok(mut guard) = token_cell().lock() {
        *guard = None;
    }
}

/// One search page: its anonymous API token (if any) and the matching comics.
async fn fetch_page(
    client: &reqwest::Client,
    title: &str,
) -> anyhow::Result<(Option<String>, Vec<OfficialHit>)> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.query_pairs_mut()
        .append_pair("q", title)
        .append_pair("searchOption", "url_slug_like");
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?;
    let body = official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await?;
    Ok(parse_search(&body))
}

/// Search and keep the page's token for the next `fetch_meta`.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let (token, hits) = fetch_page(client, title).await?;
    if let Some(token) = token {
        store_token(&token);
    }
    Ok(hits)
}

/// Pure parser over the SSR page: `__NEXT_DATA__` -> (token, hits). The token
/// is the anonymous `Bearer …` the page's API client is initialised with; the
/// hits come from the Redux `entities.comics` map (id -> title). Malformed or
/// truncated HTML yields `(None, [])` - never a panic.
pub fn parse_search(html: &str) -> (Option<String>, Vec<OfficialHit>) {
    let Some(json) = official_meta::next_data_str(html) else {
        return (None, Vec::new());
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return (None, Vec::new());
    };
    let state = value.pointer("/props/initialState");
    let token = state
        .and_then(|s| s.pointer("/axios/headers/Authorization"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|t| t.starts_with("Bearer "))
        .map(str::to_string);
    let mut hits = Vec::new();
    if let Some(comics) = state
        .and_then(|s| s.pointer("/entities/comics"))
        .and_then(|v| v.as_object())
    {
        for (id, comic) in comics.iter().take(MAX_ROWS) {
            // The id lands in the API path: digits only.
            let Some(title) = comic.get("title").and_then(|v| v.as_str()) else {
                continue;
            };
            // Tappytoon also sells novels; an explicitly non-comic entry is
            // not a manhwa binding.
            if comic
                .get("category")
                .and_then(|v| v.as_str())
                .is_some_and(|category| category != "comic")
            {
                continue;
            }
            let title = title.trim();
            if title.is_empty() || !official_meta::is_digit_id(id) {
                continue;
            }
            hits.push(OfficialHit {
                id: id.clone(),
                title: title.to_string(),
            });
        }
    }
    (token, hits)
}

/// Fetch the full chapter list through the public API. The token is reused from
/// the last search page; a 401 (expired or rotated) triggers one refresh via a
/// fresh SSR page.
pub async fn fetch_meta(client: &reqwest::Client, comic_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(comic_id) {
        return Ok(empty_meta());
    }
    let token = match cached_token(Instant::now()) {
        Some(token) => token,
        None => refresh_token(client).await?,
    };
    let path = format!("/comics/{comic_id}/chapters?locale=en");
    match api_get(client, &token, &path).await? {
        ApiResponse::Body(body) => Ok(parse_chapters(&body)),
        ApiResponse::Unauthorized => {
            // The cached token is known dead: drop it first so the refresh
            // cannot hand it back, then take the token of its own fresh page.
            clear_token();
            let token = refresh_token(client).await?;
            match api_get(client, &token, &path).await? {
                ApiResponse::Body(body) => Ok(parse_chapters(&body)),
                ApiResponse::Unauthorized => Ok(empty_meta()),
            }
        }
    }
}

/// One API answer: the body, or the 401 that triggers a token refresh.
enum ApiResponse {
    Body(String),
    Unauthorized,
}

async fn api_get(client: &reqwest::Client, token: &str, path: &str) -> anyhow::Result<ApiResponse> {
    let url = url::Url::parse(&format!("{API_BASE}{path}"))?;
    // The anonymous token must not surface in a future debug print or trace log.
    let mut auth = reqwest::header::HeaderValue::from_str(token)?;
    auth.set_sensitive(true);
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::AUTHORIZATION, auth)
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Ok(ApiResponse::Unauthorized);
    }
    let response = response.error_for_status()?;
    Ok(ApiResponse::Body(
        official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await?,
    ))
}

/// A fresh token taken from a new SSR page; an answer without one is an error
/// (the caller then returns an empty meta instead of retrying a dead token).
async fn refresh_token(client: &reqwest::Client) -> anyhow::Result<String> {
    let (token, _) = fetch_page(client, TOKEN_REFRESH_QUERY).await?;
    let Some(token) = token else {
        return Err(anyhow::anyhow!("tappytoon: no api token in the SSR state"));
    };
    store_token(&token);
    Ok(token)
}

/// Pure parser over `/comics/{id}/chapters`: the answer is the full episode list
/// in reading order. Titles carry no reliable number (`Episode 0`, `Epilogue
/// 21: Finale`), so main rows are numbered by position like the other
/// title-numberless sources, and side content (`Epilogue …`, `Special …`) goes
/// to the bonus section. Dates come from the `createdAt` day.
pub fn parse_chapters(json: &str) -> OfficialMeta {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return empty_meta();
    };
    let Some(items) = value.as_array() else {
        return empty_meta();
    };
    // The answer is expected oldest-first, but the numbering must not depend on
    // it: sort by the row's own `order` (missing values last).
    let mut items: Vec<&serde_json::Value> = items.iter().collect();
    items.sort_by_key(|i| i.get("order").and_then(|v| v.as_i64()).unwrap_or(i64::MAX));
    let mut rows: Vec<RawRow> = Vec::new();
    for item in items.iter().take(MAX_ROWS) {
        // A scheduled / not-yet-public episode must not consume a number.
        if item.get("isPublished").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let Some(title) = item
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let release_date = item
            .get("createdAt")
            .and_then(|v| v.as_str())
            .and_then(|s| s.get(..10))
            .and_then(official_meta::parse_date);
        rows.push(RawRow {
            title: Some(title.chars().take(MAX_TITLE_CHARS).collect()),
            release_date,
            is_bonus: is_bonus_title(title),
        });
    }
    let chapters = chapter_title::positional_rows(rows);
    let main = chapter_title::main_count(&chapters);
    OfficialMeta {
        source: "tappytoon".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    }
}

/// Side-content prefixes Tappytoon uses on episode titles. The match is on a
/// word boundary (`Extra 3` is a bonus, `Extraordinary Days` is a chapter; a
/// trailing plural `s` is allowed, so `Extras` counts too) and only at the
/// start, so a main row like `Episode 5: The Special One` stays one.
fn is_bonus_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    [
        "epilogue",
        "special",
        "side story",
        "extra",
        "notice",
        "preview",
    ]
    .iter()
    .any(|marker| chapter_title::starts_with_word(&lower, marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/tappytoon_search_next.html");
    const CHAPTERS_FIXTURE: &str = include_str!("../../../tests/fixtures/tappytoon_chapters.json");

    #[test]
    fn search_fixture_yields_token_and_hits() {
        let (token, hits) = parse_search(SEARCH_FIXTURE);
        assert!(
            token.as_deref().is_some_and(|t| t.starts_with("Bearer ")),
            "the SSR state carries the anonymous bearer token"
        );
        assert_eq!(hits.len(), 2);
        assert!(hits.iter().any(|h| h.title == "Solo Leveling"));
        assert!(hits.iter().all(|h| official_meta::is_digit_id(&h.id)));
    }

    #[test]
    fn search_malformed_is_empty() {
        assert_eq!(parse_search(""), (None, Vec::new()));
        assert_eq!(parse_search("<html>no state</html>"), (None, Vec::new()));
        let (token, hits) = parse_search(r#"<script id="__NEXT_DATA__">{bad json</script>"#);
        assert!(token.is_none() && hits.is_empty());
    }

    #[test]
    fn chapters_fixture_positional_with_epilogue_bonus() {
        let meta = parse_chapters(CHAPTERS_FIXTURE);
        assert_eq!(meta.source, "tappytoon");
        // 10 episodes + 2 epilogues: the epilogues are side content.
        assert_eq!(meta.count, Some(10));
        assert_eq!(meta.chapters.len(), 12);
        let mains: Vec<i32> = meta
            .chapters
            .iter()
            .filter(|c| !c.is_bonus)
            .map(|c| c.number_x100)
            .collect();
        assert_eq!(mains, (1..=10).map(|n| n * 100).collect::<Vec<_>>());
        let bonuses: Vec<&official_meta::ChapterMeta> =
            meta.chapters.iter().filter(|c| c.is_bonus).collect();
        assert_eq!(bonuses.len(), 2);
        assert!(bonuses.iter().all(|c| c.number_x100 == 0));
        assert_eq!(
            bonuses[1].title.as_deref(),
            Some("Epilogue 21: Finale"),
            "bonus titles stay verbatim"
        );
        // Dates come from the `createdAt` day; titles stay verbatim.
        assert_eq!(
            meta.chapters[0].release_date,
            official_meta::parse_date("2020-05-25")
        );
        assert_eq!(meta.chapters[0].title.as_deref(), Some("Episode 0"));
    }

    #[test]
    fn chapters_malformed_is_empty() {
        assert!(parse_chapters("").chapters.is_empty());
        assert!(parse_chapters("{}").chapters.is_empty());
        assert!(parse_chapters(r#"[{"title":"  "}]"#).chapters.is_empty());
    }

    #[test]
    fn chapters_sort_by_order_before_numbering() {
        // The numbering relies on `order`, not on the array order.
        let body = r#"[
            {"order": 1, "title": "Episode 1", "createdAt": "2020-05-26T00:00:00Z"},
            {"order": 0, "title": "Episode 0", "createdAt": "2020-05-25T00:00:00Z"}
        ]"#;
        let meta = parse_chapters(body);
        assert_eq!(meta.chapters[0].title.as_deref(), Some("Episode 0"));
        assert_eq!(meta.chapters[1].title.as_deref(), Some("Episode 1"));
    }

    #[test]
    fn chapters_skip_unpublished_rows() {
        let body = r#"[
            {"order": 0, "title": "Episode 0", "isPublished": true},
            {"order": 1, "title": "Episode 1", "isPublished": false}
        ]"#;
        let meta = parse_chapters(body);
        assert_eq!(meta.count, Some(1));
        assert_eq!(meta.chapters[0].title.as_deref(), Some("Episode 0"));
    }

    #[test]
    fn search_keeps_comic_entries_only() {
        let html = "<script id=\"__NEXT_DATA__\">{\"props\":{\"initialState\":{\
            \"axios\":{\"headers\":{\"Authorization\":\"Bearer t\"}},\
            \"entities\":{\"comics\":{\"1\":{\"title\":\"A\",\"category\":\"comic\"},\
            \"2\":{\"title\":\"B\",\"category\":\"novel\"},\"3\":{\"title\":\"C\"}}}}}}</script>";
        let (token, hits) = parse_search(html);
        assert_eq!(token.as_deref(), Some("Bearer t"));
        assert_eq!(
            hits.len(),
            2,
            "the novel entry is dropped, a bare entry kept"
        );
        assert!(hits.iter().all(|h| h.id != "2"));
    }

    #[test]
    fn bonus_markers_are_prefix_only() {
        assert!(is_bonus_title("Epilogue 21: Finale"));
        assert!(is_bonus_title("Special 3"));
        assert!(is_bonus_title("Specials"));
        assert!(is_bonus_title("side story 2"));
        assert!(is_bonus_title("Extra"));
        assert!(is_bonus_title("Extras 3"));
        assert!(!is_bonus_title("Extraordinary Days"));
        assert!(!is_bonus_title("Extraspecial"));
        assert!(!is_bonus_title("Episode 5: The Special One"));
        assert!(!is_bonus_title("Episode 0"));
    }
}
