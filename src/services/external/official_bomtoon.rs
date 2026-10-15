//! Bomtoon official-source search + chapter metadata (Korean catalogue).
//!
//! Search goes through the site's own rewrite proxy to its search API:
//! `GET /api/balcony-search-api/search?searchText=` answers with the matching
//! series (alias id, title, authors, tags, cover, open date). The episode list
//! is server-rendered into `/detail/{alias}`'s `__NEXT_DATA__`
//! (`openGraphData.episodes`: `1화`, `완결 후기`, …).
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use crate::services::external::chapter_title::{self, is_kr_bonus_title, parse_kr_chapter_number};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

const SEARCH_BASE: &str = "https://www.bomtoon.com/api/balcony-search-api/search";
const DETAIL_BASE: &str = "https://www.bomtoon.com/detail";

/// Browser-like User-Agent; the site rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Defensive cap on one search answer (the API returns up to 100).
const SEARCH_MAX_HITS: usize = 100;
/// Longest stored episode title.
const MAX_TITLE_CHARS: usize = 512;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "bomtoon".to_string(),
        ..OfficialMeta::default()
    }
}

/// Search the Korean catalogue.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.query_pairs_mut().append_pair("searchText", title);
    let body = get_text(client, url).await?;
    Ok(parse_search(&body))
}

/// Pure parser over the search answer: `data.results[]` with the alias id,
/// title, authors, tags and cover (the `MAIN` thumbnail variant). Malformed
/// JSON yields an empty vector — never a panic.
pub fn parse_search(json: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.pointer("/data/results").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for item in items.iter().take(SEARCH_MAX_HITS) {
        let Some(alias) = item
            .get("alias")
            .and_then(|v| v.as_str())
            .filter(|a| official_meta::is_safe_path_id(a))
        else {
            continue;
        };
        let Some(title) = item
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        if hits.iter().any(|h| h.id == alias) {
            continue;
        }
        let poster_url = item
            .get("thumbnail")
            .and_then(|v| v.as_array())
            .and_then(|list| {
                list.iter()
                    .find(|t| t.get("thumbnailType").and_then(|v| v.as_str()) == Some("MAIN"))
                    .or_else(|| list.first())
            })
            .and_then(|t| t.get("imagePath").and_then(|v| v.as_str()))
            .map(str::to_string);
        let author = item
            .get("author")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::trim)
                    .filter(|a| !a.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|a| !a.is_empty());
        let genres = item
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let year = item
            .get("openedAt")
            .and_then(|v| v.as_str())
            .and_then(|s| s.get(..4))
            .and_then(|y| y.parse().ok());
        hits.push(OfficialHit::with_extra(
            alias.to_string(),
            title.chars().take(MAX_TITLE_CHARS).collect::<String>(),
            HitExtra {
                poster_url,
                author,
                genres,
                year,
                ..HitExtra::default()
            },
        ));
    }
    hits
}

/// One episode row of the server-rendered detail page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BomtoonEpisode {
    pub number: Option<i32>,
    pub title: String,
}

/// Pure parser over `/detail/{alias}`: the `__NEXT_DATA__`
/// `openGraphData.episodes` list. `None` when the page has no such data (the
/// caller treats it as "no official match").
pub fn parse_episodes(html: &str) -> Option<Vec<BomtoonEpisode>> {
    let json = official_meta::between(
        html,
        "<script id=\"__NEXT_DATA__\" type=\"application/json\">",
        "</script>",
    )
    .or_else(|| official_meta::between(html, "<script id=\"__NEXT_DATA__\">", "</script>"))?;
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let items = value
        .pointer("/props/pageProps/openGraphData/episodes")
        .and_then(|v| v.as_array())?;
    let mut rows = Vec::new();
    for item in items {
        let Some(title) = item
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let sub_title = item
            .get("subTitle")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let full = match sub_title {
            Some(sub) => format!("{title} {sub}"),
            None => title.to_string(),
        };
        rows.push(BomtoonEpisode {
            number: parse_kr_chapter_number(title),
            title: full.chars().take(MAX_TITLE_CHARS).collect(),
        });
    }
    Some(rows)
}

