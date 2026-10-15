//! Manman (漫漫漫画) official-source search + chapter metadata.
//!
//! Search is server-rendered: `/search/index.html?keyword=` answers with
//! `div.classification` result blocks (`/comic-{id}.html` links, title,
//! author, cover, synopsis). The chapter list comes from the site's own
//! `POST /works/comic-list-ajax.html` (`id`, `sort`, `page`), which pages
//! ten rows at a time, newest first; the walk stops at the first short page
//! and the rows are reversed to reading order.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use chrono::NaiveDate;

use crate::services::external::chapter_title::{self, is_cn_bonus_title};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

const SEARCH_BASE: &str = "https://www.manmanapp.com/search/index.html";
const LIST_AJAX: &str = "https://www.manmanapp.com/works/comic-list-ajax.html";

/// Browser-like User-Agent; the site rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Defensive cap on one search answer.
const SEARCH_MAX_HITS: usize = 50;
/// Rows the chapter AJAX returns per page (observed).
const PAGE_SIZE: usize = 10;
/// Hard page cap: 100 pages is far beyond any real manhua.
const MAX_PAGES: usize = 100;
/// Longest stored episode title.
const MAX_TITLE_CHARS: usize = 512;
/// Longest stored synopsis taken from a search answer.
const MAX_DESCRIPTION_CHARS: usize = 2000;
/// How far a tag search may reach from its anchor.
const WINDOW: usize = 8192;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "manmanapp".to_string(),
        ..OfficialMeta::default()
    }
}

/// Search the Chinese catalogue.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.query_pairs_mut().append_pair("keyword", title);
    let html = get_text(client, url).await?;
    Ok(parse_search(&html))
}

/// Pure parser over the search page: `div.classification` blocks with the
/// `/comic-{id}.html` link, title, author, cover and synopsis. Malformed HTML
/// yields an empty vector — never a panic.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("class=\"classification\"") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(end_rel) = html[at..hi].find("</ul>") else {
            break;
        };
        let block = &html[at..at + end_rel];
        cursor = at + end_rel + "</ul>".len();
        let Some(id) = official_meta::between(block, "/comic-", ".html")
            .map(str::to_string)
            .filter(|id| official_meta::is_digit_id(id))
        else {
            continue;
        };
        if hits.iter().any(|h| h.id == id) {
            continue;
        }
        let Some(title) = official_meta::between(block, "class=\"title\">", "</li>")
            .and_then(|raw| official_meta::between(raw, ">", "</a>"))
            .map(official_meta::strip_tags)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        // The cover's `src` may sit on its own line inside the `<img>` tag.
        let poster_url = official_meta::between(block, "<img", ">")
            .and_then(|tag| official_meta::between(tag, "src=\"", "\""))
            .map(str::to_string);
        let author = official_meta::between(block, "class=\"subtitle\">", "</li>")
            .and_then(|raw| official_meta::between(raw, ">", "</a>"))
            .map(official_meta::strip_tags)
            .filter(|a| !a.is_empty());
        let description = official_meta::between(block, "class=\"text ellipsis\">", "</li>")
            .map(official_meta::strip_tags)
            .filter(|d| !d.is_empty())
            .map(|d| d.chars().take(MAX_DESCRIPTION_CHARS).collect());
        hits.push(OfficialHit::with_extra(
            id,
            title.chars().take(MAX_TITLE_CHARS).collect::<String>(),
            HitExtra {
                poster_url,
                description,
                author,
                ..HitExtra::default()
            },
        ));
        if hits.len() >= SEARCH_MAX_HITS {
            break;
        }
    }
    hits
}

/// One chapter row of the AJAX answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManmanChapter {
    pub number: Option<i32>,
    pub title: String,
    pub release_date: Option<NaiveDate>,
    pub is_bonus: bool,
}

/// Pure parser over one AJAX page: `data[]` rows with the chapter title (its
/// leading digits are the number) and the publish date.
pub fn parse_chapters(json: &str) -> Vec<ManmanChapter> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|t| !t.is_empty())?;
            let digits: String = title.chars().take_while(char::is_ascii_digit).collect();
            let number = digits.parse::<i32>().ok().filter(|n| *n >= 1);
            let release_date = item
                .get("publish_time")
                .and_then(|v| v.as_str())
                .and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok());
            Some(ManmanChapter {
                number,
                is_bonus: is_cn_bonus_title(title),
                title: title.chars().take(MAX_TITLE_CHARS).collect(),
                release_date,
            })
        })
        .collect()
}

