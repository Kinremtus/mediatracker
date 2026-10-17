//! Yanmaga Web (ヤンマガWeb) official-source search + chapter metadata.
//!
//! Search is server-rendered (`/search?q=`): each result is an `<a>` to
//! `/comics/{percent-encoded-title}` with the cover, title and author lines.
//! The series page carries the whole episode list as `mod-episode-thumbnail`
//! blocks: the episode link, `mod-episode-title` (`第１話 …`, full-width
//! digits) and `mod-episode-date` (`2026/02/09`).
//!
//! The binding id is the percent-encoded slug as it appears in the href, so
//! fetching is `GET /comics/{id}` without re-encoding.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use chrono::NaiveDate;

use crate::services::external::chapter_title::{self, EpisodeRow};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

const SEARCH_BASE: &str = "https://yanmaga.jp/search";
const COMICS_BASE: &str = "https://yanmaga.jp/comics";

/// Browser-like User-Agent; the site rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Defensive cap on one search answer.
const SEARCH_MAX_HITS: usize = 50;
/// Rows read from one series page (a hostile answer must not blow up memory).
const EPISODE_CAP: usize = 5000;
/// Longest stored episode title.
const MAX_TITLE_CHARS: usize = 512;
/// How far a tag search may reach from its anchor.
const WINDOW: usize = 8192;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "yanmaga".to_string(),
        ..OfficialMeta::default()
    }
}

/// True for the percent-encoded slugs the site uses (`/comics/%E3%…`).
fn is_slug_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 512
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'%' | b'_' | b'-' | b'.' | b'~'))
}

/// Search the Japanese catalogue.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.query_pairs_mut().append_pair("q", title);
    let html = get_text(client, url).await?;
    Ok(parse_search(&html))
}

/// Pure parser over the search page: `<a href="/comics/{slug}">` result
/// blocks (the nav's `/comics/series` and `/comics/authors` are skipped).
/// Malformed HTML yields an empty vector — never a panic.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("href=\"/comics/") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(end_rel) = html[at..hi].find("</a>") else {
            break;
        };
        let block = &html[at..at + end_rel];
        cursor = at + end_rel + "</a>".len();
        let Some(slug) = official_meta::between(block, "href=\"/comics/", "\"")
            .map(str::to_string)
            .filter(|s| is_slug_id(s))
        else {
            continue;
        };
        if hits.iter().any(|h| h.id == slug) {
            continue;
        }
        // The title is the first bold line, the author the second line.
        let mut paragraphs = Vec::new();
        let mut rest = block;
        while let Some(start) = rest.find("<p ") {
            let after = &rest[start..];
            let Some(gt) = after.find('>') else {
                break;
            };
            let body = &after[gt + 1..];
            let Some(end) = body.find("</p>") else {
                break;
            };
            paragraphs.push(official_meta::strip_tags(&body[..end]));
            rest = &body[end..];
        }
        let title = paragraphs
            .iter()
            .find(|p| !p.trim().is_empty())
            .map(|t| t.trim().to_string());
        let Some(title) = title else {
            continue;
        };
        let author = paragraphs
            .iter()
            .skip(1)
            .find(|p| !p.trim().is_empty())
            .map(|a| a.trim().to_string());
        let poster_url = official_meta::between(block, "<img", ">")
            .and_then(|tag| official_meta::between(tag, "src=\"", "\""))
            .map(|raw| raw.replace("&amp;", "&"));
        hits.push(OfficialHit::with_extra(
            slug,
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

/// One episode row of the series page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YanmagaEpisode {
    pub title: String,
    pub release_date: Option<NaiveDate>,
}

/// Pure parser over the series page: `mod-episode-thumbnail` blocks with the
/// `mod-episode-title` line and the `mod-episode-date` (`2026/02/09`).
pub fn parse_episodes(html: &str) -> Vec<YanmagaEpisode> {
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("mod-episode-thumbnail") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(end_rel) = html[at..hi].find("</a>") else {
            break;
        };
        let block = &html[at..at + end_rel];
        cursor = at + end_rel + "</a>".len();
        let Some(title) = official_meta::between(block, "class=\"mod-episode-title\">", "</p>")
            .map(official_meta::strip_tags)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let release_date = official_meta::between(block, "class=\"mod-episode-date\">", "</time>")
            .and_then(|raw| NaiveDate::parse_from_str(raw.trim(), "%Y/%m/%d").ok());
        rows.push(YanmagaEpisode {
            title: title.chars().take(MAX_TITLE_CHARS).collect(),
            release_date,
        });
        if rows.len() >= EPISODE_CAP {
            break;
        }
    }
    rows
}

