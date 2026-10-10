//! Kakao Page (page.kakao.com) official-source title search + chapter metadata.
//!
//! Used to verify that a title has an official Korean publication before the
//! tracker marks it as officially licensed. All failures are non-fatal: the
//! caller treats an empty result (or an `Err`) as "no official match".
//!
//! Detail metadata comes from the guest BFF product list
//! (`bff-page.kakao.com/api/gateway/api/v2/content/product/list?series_id=…`):
//! `result.list[]` carries one entry per episode (`item.title` = `<series> N화`,
//! `item.last_release_dt`), paginated by `cursor_index` + `cursor_direction=NEXT`.
//! The endpoint repeats the free-episode block, so rows are deduped by
//! `product_id`. Titles are classified by the shared [`chapter_title`] helpers.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use chrono::NaiveDate;

use crate::services::external::chapter_title::{
    self, EpisodeRow, ShortTitle, classify_kakao_title, main_count, parse_kr_chapter_number,
};
use crate::services::external::official_meta::{self, OfficialMeta};
use crate::services::official_types::OfficialHit;

pub const KAKAO_SEARCH_BASE: &str = "https://bff-page.kakao.com/api/gateway/api/v2/search/series";

const KAKAO_PRODUCT_LIST: &str =
    "https://bff-page.kakao.com/api/gateway/api/v2/content/product/list";

/// Page size for the product-list cursor walk (the endpoint caps it at 100).
const PRODUCT_WINDOW: usize = 100;
/// Hard page cap: 30 x 100 products is far beyond any real webtoon.
const MAX_PRODUCT_PAGES: usize = 30;
/// One page never carries more than the requested window; cap the parsed rows
/// so a hostile or buggy page cannot blow up memory.
const MAX_ROWS_PER_PAGE: usize = 200;

/// Search hits are cached for a week and chapter lists for 15 minutes (the
/// bilibili Decision #8 pattern, simplified): without them an unbound title
/// fires a live search on every drawer open and a bound one re-walks up to
/// `MAX_PRODUCT_PAGES` requests. A failed search starts a cooldown.
const SEARCH_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const META_TTL: Duration = Duration::from_secs(15 * 60);
const COOLDOWN: Duration = Duration::from_secs(90 * 60);

/// Small module-local TTL cache: values expire after `ttl`, keys are opaque.
struct TtlCache<T> {
    ttl: Duration,
    entries: HashMap<String, (Instant, T)>,
}

impl<T: Clone> TtlCache<T> {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: HashMap::new(),
        }
    }

    fn get(&self, key: &str, now: Instant) -> Option<T> {
        let (at, value) = self.entries.get(key)?;
        (now.duration_since(*at) < self.ttl).then(|| value.clone())
    }

    fn put(&mut self, key: &str, value: T, now: Instant) {
        self.entries.insert(key.to_string(), (now, value));
    }
}

fn search_cache() -> &'static Mutex<TtlCache<Vec<OfficialHit>>> {
    static CELL: OnceLock<Mutex<TtlCache<Vec<OfficialHit>>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(TtlCache::new(SEARCH_TTL)))
}

fn meta_cache() -> &'static Mutex<TtlCache<OfficialMeta>> {
    static CELL: OnceLock<Mutex<TtlCache<OfficialMeta>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(TtlCache::new(META_TTL)))
}

fn cooldown_cell() -> &'static Mutex<Option<Instant>> {
    static CELL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// Browser-like User-Agent; the search endpoint rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// GET https://bff-page.kakao.com/api/gateway/api/v2/search/series
///   ?keyword=<kr title>&page=0&size=10
///
/// No cookies: the BFF only needs the accept/origin/referer headers plus a
/// non-empty User-Agent.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(KAKAO_SEARCH_BASE)?;
    url.query_pairs_mut()
        .append_pair("keyword", title)
        .append_pair("page", "0")
        .append_pair("size", "10");
    let body = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::ORIGIN, "https://page.kakao.com")
        .header(reqwest::header::REFERER, "https://page.kakao.com/")
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_search(&body))
}

