//! Sunday Webry (サンデーうぇぶり) official-source search + chapter metadata.
//!
//! Search is server-rendered: `/search?q=` answers with
//! `li[data-title]` result blocks (series title, author, cover). The series id
//! is embedded in the cover URL's `series-thumbnail/{id}-{hash}` path. The
//! chapter list is the series' own RSS feed (`/rss/series/{id}`, newest first,
//! the episodes the site currently offers) — reversed into reading order and
//! numbered positionally, exactly as the feed lists them.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use chrono::NaiveDate;

use crate::services::external::chapter_title::{self, RawRow, is_jp_bonus_title};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

const SEARCH_BASE: &str = "https://www.sunday-webry.com/search";
const RSS_BASE: &str = "https://www.sunday-webry.com/rss/series";

/// Browser-like User-Agent; the site rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Defensive cap on one search answer.
const SEARCH_MAX_HITS: usize = 50;
/// Rows read from one feed (the site caps the feed itself; a hostile answer
/// must not blow up memory).
const FEED_CAP: usize = 1000;
/// Longest stored episode title.
const MAX_TITLE_CHARS: usize = 512;
/// How far a tag search may reach from its anchor.
const WINDOW: usize = 8192;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "sundaywebry".to_string(),
        ..OfficialMeta::default()
    }
}

/// Search the Japanese catalogue.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.query_pairs_mut().append_pair("q", title);
    let html = get_text(client, url).await?;
    Ok(parse_search(&html))
}