/// Number the rows: the `第N話` counter from the title (shared JP classifier);
/// rows without one are side content and become bonuses.
pub fn chapters_from_rows(rows: Vec<YanmagaEpisode>) -> Vec<ChapterMeta> {
    let typed: Vec<EpisodeRow> = rows
        .into_iter()
        .map(|row| EpisodeRow {
            kind: chapter_title::classify_jp_title(&row.title),
            title: Some(row.title),
            release_date: row.release_date,
        })
        .collect();
    chapter_title::rows_to_chapters(typed)
}

/// Fetch one series' episode list and map it onto the shared chapter model.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    if !is_slug_id(source_id) {
        return Ok(empty_meta());
    }
    let url = url::Url::parse(&format!("{COMICS_BASE}/{source_id}"))?;
    let html = get_text(client, url).await?;
    // The series page shows only a window (first + latest) when more episodes
    // exist — the full list lives on `/comics/{slug}/episodes` behind a
    // JS-injected markup. A partial list must not reach `apply_meta` (its
    // orphan cleanup would drop rows and the counter would lie), so windowed
    // series are skipped until the full-list walk lands.
    if html.contains("mod-episode-more-button") {
        anyhow::bail!("yanmaga: episode list is windowed (more episodes exist)");
    }
    let rows = parse_episodes(&html);
    if rows.is_empty() {
        return Ok(empty_meta());
    }
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "yanmaga".to_string(),
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

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/yanmaga_search.html");
    const SERIES_FIXTURE: &str = include_str!("../../../tests/fixtures/yanmaga_series.html");

    #[test]
    fn search_fixture_parses_result_blocks() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert!(hits.len() >= 2, "got {} hits", hits.len());
        let first = hits
            .iter()
            .find(|h| h.title.contains("ドラゴンの胃"))
            .expect("the dragon result");
        assert!(first.id.starts_with("%E3%83%89%E3%83%A9%E3%82%B4"));
        assert_eq!(first.extra.author.as_deref(), Some("大橋ユウ 西村隆"));
        assert!(
            first
                .extra
                .poster_url
                .as_deref()
                .is_some_and(|u| u.contains("cdn")),
            "{:?}",
            first.extra.poster_url
        );
        assert!(
            !hits.iter().any(|h| h.id == "series" || h.id == "authors"),
            "the nav links are skipped"
        );
        // Malformed pages are empty, never a panic.
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn series_fixture_parses_episodes() {
        let rows = parse_episodes(SERIES_FIXTURE);
        assert!(rows.len() >= 3, "got {} rows", rows.len());
        assert!(rows[0].title.starts_with("第"));
        assert!(rows[0].release_date.is_some(), "the date parses");
        // Malformed pages are empty, never a panic.
        assert!(parse_episodes("").is_empty());
        assert!(parse_episodes("<html></html>").is_empty());
    }

    #[test]
    fn chapters_number_from_the_jp_counter() {
        let rows = vec![
            YanmagaEpisode {
                title: "第１話 ドラゴンの胃でおやすみ".to_string(),
                release_date: None,
            },
            YanmagaEpisode {
                title: "第２話 つづき".to_string(),
                release_date: None,
            },
            YanmagaEpisode {
                title: "番外編 おまけ".to_string(),
                release_date: None,
            },
        ];
        let chapters = chapters_from_rows(rows);
        assert_eq!(chapters[0].number_x100, 100, "full-width digits parse");
        assert_eq!(chapters[1].number_x100, 200);
        assert!(chapters[2].is_bonus);
        assert_eq!(chapter_title::main_count(&chapters), 2);
    }

    #[tokio::test]
    async fn unsafe_id_is_empty() {
        let client = crate::services::external::http_client();
        let meta = fetch_meta(&client, "../etc/passwd")
            .await
            .expect("a bad id must not hit the network");
        assert!(meta.chapters.is_empty());
        assert!(meta.count.is_none());
    }
}