/// Cached + cooled-down wrapper wired into `HttpOfficialSearch` for `"kakao"`.
/// The BFF has no documented quota, but without this an unbound title would
/// fire a live search on every drawer open.
pub async fn search_cached(
    client: &reqwest::Client,
    title: &str,
) -> anyhow::Result<Vec<OfficialHit>> {
    let now = Instant::now();
    if let Some(hits) = search_cache()
        .lock()
        .expect("kakao search cache poisoned")
        .get(title, now)
    {
        return Ok(hits);
    }
    {
        let cooldown = cooldown_cell().lock().expect("kakao cooldown poisoned");
        if cooldown.is_some_and(|until| now < until) {
            return Ok(Vec::new());
        }
    }
    match search(client, title).await {
        Ok(hits) => {
            search_cache()
                .lock()
                .expect("kakao search cache poisoned")
                .put(title, hits.clone(), now);
            Ok(hits)
        }
        Err(e) => {
            *cooldown_cell().lock().expect("kakao cooldown poisoned") = Some(now + COOLDOWN);
            Err(e)
        }
    }
}

/// Pure parser, unit-testable without network.
///
/// Reads `result.list[]` where each entry has a `title` (string) and a numeric
/// `series_id` (number or numeric string — non-numeric ids are rejected because
/// they would land in the product-list query). Only webtoon entries
/// (`category_uid == 10`) are kept: Kakao returns the same title as both a web
/// novel (11) and a webtoon (10), and only the latter is a valid manhwa match.
/// Entries with a missing/non-numeric `category_uid` are skipped too.
/// Malformed JSON, a missing `result` branch, or any non-array `list` yields an
/// empty vector - never a panic. Entries without a title or series_id are skipped.
pub fn parse_search(body: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };

    let Some(entries) = value
        .get("result")
        .and_then(|result| result.get("list"))
        .and_then(|list| list.as_array())
    else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            // Kakao category ids: 10 = 웹툰 (webtoon), 11 = 웹소설 (web novel),
            // 16 = 책 (book). A missing/non-numeric uid is treated as no match.
            if entry.get("category_uid").and_then(|uid| uid.as_u64()) != Some(10) {
                return None;
            }
            let title = entry.get("title")?.as_str()?.to_string();
            let series_id = entry.get("series_id")?;
            let id = series_id
                .as_str()
                .map(str::to_string)
                .or_else(|| series_id.as_number().map(|n| n.to_string()))?;
            // The id ends up in the product-list query string: digits only.
            official_meta::is_digit_id(&id).then_some(OfficialHit { id, title })
        })
        .collect()
}

/// One product-list page reduced to what the chapter mapper keeps.
struct ProductRow {
    product_id: i64,
    title: String,
    release_date: Option<NaiveDate>,
}

struct ProductPage {
    rows: Vec<ProductRow>,
    series_title: Option<String>,
    /// `cursor_index` to resume from; `None` when the walk is done.
    next_cursor: Option<i64>,
}

/// GET the guest product list page by page, following the cursor. Cached for
/// `META_TTL` so a drawer open does not re-walk the whole list every time.
pub async fn fetch_meta(client: &reqwest::Client, series_id: &str) -> anyhow::Result<OfficialMeta> {
    // The id lands in the query string: only digits are ever valid (search
    // returns numeric series ids), so a malformed binding cannot smuggle params.
    if !official_meta::is_digit_id(series_id) {
        return Ok(build_meta(Vec::new(), None));
    }
    let now = Instant::now();
    if let Some(meta) = meta_cache()
        .lock()
        .expect("kakao meta cache poisoned")
        .get(series_id, now)
    {
        return Ok(meta);
    }
    let meta = fetch_meta_uncached(client, series_id).await?;
    meta_cache()
        .lock()
        .expect("kakao meta cache poisoned")
        .put(series_id, meta.clone(), now);
    Ok(meta)
}

