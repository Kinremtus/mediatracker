//! Kanman (看漫画) official-source search + chapter metadata.
//!
//! Search is a local filter over the site's own catalogue API
//! (`GET /api/getComicList/`, which needs the site's `Referer` and
//! `X-Requested-With` headers): one request carries every comic with its id,
//! title, author, genres, synopsis and score — the site itself filters this
//! list client-side, so the parser does the same
//! ([`official_title::filter_hits_by_query`]). The chapter list is
//! server-rendered on `/{comic_id}/`: `li.item` rows with the chapter
//! position (`data-id`), the chapter id (`data-chapter`) and the title
//! (`第N话 …`), classified through the shared JP counter.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use crate::services::external::chapter_title::{self, EpisodeRow};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta, get_text};
use crate::services::official_types::{HitExtra, OfficialHit};

const CATALOG_BASE: &str = "https://www.kanman.com/api/getComicList/";
const SERIES_BASE: &str = "https://www.kanman.com";

const UA: &str = official_meta::BROWSER_UA;

/// Rows read from one series page (a hostile answer must not blow up memory).
const CHAPTER_CAP: usize = 5000;
/// Longest stored episode title.
const MAX_TITLE_CHARS: usize = 512;
/// Longest stored synopsis taken from a search answer.
const MAX_DESCRIPTION_CHARS: usize = 2000;
/// How far a tag search may reach from its anchor.
const WINDOW: usize = 8192;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "kanman".to_string(),
        ..OfficialMeta::default()
    }
}

/// Fetch the catalogue and filter it locally by the query (the site's search
/// is a client-side filter over exactly this list).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let body = get_catalog(client).await?;
    let hits = parse_catalog(&body);
    Ok(crate::services::official_title::filter_hits_by_query(
        hits, title,
    ))
}

/// Pure parser over the catalogue answer: `data[]` rows with the comic id,
/// title, author, genre pairs (`slug,label|…`), synopsis and score.
/// Malformed JSON yields an empty vector — never a panic.
pub fn parse_catalog(json: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for item in items {
        let Some(id) = item.get("comic_id").and_then(|v| v.as_i64()) else {
            continue;
        };
        let Some(title) = item
            .get("comic_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        if hits.iter().any(|h| h.id == id.to_string()) {
            continue;
        }
        let author = item
            .get("cartoon_author_list_name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string);
        let genres = item
            .get("sort_typelist")
            .and_then(|v| v.as_str())
            .map(|raw| {
                raw.split('|')
                    .filter_map(|pair| pair.split_once(',').map(|(_, label)| label.trim()))
                    .filter(|label| !label.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let description = item
            .get("comic_feature")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(|d| d.chars().take(MAX_DESCRIPTION_CHARS).collect());
        let score = item.get("score").and_then(|v| v.as_f64());
        hits.push(OfficialHit::with_extra(
            id.to_string(),
            title.chars().take(MAX_TITLE_CHARS).collect::<String>(),
            HitExtra {
                description,
                author,
                genres,
                score,
                ..HitExtra::default()
            },
        ));
    }
    hits
}

/// One chapter row of the series page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KanmanChapter {
    pub title: String,
}

/// Pure parser over a series page: `li.item` rows with their `data-id` and the
/// chapter title (`第N话 …`). Rows are returned in the page's reading order.
pub fn parse_chapters(html: &str) -> Vec<KanmanChapter> {
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("data-id=\"") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(end_rel) = html[at..hi].find("</li>") else {
            break;
        };
        let item = &html[at..at + end_rel];
        cursor = at + end_rel + "</li>".len();
        // The slice starts at `data-id`, so the class attribute is behind us;
        // the chapter id follows it on every real row.
        if !item.contains("data-chapter=\"") {
            continue;
        }
        let Some(title) = official_meta::between(item, "<a title=\"", "\"")
            .map(official_meta::strip_tags)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| {
                official_meta::between(item, "class=\"name\">", "</p>")
                    .map(official_meta::strip_tags)
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
            })
        else {
            continue;
        };
        rows.push(KanmanChapter {
            title: title.chars().take(MAX_TITLE_CHARS).collect(),
        });
        if rows.len() >= CHAPTER_CAP {
            break;
        }
    }
    rows
}

/// Number the rows: the `第N话` counter from the title (shared JP classifier,
/// parts included); rows without one are side content and become bonuses.
pub fn chapters_from_rows(rows: Vec<KanmanChapter>) -> Vec<ChapterMeta> {
    let typed: Vec<EpisodeRow> = rows
        .into_iter()
        .map(|row| EpisodeRow {
            kind: chapter_title::classify_jp_title(&row.title),
            title: Some(row.title),
            release_date: None,
        })
        .collect();
    chapter_title::rows_to_chapters(typed)
}

/// Fetch one series' chapter list and map it onto the shared chapter model.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(source_id) {
        return Ok(empty_meta());
    }
    let url = url::Url::parse(&format!("{SERIES_BASE}/{source_id}/"))?;
    let html = get_text(client, url).await?;
    let rows = parse_chapters(&html);
    if rows.is_empty() {
        return Ok(empty_meta());
    }
    if rows.len() >= CHAPTER_CAP {
        // The cap cut the list off; `apply_meta`'s orphan cleanup would drop
        // rows the walk never saw, so refuse the truncated list.
        anyhow::bail!("kanman: chapter row cap hit, refusing a truncated list");
    }
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "kanman".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    })
}

