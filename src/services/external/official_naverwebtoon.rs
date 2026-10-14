//! Naver Webtoon KR (comic.naver.com) official-source title search + chapter
//! metadata.
//!
//! Two public JSON endpoints (browser UA, no keys or cookies):
//! - `/api/search/all?keyword=` — the `searchWebtoonResult` section lists the
//!   licensed webtoons (`titleId` + `titleName`). The sibling sections are
//!   user uploads (`CHALLENGE`/`BEST_CHALLENGE`) and the paid Nbooks store,
//!   which the existing `naver` source (series.naver.com) already covers.
//! - `/api/article/list?titleId=&page=&sort=ASC` — 20 episodes per page with
//!   the platform's own episode number (`no`, the `detail?no=` id), the
//!   episode subtitle and its service date; `pageInfo.totalPages` bounds the
//!   walk and `totalCount` proves it complete.
//!
//! Episode numbers are kept verbatim (the `no` a reader sees in the episode
//! URL), exactly like WEBTOON's `episodeNo`; side content (`예고편`, `외전`,
//! `후기`, …) is classified through the shared KR marker list into bonuses.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use chrono::NaiveDate;

use crate::services::external::chapter_title::{self, is_kr_bonus_title, strip_catalog_markers};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const SEARCH_BASE: &str = "https://comic.naver.com/api/search/all";
const LIST_BASE: &str = "https://comic.naver.com/api/article/list";

/// Browser-like User-Agent; the endpoints reject obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Walk cap (100 pages x 20 rows = 2000 episodes); a longer series is an
/// error, never a truncated answer.
const MAX_PAGES: i64 = 100;
/// Pages fetched concurrently. The walk runs inline in the drawer request, so
/// a 600-episode series (30 pages) must not cost 30 sequential round-trips;
/// batches of 8 stay a polite load for the CDN.
const PAGE_BATCH: i64 = 8;
/// Rows read from one page (the endpoint serves 20; a longer answer is a
/// broken one, not a reason to grow the walk).
const ROWS_PER_PAGE: usize = 20;
/// Upper bound on a plausible episode number; anything larger is a broken or
/// hostile answer.
const MAX_EPISODE_NO: i64 = 100_000;
/// Defensive cap on one search answer (only the webtoon section is read).
const SEARCH_MAX_HITS: usize = 50;
/// Longest stored episode title (a hostile answer must not bloat the DB).
const MAX_TITLE_CHARS: usize = 512;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "naverwebtoon".to_string(),
        ..OfficialMeta::default()
    }
}

/// GET `/api/search/all?keyword=<urlencoded>`.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.query_pairs_mut().append_pair("keyword", title);
    let body = get_json(client, url).await?;
    Ok(parse_search(&body))
}

/// Pure parser over the search answer: the `searchWebtoonResult` section only
/// (licensed webtoons), each `WEBTOON`-levelled entry with a numeric
/// `titleId`. Malformed JSON yields an empty vector — never a panic.
pub fn parse_search(json: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value
        .pointer("/searchWebtoonResult/searchViewList")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for item in items.iter().take(SEARCH_MAX_HITS) {
        // The section should already be webtoon-only; the level check keeps a
        // user-upload (`CHALLENGE`) entry from ever binding as an official one.
        if item.get("webtoonLevelCode").and_then(|v| v.as_str()) != Some("WEBTOON") {
            continue;
        }
        let Some(id) = item.get("titleId").and_then(title_id_string) else {
            continue;
        };
        if hits.iter().any(|h| h.id == id) {
            continue;
        }
        let Some(name) = item
            .get("titleName")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        // Catalog markers (`(독점)`, `[무료]`) are not part of the title.
        let title = strip_catalog_markers(name).trim();
        if title.is_empty() {
            continue;
        }
        hits.push(OfficialHit {
            id,
            title: title.chars().take(MAX_TITLE_CHARS).collect(),
        });
    }
    hits
}

/// `titleId` as a digit string (the live answers use a JSON number; a string
/// form is accepted defensively).
fn title_id_string(value: &serde_json::Value) -> Option<String> {
    let id = match value {
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => s.clone(),
        _ => return None,
    };
    official_meta::is_digit_id(&id).then_some(id)
}

/// One `/api/article/list` page: its rows plus the walk bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticlePage {
    pub rows: Vec<ArticleRow>,
    pub total_count: i64,
    pub total_pages: i64,
}

/// One episode row before numbering: the platform's episode number, the
/// subtitle, its service date, and whether the subtitle marks side content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleRow {
    pub no: i64,
    pub title: String,
    pub release_date: Option<NaiveDate>,
    pub is_bonus: bool,
}