/// Number the rows: the leading digits of the title; side content
/// (`01 预告`, notices) becomes bonuses.
pub fn chapters_from_rows(rows: Vec<ManmanChapter>) -> Vec<ChapterMeta> {
    rows.into_iter()
        .map(|row| {
            let is_bonus = row.is_bonus || row.number.is_none();
            ChapterMeta {
                number_x100: if is_bonus {
                    0
                } else {
                    row.number.unwrap_or(0) * 100
                },
                title: Some(row.title),
                release_date: row.release_date,
                is_bonus,
                label: None,
            }
        })
        .collect()
}

/// Walk the chapter AJAX (newest first, ten rows per page) and map it onto
/// the shared chapter model. The walk stops at the first short page; the rows
/// are reversed into reading order.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(source_id) {
        return Ok(empty_meta());
    }
    let mut rows: Vec<ManmanChapter> = Vec::new();
    for page in 1..=MAX_PAGES {
        let body = post_chapters(client, source_id, page).await?;
        let page_rows = parse_chapters(&body);
        let short = page_rows.len() < PAGE_SIZE;
        rows.extend(page_rows);
        if short {
            break;
        }
    }
    if rows.is_empty() {
        return Ok(empty_meta());
    }
    rows.reverse();
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "manmanapp".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    })
}

/// One chapter page of one comic.
async fn post_chapters(
    client: &reqwest::Client,
    comic_id: &str,
    page: usize,
) -> anyhow::Result<String> {
    // The `form` feature of reqwest is not enabled: build the body by hand.
    let body = format!(
        "id={}&sort=&page={}",
        url::form_urlencoded::byte_serialize(comic_id.as_bytes()).collect::<String>(),
        page
    );
    let response = client
        .post(LIST_AJAX)
        .header(reqwest::header::USER_AGENT, UA)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(reqwest::header::ACCEPT, "application/json, text/html")
        .body(body)
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

/// GET a page and read the body with a hard size cap.
async fn get_text(client: &reqwest::Client, url: url::Url) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/manmanapp_search.html");
    const CHAPTERS_FIXTURE: &str = include_str!("../../../tests/fixtures/manmanapp_chapters.json");

    #[test]
    fn search_fixture_parses_result_blocks() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert_eq!(hits.len(), 3, "the trimmed fixture keeps three results");
        assert_eq!(hits[0].id, "1405541");
        assert_eq!(hits[0].title, "魔女罗琳");
        assert_eq!(hits[0].extra.author.as_deref(), Some("创迹文化"));
        assert!(
            hits[0]
                .extra
                .poster_url
                .as_deref()
                .is_some_and(|u| u.contains("images.tmanga.com")),
            "{:?}",
            hits[0].extra.poster_url
        );
        assert!(
            hits[0]
                .extra
                .description
                .as_deref()
                .is_some_and(|d| d.contains("复仇")),
            "{:?}",
            hits[0].extra.description
        );
        // Malformed pages are empty, never a panic.
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn chapters_fixture_numbers_and_dates() {
        let rows = parse_chapters(CHAPTERS_FIXTURE);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].number, Some(2));
        assert_eq!(
            rows[0].release_date,
            official_meta::parse_date("2025-06-28")
        );
        let preview = rows.last().expect("preview row");
        assert_eq!(preview.title, "01 预告");
        assert!(preview.is_bonus, "预告 is side content");
        // Malformed answers are empty, never a panic.
        assert!(parse_chapters("").is_empty());
        assert!(parse_chapters("{}").is_empty());
    }

    #[test]
    fn chapters_number_by_title_and_reverse_to_reading_order() {
        // The AJAX walks newest first; the fixture is a mid-walk page.
        let rows = vec![
            ManmanChapter {
                number: Some(3),
                title: "03".to_string(),
                release_date: None,
                is_bonus: false,
            },
            ManmanChapter {
                number: Some(2),
                title: "02".to_string(),
                release_date: None,
                is_bonus: false,
            },
            ManmanChapter {
                number: Some(1),
                title: "01 预告".to_string(),
                release_date: None,
                is_bonus: true,
            },
        ];
        let chapters = chapters_from_rows(rows);
        assert_eq!(chapters[0].number_x100, 300);
        assert_eq!(chapters[1].number_x100, 200);
        assert!(chapters[2].is_bonus);
        assert_eq!(chapter_title::main_count(&chapters), 3);
    }

    #[tokio::test]
    async fn non_numeric_id_is_empty() {
        let client = crate::services::external::http_client();
        let meta = fetch_meta(&client, "abc")
            .await
            .expect("a bad id must not hit the network");
        assert!(meta.chapters.is_empty());
        assert!(meta.count.is_none());
    }
}