/// Pure parser over the search page: `li[data-title]` blocks with the series
/// title, author and cover; the series id comes from the cover URL
/// (`…/series-thumbnail/{id}-{hash}/…`). Malformed HTML yields an empty vector
/// — never a panic.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("data-title=\"") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(end_rel) = html[at..hi].find("</li>") else {
            break;
        };
        let item = &html[at..at + end_rel];
        cursor = at + end_rel + "</li>".len();
        let Some(title) = official_meta::between(item, "class=\"series-title\">", "</p>")
            .map(official_meta::strip_tags)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        // The cover URL is percent-encoded (`series-thumbnail%2F{id}-…`), so
        // the id is everything digit-leading after the marker.
        let Some(series_id) = official_meta::between(item, "series-thumbnail", "-")
            .and_then(|raw| raw.rsplit(|c: char| !c.is_ascii_digit()).next())
            .map(str::to_string)
        else {
            continue;
        };
        if !official_meta::is_digit_id(&series_id) || hits.iter().any(|h| h.id == series_id) {
            continue;
        }
        let author = official_meta::between(item, "class=\"author\">", "</p>")
            .map(official_meta::strip_tags)
            .filter(|a| !a.is_empty());
        let poster_url =
            official_meta::between(item, "<img src=\"", "\"").map(|raw| raw.replace("&amp;", "&"));
        hits.push(OfficialHit::with_extra(
            series_id,
            title.chars().take(MAX_TITLE_CHARS).collect::<String>(),
            HitExtra {
                poster_url,
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

/// One feed item (the feed lists newest first).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebryItem {
    pub title: String,
    pub release_date: Option<NaiveDate>,
}

/// Pure parser over the series RSS: every `<item>`'s title and pubDate.
pub fn parse_rss(xml: &str) -> Vec<WebryItem> {
    let mut items = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = xml[cursor..].find("<item>") {
        let start = cursor + relative + "<item>".len();
        let Some(end_rel) = xml[start..].find("</item>") else {
            break;
        };
        let item = &xml[start..start + end_rel];
        cursor = start + end_rel + "</item>".len();
        let Some(raw_title) = rss_tag(item, "title") else {
            continue;
        };
        let title = raw_title.trim();
        if title.is_empty() {
            continue;
        }
        let title: String = title.chars().take(MAX_TITLE_CHARS).collect();
        let release_date = rss_tag(item, "pubDate").and_then(parse_rss_date);
        items.push(WebryItem {
            title,
            release_date,
        });
        if items.len() >= FEED_CAP {
            break;
        }
    }
    items
}

/// The text of one `<tag>…</tag>` inside an RSS item, with an optional CDATA
/// wrapper stripped.
fn rss_tag<'a>(item: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let raw = official_meta::between(item, &open, &close)?;
    Some(
        raw.strip_prefix("<![CDATA[")
            .and_then(|s| s.strip_suffix("]]>"))
            .unwrap_or(raw),
    )
}

/// RFC 2822 (`Fri, 14 Mar 2025 15:00:00 +0000`) -> the UTC day.
fn parse_rss_date(raw: &str) -> Option<NaiveDate> {
    chrono::DateTime::parse_from_rfc2822(raw.trim())
        .ok()
        .map(|dt| dt.date_naive())
}

/// Number the feed rows by position, oldest first: the feed lists the newest
/// item first, so it is reversed; side-content markers (`番外編`, …) become
/// bonuses and never consume a number.
pub fn chapters_from_items(items: Vec<WebryItem>) -> Vec<ChapterMeta> {
    let raw: Vec<RawRow> = items
        .into_iter()
        .rev()
        .map(|item| RawRow {
            is_bonus: is_jp_bonus_title(&item.title),
            title: Some(item.title),
            release_date: item.release_date,
        })
        .collect();
    chapter_title::positional_rows(raw)
}

/// Fetch one series' feed and map it onto the shared chapter model.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(source_id) {
        return Ok(empty_meta());
    }
    let url = url::Url::parse(&format!("{RSS_BASE}/{source_id}"))?;
    let xml = get_text(client, url).await?;
    let items = parse_rss(&xml);
    if items.is_empty() {
        return Ok(empty_meta());
    }
    let chapters = chapters_from_items(items);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "sundaywebry".to_string(),
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
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/sundaywebry_search.html");
    const RSS_FIXTURE: &str = include_str!("../../../tests/fixtures/sundaywebry_rss.xml");

    #[test]
    fn search_fixture_parses_series_blocks() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert_eq!(hits.len(), 1, "the query matched one series");
        assert_eq!(hits[0].id, "3269754496548915224");
        assert_eq!(hits[0].title, "古見さんは、コミュ症です。");
        assert_eq!(hits[0].extra.author.as_deref(), Some("オダトモヒト"));
        assert!(
            hits[0]
                .extra
                .poster_url
                .as_deref()
                .is_some_and(|u| u.contains("series-thumbnail")),
            "{:?}",
            hits[0].extra.poster_url
        );
        // Malformed pages are empty, never a panic.
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn rss_fixture_is_newest_first_with_dates() {
        let items = parse_rss(RSS_FIXTURE);
        assert_eq!(items.len(), 5);
        assert_eq!(items[0].title, "コミュ72 後夜祭です。");
        assert_eq!(items[4].title, "コミュ68 只野くんもメイドです。");
        assert!(items[0].release_date.is_some());
        // Malformed XML is empty, never a panic.
        assert!(parse_rss("").is_empty());
        assert!(parse_rss("<rss></rss>").is_empty());
    }

    #[test]
    fn chapters_number_by_position_oldest_first() {
        let items = vec![
            WebryItem {
                title: "コミュ72 後夜祭です。".to_string(),
                release_date: None,
            },
            WebryItem {
                title: "コミュ71 文化祭です。2".to_string(),
                release_date: None,
            },
            WebryItem {
                title: "番外編 おまけ".to_string(),
                release_date: None,
            },
            WebryItem {
                title: "コミュ1 普通の人です。".to_string(),
                release_date: None,
            },
        ];
        let chapters = chapters_from_items(items);
        // Reversed: コミュ1 first, then the bonus, then コミュ71/72.
        assert_eq!(chapters[0].title.as_deref(), Some("コミュ1 普通の人です。"));
        assert_eq!(chapters[0].number_x100, 100);
        assert!(chapters[1].is_bonus, "番外編 never consumes a number");
        assert_eq!(chapters[2].number_x100, 200);
        assert_eq!(chapters[3].number_x100, 300);
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