/// Pure parser over one `/api/article/list` page. `None` when the answer is
/// malformed or misses the walk bounds (`totalCount`/`pageInfo.totalPages`) —
/// a walk without bounds must not run. Rows with an out-of-range `no` or an
/// empty subtitle are dropped; the caller's completeness check then fails and
/// the answer is discarded.
pub fn parse_page(json: &str) -> Option<ArticlePage> {
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let total_count = value.get("totalCount").and_then(|v| v.as_i64())?;
    let total_pages = value
        .pointer("/pageInfo/totalPages")
        .and_then(|v| v.as_i64())?;
    let items = value.get("articleList").and_then(|v| v.as_array())?;
    let mut rows = Vec::with_capacity(items.len().min(ROWS_PER_PAGE));
    for item in items.iter().take(ROWS_PER_PAGE) {
        let Some(no) = item
            .get("no")
            .and_then(|v| v.as_i64())
            .filter(|n| (1..=MAX_EPISODE_NO).contains(n))
        else {
            continue;
        };
        let Some(subtitle) = item
            .get("subtitle")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let title: String = subtitle.chars().take(MAX_TITLE_CHARS).collect();
        let release_date = item
            .get("serviceDateDescription")
            .and_then(|v| v.as_str())
            .and_then(parse_service_date);
        rows.push(ArticleRow {
            no,
            is_bonus: is_kr_bonus_title(&title),
            title,
            release_date,
        });
    }
    Some(ArticlePage {
        rows,
        total_count,
        total_pages,
    })
}

/// `YY.MM.DD` (`25.02.02`) -> the date. The platform's service dates all start
/// in 2004, so the year is `20YY`; an out-of-range value yields `None`.
fn parse_service_date(raw: &str) -> Option<NaiveDate> {
    let mut parts = raw.trim().split('.');
    let yy: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || yy > 99 {
        return None;
    }
    NaiveDate::from_ymd_opt(2000 + yy, month, day)
}

/// Number the rows: the platform's own episode number (`no`) on main rows,
/// `0` for side content (bonus rows carry their label downstream). The walk
/// order is oldest-first (`sort=ASC`), so the rows read chronologically.
pub fn chapters_from_rows(rows: Vec<ArticleRow>) -> Vec<ChapterMeta> {
    rows.into_iter()
        .map(|row| ChapterMeta {
            number_x100: if row.is_bonus {
                0
            } else {
                (row.no * 100) as i32
            },
            title: Some(row.title),
            release_date: row.release_date,
            is_bonus: row.is_bonus,
            label: None,
        })
        .collect()
}

/// Walk the episode list oldest-first and map it onto the shared chapter
/// model. The walk is accepted only when it returned exactly `totalCount`
/// rows — a partial answer must never reach `apply_meta`, whose orphan
/// cleanup would drop rows the answer never saw. Pages are fetched in
/// batches of [`PAGE_BATCH`]; a failed page fails the whole walk.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    // The id lands in the query string: only digits are ever valid.
    if !official_meta::is_digit_id(source_id) {
        return Ok(empty_meta());
    }
    let first = fetch_page(client, source_id, 1).await?;
    if first.total_count <= 0 {
        return Ok(empty_meta());
    }
    if first.total_pages > MAX_PAGES {
        anyhow::bail!(
            "naverwebtoon: series too long ({} pages)",
            first.total_pages
        );
    }
    let mut rows = first.rows;
    let mut page = 2i64;
    while page <= first.total_pages {
        let last = (page + PAGE_BATCH - 1).min(first.total_pages);
        let mut set = tokio::task::JoinSet::new();
        for batch_page in page..=last {
            let client = client.clone();
            let title_id = source_id.to_string();
            set.spawn(
                async move { (batch_page, fetch_page(&client, &title_id, batch_page).await) },
            );
        }
        let mut batch: Vec<(i64, ArticlePage)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((batch_page, Ok(parsed))) => batch.push((batch_page, parsed)),
                Ok((batch_page, Err(e))) => {
                    anyhow::bail!("naverwebtoon: page {batch_page} failed: {e}");
                }
                Err(e) => anyhow::bail!("naverwebtoon: page task failed: {e}"),
            }
        }
        // The join order is arbitrary; the rows must stay oldest-first.
        batch.sort_by_key(|(batch_page, _)| *batch_page);
        for (_, parsed) in batch {
            rows.extend(parsed.rows);
        }
        page = last + 1;
    }
    if rows.len() as i64 != first.total_count {
        anyhow::bail!(
            "naverwebtoon: episode walk incomplete ({}/{} rows)",
            rows.len(),
            first.total_count
        );
    }
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "naverwebtoon".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    })
}

/// One episode page (oldest-first).
async fn fetch_page(
    client: &reqwest::Client,
    title_id: &str,
    page: i64,
) -> anyhow::Result<ArticlePage> {
    let mut url = url::Url::parse(LIST_BASE)?;
    url.query_pairs_mut()
        .append_pair("titleId", title_id)
        .append_pair("page", &page.to_string())
        .append_pair("sort", "ASC");
    let body = get_json(client, url).await?;
    parse_page(&body)
        .ok_or_else(|| anyhow::anyhow!("naverwebtoon: unparseable episode page {page}"))
}