/// GET the catalogue (the API requires the site's Referer + XRW headers).
async fn get_catalog(client: &reqwest::Client) -> anyhow::Result<String> {
    let response = client
        .get(CATALOG_BASE)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::REFERER, "https://www.kanman.com/sort/")
        .header("X-Requested-With", "XMLHttpRequest")
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const CATALOG_FIXTURE: &str = include_str!("../../../tests/fixtures/kanman_catalog.json");
    const SERIES_FIXTURE: &str = include_str!("../../../tests/fixtures/kanman_series.html");

    #[test]
    fn catalog_fixture_filters_locally() {
        let hits = parse_catalog(CATALOG_FIXTURE);
        assert!(hits.len() >= 3, "got {} hits", hits.len());
        let dragon = hits
            .iter()
            .find(|h| h.title == "凤逆天下")
            .expect("the dragon comic");
        assert_eq!(dragon.id, "17745");
        assert_eq!(dragon.extra.author.as_deref(), Some("飒飒动漫"));
        assert!(
            dragon.extra.genres.iter().any(|g| g == "冒险"),
            "{:?}",
            dragon.extra.genres
        );
        assert!(dragon.extra.description.is_some());
        // The query filter keeps only matching titles.
        let filtered = crate::services::official_title::filter_hits_by_query(
            parse_catalog(CATALOG_FIXTURE),
            "凤逆天下",
        );
        assert_eq!(filtered.len(), 1);
        assert!(
            crate::services::official_title::filter_hits_by_query(
                parse_catalog(CATALOG_FIXTURE),
                "zzzqwerty"
            )
            .is_empty()
        );
        // Malformed answers are empty, never a panic.
        assert!(parse_catalog("").is_empty());
        assert!(parse_catalog("{}").is_empty());
    }

    #[test]
    fn series_fixture_parses_chapters() {
        let rows = parse_chapters(SERIES_FIXTURE);
        assert!(rows.len() >= 5, "got {} rows", rows.len());
        assert!(rows[0].title.starts_with("第"));
        let chapters = chapters_from_rows(rows);
        assert!(chapters[0].number_x100 > 0, "第N话 numbers the row");
        // Malformed pages are empty, never a panic.
        assert!(parse_chapters("").is_empty());
        assert!(parse_chapters("<html></html>").is_empty());
    }

    #[test]
    fn chapters_number_from_the_cn_counter() {
        let rows = vec![
            KanmanChapter {
                title: "第1话 穿越".to_string(),
            },
            KanmanChapter {
                title: "第2话 觉醒".to_string(),
            },
            KanmanChapter {
                title: "番外 小剧场".to_string(),
            },
        ];
        let chapters = chapters_from_rows(rows);
        assert_eq!(chapters[0].number_x100, 100);
        assert_eq!(chapters[1].number_x100, 200);
        assert!(chapters[2].is_bonus);
        assert_eq!(chapter_title::main_count(&chapters), 2);
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