/// The uncached cursor walk behind [`fetch_meta`].
async fn fetch_meta_uncached(
    client: &reqwest::Client,
    series_id: &str,
) -> anyhow::Result<OfficialMeta> {
    let mut rows: Vec<ProductRow> = Vec::new();
    let mut series_title: Option<String> = None;
    let mut cursor: Option<i64> = None;
    for _ in 0..MAX_PRODUCT_PAGES {
        let mut url = url::Url::parse(KAKAO_PRODUCT_LIST)?;
        url.query_pairs_mut()
            .append_pair("series_id", series_id)
            .append_pair("window_size", &PRODUCT_WINDOW.to_string());
        if let Some(cursor_index) = cursor {
            url.query_pairs_mut()
                .append_pair("cursor_index", &cursor_index.to_string())
                .append_pair("cursor_direction", "NEXT");
        }
        let response = client
            .get(url)
            .header(reqwest::header::ACCEPT, "application/json")
            .header(reqwest::header::ORIGIN, "https://page.kakao.com")
            .header(reqwest::header::REFERER, "https://page.kakao.com/")
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?;
        // A hostile or broken page must not be buffered whole.
        if response
            .content_length()
            .is_some_and(|len| len > official_meta::MAX_DETAIL_BYTES)
        {
            tracing::warn!(series_id, "kakao: product page too large, stopping");
            break;
        }
        let page = parse_product_list(&response.text().await?);
        if page.rows.is_empty() {
            break;
        }
        if series_title.is_none() {
            series_title = page.series_title.clone();
        }
        cursor = advance(cursor, page.next_cursor);
        rows.extend(page.rows);
        if cursor.is_none() {
            break;
        }
    }
    Ok(build_meta(rows, series_title.as_deref()))
}

/// Next cursor to request; `None` ends the walk. A page that does not move the
/// cursor forward also ends it — a buggy or hostile endpoint must not be
/// re-fetched up to the page cap.
fn advance(current: Option<i64>, next: Option<i64>) -> Option<i64> {
    match (current, next) {
        (_, None) => None,
        (Some(cur), Some(next)) if next == cur => None,
        (_, Some(next)) => Some(next),
    }
}