/// GET a JSON endpoint and read the body with a hard size cap.
async fn get_json(client: &reqwest::Client, url: url::Url) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/naverwebtoon_search.json");
    const LIST_FIXTURE: &str = include_str!("../../../tests/fixtures/naverwebtoon_list.json");
    const SIDES_FIXTURE: &str =
        include_str!("../../../tests/fixtures/naverwebtoon_list_sides.json");

    #[test]
    fn search_fixture_yields_webtoon_hits() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "703846");
        assert_eq!(hits[0].title, "여신강림");
        assert_eq!(hits[1].id, "833705");
        assert_eq!(hits[1].title, "귀혼");
    }

    #[test]
    fn search_drops_challenge_and_malformed_entries() {
        let json = r#"{"searchWebtoonResult":{"totalCount":3,"searchViewList":[
            {"titleId":1,"titleName":"정식 연재","webtoonLevelCode":"WEBTOON"},
            {"titleId":2,"titleName":"유저 업로드","webtoonLevelCode":"CHALLENGE"},
            {"titleId":"bad","titleName":"슬래시","webtoonLevelCode":"WEBTOON"},
            {"titleId":4,"titleName":"  ","webtoonLevelCode":"WEBTOON"}
        ]}}"#;
        let hits = parse_search(json);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "1");
        // The id may arrive as a string.
        let stringy = r#"{"searchWebtoonResult":{"searchViewList":[
            {"titleId":"99","titleName":"문자열","webtoonLevelCode":"WEBTOON"}]}}"#;
        assert_eq!(parse_search(stringy)[0].id, "99");
        // Malformed and unrelated answers are empty, never a panic.
        assert!(parse_search("").is_empty());
        assert!(parse_search("{}").is_empty());
        assert!(parse_search("<html>").is_empty());
        assert!(parse_search(r#"{"searchWebtoonResult":{"searchViewList":[]}}"#).is_empty());
    }

    #[test]
    fn page_fixture_parses_rows_and_bounds() {
        let page = parse_page(LIST_FIXTURE).expect("fixture parses");
        assert_eq!(page.total_count, 59);
        assert_eq!(page.total_pages, 3);
        assert_eq!(page.rows.len(), 20);
        // The prologue is side content; the first real episode is not.
        assert_eq!(page.rows[0].no, 1);
        assert_eq!(page.rows[0].title, "프롤로그");
        assert!(page.rows[0].is_bonus);
        assert_eq!(
            page.rows[0].release_date,
            official_meta::parse_date("2024-10-28"),
            "serviceDateDescription is YY.MM.DD"
        );
        assert_eq!(page.rows[1].no, 2);
        assert_eq!(page.rows[1].title, "1화");
        assert!(!page.rows[1].is_bonus);
        // Malformed pages are None, never a panic.
        assert_eq!(parse_page(""), None);
        assert_eq!(parse_page("{}"), None);
        assert_eq!(parse_page(r#"{"totalCount":1}"#), None);
    }

    #[test]
    fn page_side_stories_are_bonus_rows() {
        let page = parse_page(SIDES_FIXTURE).expect("fixture parses");
        assert_eq!(page.total_count, 261);
        assert_eq!(page.total_pages, 14);
        // 216화..마지막화 are main rows, the 외전 run is side content.
        let main = page.rows.iter().filter(|r| !r.is_bonus).count();
        let bonus = page.rows.iter().filter(|r| r.is_bonus).count();
        assert_eq!((main, bonus), (8, 12));
        let last_main = page
            .rows
            .iter()
            .find(|r| r.title == "마지막화")
            .expect("final chapter row");
        assert_eq!(last_main.no, 228);
        assert!(!last_main.is_bonus, "마지막화 is the final chapter");
        let side = page
            .rows
            .iter()
            .find(|r| r.title == "외전-1화 [희경ep]")
            .expect("side-story row");
        assert!(side.is_bonus);
    }

    #[test]
    fn chapters_keep_source_numbers_and_zero_the_bonus() {
        let page = parse_page(SIDES_FIXTURE).expect("fixture parses");
        let chapters = chapters_from_rows(page.rows);
        let mains: Vec<i32> = chapters
            .iter()
            .filter(|c| !c.is_bonus)
            .map(|c| c.number_x100)
            .collect();
        assert_eq!(mains, (221..=228).map(|n| n * 100).collect::<Vec<_>>());
        assert!(
            chapters
                .iter()
                .filter(|c| c.is_bonus)
                .all(|c| c.number_x100 == 0)
        );
        assert_eq!(chapter_title::main_count(&chapters), 228);
    }

    #[test]
    fn service_date_parses_two_digit_years() {
        assert_eq!(
            parse_service_date("25.02.02"),
            official_meta::parse_date("2025-02-02")
        );
        assert_eq!(
            parse_service_date("10.06.30"),
            official_meta::parse_date("2010-06-30")
        );
        for raw in [
            "",
            "25.2",
            "xx.yy.zz",
            "25.13.01",
            "25.02.02.03",
            "125.02.02",
        ] {
            assert_eq!(parse_service_date(raw), None, "{raw:?}");
        }
    }

    #[tokio::test]
    async fn non_numeric_title_id_is_empty() {
        let client = crate::services::external::http_client();
        let meta = fetch_meta(&client, "abc")
            .await
            .expect("a bad id must not hit the network");
        assert!(meta.chapters.is_empty());
        assert!(meta.count.is_none());
    }
}