/// Number the rows: the `N화` counter from the title; rows without one are
/// side content (`완결 후기`, notices) and become bonuses.
pub fn chapters_from_rows(rows: Vec<BomtoonEpisode>) -> Vec<ChapterMeta> {
    rows.into_iter()
        .map(|row| {
            let is_bonus = row.number.is_none() || is_kr_bonus_title(&row.title);
            ChapterMeta {
                number_x100: if is_bonus {
                    0
                } else {
                    row.number.unwrap_or(0) * 100
                },
                title: Some(row.title),
                release_date: None,
                is_bonus,
                label: None,
            }
        })
        .collect()
}

/// Fetch one series' episode list and map it onto the shared chapter model.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_safe_path_id(source_id) {
        return Ok(empty_meta());
    }
    let url = url::Url::parse(&format!("{DETAIL_BASE}/{source_id}"))?;
    let html = get_text(client, url).await?;
    let Some(rows) = parse_episodes(&html) else {
        return Ok(empty_meta());
    };
    if rows.is_empty() {
        return Ok(empty_meta());
    }
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "bomtoon".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    })
}

/// GET a page and read the body with a hard size cap.
async fn get_text(client: &reqwest::Client, url: url::Url) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT, "application/json, text/html")
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const DETAIL_FIXTURE: &str = include_str!("../../../tests/fixtures/bomtoon_detail.html");

    #[test]
    fn search_parses_alias_titles_and_covers() {
        let json = r#"{"result":"SUCCESS","data":{"totalResults":2,"results":[
            {"alias":"Victory_is_mine","title":"짝사랑 필승법","author":["사랑도피"],
             "tags":["BL","오직봄툰"],"openedAt":"2025-08-06T13:00:00",
             "thumbnail":[{"imagePath":"https://image.balcony.studio/ko/x/detail.webp","thumbnailType":"DETAIL"},
                          {"imagePath":"https://image.balcony.studio/ko/x/main.webp","thumbnailType":"MAIN"}]},
            {"alias":"bad alias!","title":"Broken","author":[]}
        ]}}"#;
        let hits = parse_search(json);
        assert_eq!(hits.len(), 1, "an unsafe alias is dropped");
        assert_eq!(hits[0].id, "Victory_is_mine");
        assert_eq!(hits[0].title, "짝사랑 필승법");
        assert_eq!(
            hits[0].extra.poster_url.as_deref(),
            Some("https://image.balcony.studio/ko/x/main.webp")
        );
        assert_eq!(hits[0].extra.author.as_deref(), Some("사랑도피"));
        assert_eq!(
            hits[0].extra.genres,
            vec!["BL".to_string(), "오직봄툰".to_string()]
        );
        assert_eq!(hits[0].extra.year, Some(2025));
        // Malformed answers are empty, never a panic.
        assert!(parse_search("").is_empty());
        assert!(parse_search("{}").is_empty());
    }

    #[test]
    fn detail_fixture_parses_episodes() {
        let rows = parse_episodes(DETAIL_FIXTURE).expect("fixture has __NEXT_DATA__");
        assert_eq!(rows.len(), 43);
        assert_eq!(rows[0].number, Some(1));
        assert_eq!(rows[0].title, "1화");
        let last = rows.last().expect("last row");
        assert_eq!(last.number, None, "완결 후기 carries no number");
        assert_eq!(last.title, "완결 후기");
        // Malformed pages are None, never a panic.
        assert!(parse_episodes("").is_none());
        assert!(parse_episodes("<html></html>").is_none());
    }

    #[test]
    fn chapters_number_from_the_korean_counter() {
        let rows = vec![
            BomtoonEpisode {
                number: Some(1),
                title: "1화".to_string(),
            },
            BomtoonEpisode {
                number: Some(2),
                title: "2화".to_string(),
            },
            BomtoonEpisode {
                number: None,
                title: "완결 후기".to_string(),
            },
        ];
        let chapters = chapters_from_rows(rows);
        assert_eq!(chapters[0].number_x100, 100);
        assert_eq!(chapters[1].number_x100, 200);
        assert!(chapters[2].is_bonus);
        assert_eq!(chapters[2].number_x100, 0);
        assert_eq!(chapter_title::main_count(&chapters), 2);
    }

    #[tokio::test]
    async fn unsafe_alias_is_empty() {
        let client = crate::services::external::http_client();
        let meta = fetch_meta(&client, "../etc/passwd")
            .await
            .expect("a bad alias must not hit the network");
        assert!(meta.chapters.is_empty());
        assert!(meta.count.is_none());
    }
}