/// Pure parser for one `product/list` page. Malformed input degrades to an
/// empty page - never a panic.
fn parse_product_list(body: &str) -> ProductPage {
    let empty = ProductPage {
        rows: Vec::new(),
        series_title: None,
        next_cursor: None,
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return empty;
    };
    let Some(result) = value.get("result") else {
        return empty;
    };
    let series_title = result
        .get("series_item")
        .and_then(|s| s.get("title"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let has_next = result
        .get("has_next")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut rows = Vec::new();
    let mut max_cursor: Option<i64> = None;
    if let Some(list) = result.get("list").and_then(|l| l.as_array()) {
        for entry in list.iter().take(MAX_ROWS_PER_PAGE) {
            let Some(cursor_index) = entry.get("cursor_index").and_then(|v| v.as_i64()) else {
                continue;
            };
            max_cursor = Some(max_cursor.map_or(cursor_index, |m| m.max(cursor_index)));
            let Some(item) = entry.get("item") else {
                continue;
            };
            let Some(product_id) = item.get("product_id").and_then(|v| v.as_i64()) else {
                continue;
            };
            let Some(title) = item
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let release_date = item
                .get("last_release_dt")
                .and_then(|v| v.as_str())
                .and_then(|s| s.get(..10))
                .and_then(official_meta::parse_date);
            rows.push(ProductRow {
                product_id,
                title: title.to_string(),
                release_date,
            });
        }
    }
    ProductPage {
        rows,
        series_title,
        next_cursor: has_next.then_some(max_cursor).flatten(),
    }
}

/// Dedupe by product id, order oldest-first and map titles onto the shared
/// chapter model. Kakao repeats the free-episode block, so the dedupe is
/// mandatory; the series-name prefix is stripped from the stored title and a
/// bare `N화` remainder becomes no title at all.
fn build_meta(rows: Vec<ProductRow>, series_title: Option<&str>) -> OfficialMeta {
    let mut seen: HashSet<i64> = HashSet::new();
    let mut unique: Vec<ProductRow> = rows
        .into_iter()
        .filter(|row| seen.insert(row.product_id))
        .collect();
    unique.sort_by(|a, b| {
        a.release_date
            .cmp(&b.release_date)
            .then_with(|| a.product_id.cmp(&b.product_id))
    });
    let episodes: Vec<EpisodeRow> = unique
        .into_iter()
        .map(|row| {
            let kind = classify_kakao_title(&row.title);
            EpisodeRow {
                title: display_title(&row.title, series_title, kind),
                release_date: row.release_date,
                kind,
            }
        })
        .collect();
    let chapters = chapter_title::rows_to_chapters(episodes);
    let main = main_count(&chapters);
    if !chapters.is_empty() {
        tracing::debug!(
            source = "kakao",
            rows = chapters.len(),
            main,
            bonus = chapters.len() - main,
            "product list parsed"
        );
    }
    OfficialMeta {
        source: "kakao".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        update_schedule: None,
        next_update_at: None,
    }
}

/// `<series> 132화` -> `None` (the number is already rendered as the chapter
/// label); any other remainder (`프롤로그`, a side-story name) is kept.
fn display_title(raw: &str, series_title: Option<&str>, kind: ShortTitle) -> Option<String> {
    let trimmed = raw.trim();
    let remainder = match series_title {
        Some(series) => trimmed
            .strip_prefix(series)
            .map(str::trim_start)
            .unwrap_or(trimmed),
        None => trimmed,
    };
    let remainder = remainder.trim();
    if remainder.is_empty() {
        return None;
    }
    // A bare `N화` carries no information beyond the chapter number.
    if matches!(kind, ShortTitle::Base(_)) && parse_kr_chapter_number(remainder).is_some() {
        return None;
    }
    Some(remainder.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/kakao_search.json");
    const PRODUCT_FIXTURE: &str = include_str!("../../../tests/fixtures/kakao_product_list.json");

    #[test]
    fn parses_fixture() {
        let hits = parse_search(FIXTURE);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "64096846");
        assert_eq!(hits[0].title, "용사파티");
    }

    #[test]
    fn absent_result_is_empty() {
        assert!(parse_search(r#"{"x":1}"#).is_empty());
    }

    #[test]
    fn empty_list_is_empty() {
        assert!(parse_search(r#"{"result":{"list":[]}}"#).is_empty());
    }

    #[test]
    fn rejects_non_numeric_series_id() {
        let body = r#"{"result":{"list":[{"series_id":"abc","title":"T","category_uid":10}]}}"#;
        assert!(parse_search(body).is_empty());
        // A numeric string id is accepted.
        let body =
            r#"{"result":{"list":[{"series_id":"64096846","title":"T","category_uid":10}]}}"#;
        let hits = parse_search(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "64096846");
    }

    #[test]
    fn filters_out_web_novel_listed_first() {
        // Same title as both a web novel (listed first) and a webtoon.
        let body = r#"{"result":{"list":[
            {"series_id":58736258,"title":"용사파티","category_uid":11,"category":"웹소설"},
            {"series_id":64096846,"title":"용사파티","category_uid":10,"category":"웹툰"}
        ]}}"#;
        let hits = parse_search(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "64096846");
        assert_eq!(hits[0].title, "용사파티");
    }

    #[test]
    fn skips_entries_without_numeric_category_uid() {
        let body = r#"{"result":{"list":[
            {"series_id":1,"title":"T"},
            {"series_id":2,"title":"U","category_uid":"10"},
            {"series_id":3,"title":"V","category_uid":10}
        ]}}"#;
        let hits = parse_search(body);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "3");
        assert_eq!(hits[0].title, "V");
    }

    #[test]
    fn mixed_list_keeps_only_webtoons() {
        let body = r#"{"result":{"list":[
            {"series_id":1,"title":"A","category_uid":10},
            {"series_id":2,"title":"B","category_uid":11},
            {"series_id":3,"title":"C","category_uid":16},
            {"series_id":4,"title":"D","category_uid":10}
        ]}}"#;
        let hits = parse_search(body);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "1");
        assert_eq!(hits[1].id, "4");
    }

    #[test]
    fn product_list_fixture_parses() {
        let page = parse_product_list(PRODUCT_FIXTURE);
        assert_eq!(
            page.series_title.as_deref(),
            Some("용사파티 대마법사의 환생")
        );
        assert_eq!(page.rows.len(), 25);
        assert!(page.next_cursor.is_some(), "more pages follow");
        // The first page is the oldest block, numbered from the title.
        let meta = build_meta(page.rows, page.series_title.as_deref());
        assert_eq!(meta.source, "kakao");
        // 25 products: the `0화` prologue is a bonus, 1..24 are main chapters.
        assert_eq!(meta.count, Some(24));
        assert_eq!(meta.chapters.iter().filter(|c| c.is_bonus).count(), 1);
        assert!(
            meta.chapters
                .iter()
                .filter(|c| !c.is_bonus)
                .all(|c| c.number_x100 > 0)
        );
        // The stored title drops the redundant `<series> N화` remainder on
        // main rows; the bonus prologue keeps its `0화` label text.
        let main_titles: Vec<&Option<String>> = meta
            .chapters
            .iter()
            .filter(|c| !c.is_bonus)
            .map(|c| &c.title)
            .collect();
        assert!(main_titles.iter().all(|t| t.is_none()));
        assert!(
            meta.chapters
                .iter()
                .any(|c| c.is_bonus && c.title.as_deref() == Some("0화"))
        );
    }

    #[test]
    fn product_page_malformed_is_empty() {
        let page = parse_product_list("{not json");
        assert!(page.rows.is_empty());
        assert!(page.next_cursor.is_none());
        assert!(page.series_title.is_none());
    }

    #[test]
    fn product_page_caps_rows() {
        let list: Vec<String> = (0..(MAX_ROWS_PER_PAGE + 50))
            .map(|i| {
                format!(
                    r#"{{"cursor_index":{i},"item":{{"product_id":{i},"title":"제목 {i}화"}}}}"#
                )
            })
            .collect();
        let body = format!(
            r#"{{"result":{{"has_next":true,"list":[{}]}}}}"#,
            list.join(",")
        );
        let page = parse_product_list(&body);
        assert_eq!(page.rows.len(), MAX_ROWS_PER_PAGE);
    }

    #[test]
    fn cursor_advance_stops_on_missing_or_repeated_cursor() {
        assert_eq!(advance(None, Some(24)), Some(24));
        assert_eq!(advance(Some(24), Some(120)), Some(120));
        assert_eq!(advance(Some(24), None), None);
        assert_eq!(advance(Some(24), Some(24)), None, "no forward progress");
    }

    #[test]
    fn ttl_cache_expires() {
        let mut cache = TtlCache::new(Duration::from_secs(60));
        let now = Instant::now();
        cache.put(
            "k",
            vec![OfficialHit {
                id: "1".into(),
                title: "T".into(),
            }],
            now,
        );
        assert!(cache.get("k", now + Duration::from_secs(30)).is_some());
        assert!(cache.get("k", now + Duration::from_secs(61)).is_none());
        assert!(cache.get("other", now).is_none());
    }

    #[test]
    fn build_meta_dedupes_and_classifies() {
        let rows = vec![
            ProductRow {
                product_id: 3,
                title: "제목 3화".into(),
                release_date: official_meta::parse_date("2024-03-03"),
            },
            // The free block repeats episode 3 — the duplicate must fold.
            ProductRow {
                product_id: 3,
                title: "제목 3화".into(),
                release_date: official_meta::parse_date("2024-03-03"),
            },
            ProductRow {
                product_id: 1,
                title: "제목 1화".into(),
                release_date: official_meta::parse_date("2024-03-01"),
            },
            ProductRow {
                product_id: 2,
                title: "제목 프롤로그".into(),
                release_date: official_meta::parse_date("2024-02-01"),
            },
        ];
        let meta = build_meta(rows, Some("제목"));
        // 3 unique products: two main chapters + one bonus prologue.
        assert_eq!(meta.chapters.len(), 3);
        assert_eq!(meta.count, Some(3));
        let keys: Vec<i32> = meta.chapters.iter().map(|c| c.number_x100).collect();
        assert_eq!(keys, vec![0, 100, 300]);
        let bonus: Vec<bool> = meta.chapters.iter().map(|c| c.is_bonus).collect();
        assert_eq!(bonus, vec![true, false, false]);
        assert_eq!(meta.chapters[0].title.as_deref(), Some("프롤로그"));
    }
}
