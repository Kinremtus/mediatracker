//! Naver Series (series.naver.com) official-source title search.
//!
//! The search endpoint is public and keyless (no Referer/Origin needed; a
//! browser User-Agent is enough). `fs=comic` does NOT actually filter web
//! novels out, so the parser keeps ONLY links whose href starts with
//! `/comic/detail.series?productNo=`; the same title may arrive in the
//! `웹소설` (web novel) section and must be dropped.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use crate::services::external::chapter_title::is_kr_bonus_title;
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

pub const NAVER_SEARCH_BASE: &str = "https://series.naver.com/search/search.series";

const NAVER_VOLUME_LIST: &str = "https://series.naver.com/comic/volumeList.series";
const NAVER_NOTICE_AJAX: &str = "https://m.series.naver.com/comic/moreNotiDetail.series";

/// Browser-like User-Agent; the endpoint rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Volume-list pages to walk (30 rows each) and a per-page row cap, so a
/// hostile or buggy `resultData` cannot blow up memory or DB writes.
const NAVER_VOLUME_PAGES: u32 = 20;
const NAVER_ROWS_PER_PAGE: usize = 60;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "naver".to_string(),
        ..OfficialMeta::default()
    }
}

/// GET https://series.naver.com/search/search.series?fs=comic&q=<urlencoded>
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(NAVER_SEARCH_BASE)?;
    url.query_pairs_mut()
        .append_pair("fs", "comic")
        .append_pair("q", title);
    let body = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_search(&body))
}

/// Pure parser, unit-testable without network. Manual string scanning (zero
/// deps, no regex crate).
///
/// Keeps only `/comic/detail.series?productNo=<digits>` anchors with non-empty
/// link text; strips the parasitic ` (총 N화/…)` suffix; dedups by productNo.
/// Malformed / truncated HTML yields an empty vector - never a panic.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    // Only the search-results container is relevant; text before/after (LNB,
    // footer) is ignored. `class="com_srch"` matches the container div only
    // (`com_srch_nav` has a different closing quote). Fall back to the whole
    // document if Naver ever drops the closing comment.
    let container = match html.find("class=\"com_srch\"") {
        Some(pos) => {
            let rest = &html[pos..];
            match rest.find("<!-- com_srch -->") {
                Some(end) => &rest[..end],
                None => rest,
            }
        }
        None => html,
    };

    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut rest = container;
    while let Some(a) = rest.find("<a") {
        let after = &rest[a..];
        let Some(gt) = after.find('>') else { break };
        let tag = &after[..gt];
        let body = &after[gt + 1..];
        let Some(end) = body.find("</a>") else { break };
        let inner = &body[..end];
        rest = &body[end + 4..];

        let Some(id) = href_product_no(tag) else {
            continue;
        };
        if hits.iter().any(|h| h.id == id) {
            continue;
        }
        let title = clean_title(inner);
        if title.is_empty() {
            continue;
        }
        hits.push(OfficialHit { id, title });
    }
    hits
}

/// `/comic/detail.series?productNo=10319749` -> `Some("10319749")`.
/// The `/comic/` prefix is what filters out the `/novel/` trap.
fn href_product_no(tag: &str) -> Option<String> {
    let start = tag.find("href=\"")? + "href=\"".len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    let href = &rest[..end];
    let marker = "/comic/detail.series?productNo=";
    let idx = href.find(marker)? + marker.len();
    let digits: String = href[idx..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (!digits.is_empty()).then_some(digits)
}

/// Strip tags, collapse runs of whitespace, then cut the ` (총 N화/…)` suffix.
fn clean_title(inner: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in inner.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match collapsed.find("(총 ") {
        Some(i) => collapsed[..i].trim_end().to_string(),
        None => collapsed,
    }
}

/// Fetch up to `NAVER_VOLUME_PAGES` volume pages, stopping at the first empty
/// page. Chapters are renumbered globally so page 2 continues where page 1
/// ended. The schedule comes from the notice AJAX fragment: a volume-fetch
/// failure propagates to the caller (a silent no-op there) while a notice
/// failure only drops the schedule.
pub async fn fetch_meta(
    client: &reqwest::Client,
    product_no: &str,
) -> anyhow::Result<OfficialMeta> {
    // The id lands in the query string: only digits are ever valid.
    if !official_meta::is_digit_id(product_no) {
        return Ok(empty_meta());
    }
    let mut chapters: Vec<ChapterMeta> = Vec::new();
    let mut schedule: Option<String> = None;

    for page in 1..=NAVER_VOLUME_PAGES {
        let mut url = url::Url::parse(NAVER_VOLUME_LIST)?;
        url.query_pairs_mut()
            .append_pair("productNo", product_no)
            .append_pair("page", &page.to_string());
        let response = client
            .get(url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?;
        if response
            .content_length()
            .is_some_and(|len| len > official_meta::MAX_DETAIL_BYTES)
        {
            tracing::warn!(product_no, "naver: volume page too large, stopping");
            break;
        }
        let rows = parse_volume_list(&response.text().await?);
        if rows.is_empty() {
            break;
        }
        chapters.extend(rows);
    }

    // Number the MAIN rows sequentially across pages; bonus rows (previews,
    // side stories, notices) never consume a chapter number. `checked_mul`
    // keeps a hostile multi-million-row response from wrapping into negative
    // or duplicate numbers.
    let mut main_i = 0i32;
    for chapter in chapters.iter_mut() {
        if chapter.is_bonus {
            chapter.number_x100 = 0;
        } else if let Some(n) = main_i.checked_add(1).and_then(|i| i.checked_mul(100)) {
            main_i += 1;
            chapter.number_x100 = n;
        } else {
            break;
        }
    }
    if !chapters.is_empty() {
        tracing::debug!(
            source = "naver",
            rows = chapters.len(),
            main = main_i,
            "volume list parsed"
        );
    }

    let mut url = url::Url::parse(NAVER_NOTICE_AJAX)?;
    url.query_pairs_mut().append_pair("productNo", product_no);
    let notice = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await
        .ok()
        .and_then(|r| r.error_for_status().ok());
    if let Some(resp) = notice
        && let Ok(body) = resp.text().await
    {
        schedule = parse_naver_notice(&body);
    }

    Ok(OfficialMeta {
        source: "naver".to_string(),
        count: (main_i > 0).then_some(main_i),
        chapters,
        update_schedule: schedule,
        next_update_at: None,
    })
}

/// Parse the volumeList AJAX response. The live endpoint returns JSON, not
/// HTML: `{"resultData":[{"productName":..,"volumnNameText":"1화",..},..]}`.
/// The title is the episode name (`volumnNameText`, fallback `volumeName`,
/// then `productName`); the release date is the `YYYY-MM-DD` prefix of
/// `lastVolumeUpdateDate`. Numbers are assigned globally in [`fetch_meta`] -
/// rows here carry `number_x100 = 0`. At most `NAVER_ROWS_PER_PAGE` rows are
/// read. Malformed or truncated input yields an empty vector - never a panic.
pub fn parse_volume_list(json: &str) -> Vec<ChapterMeta> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.get("resultData").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for item in items.iter().take(NAVER_ROWS_PER_PAGE) {
        let Some(title) = episode_title(item) else {
            continue;
        };
        let date = item
            .get("lastVolumeUpdateDate")
            .and_then(|v| v.as_str())
            .and_then(|s| s.get(..10))
            .and_then(official_meta::parse_date);
        rows.push(ChapterMeta {
            number_x100: 0,
            is_bonus: is_kr_bonus_title(&title),
            title: Some(title),
            release_date: date,
            label: None,
        });
    }
    rows
}

/// Episode name from a volumeList JSON row: the first non-empty of
/// `volumnNameText` (the real episode name: `1화`, `1부-1화`, `예고편`),
/// `volumeName`, then the series name `productName`. The emptiness check runs
/// per candidate — an empty `volumnNameText` must not shadow the fallback.
fn episode_title(item: &serde_json::Value) -> Option<String> {
    ["volumnNameText", "volumeName", "productName"]
        .iter()
        .find_map(|key| {
            item.get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

/// Schedule text from a notice block. The marker is `연재 공지` (Naver encodes
/// the space as `&nbsp;`, so it is replaced before the search). A `_notice`
/// block without that marker (e.g. `<li class="_notice">이용안내</li>`) is not a
/// schedule and returns `None`.
pub fn parse_naver_notice(html: &str) -> Option<String> {
    let notice = official_meta::between(html, "class=\"_notice\"", "</li>").unwrap_or(html);
    let spaced = notice.replace("&nbsp;", " ");
    let text = official_meta::strip_tags(&spaced);
    let idx = text.find("연재 공지")?;
    let after = &text[idx..];
    let rest = after.split_once(':').map(|(_, tail)| tail).unwrap_or(after);
    official_meta::normalize_schedule(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../../tests/fixtures/naver_search.html");

    #[test]
    fn parses_fixture_only_comic() {
        let hits = parse_search(FIXTURE);
        assert_eq!(hits.len(), 1, "web-novel section must be filtered out");
        assert_eq!(hits[0].id, "10319749");
    }

    #[test]
    fn strips_count_suffix() {
        let hits = parse_search(FIXTURE);
        assert_eq!(hits[0].title, "왕의 힘으로 회귀한다");
        assert!(!hits[0].title.contains("총"));
        assert!(!hits[0].title.contains("162"));
    }

    #[test]
    fn dedups_image_and_title_anchors() {
        let html = "<div class=\"com_srch\">\
            <a href=\"/comic/detail.series?productNo=1\" class=\"pic\"></a>\
            <a href=\"/comic/detail.series?productNo=1\" class=\"t\">제목 (총 5화/완결)</a>\
        </div>";
        let hits = parse_search(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "1");
        assert_eq!(hits[0].title, "제목");
    }

    #[test]
    fn novel_only_is_empty() {
        let html = "<div class=\"com_srch\">\
            <a href=\"/novel/detail.series?productNo=5108800\">소설 (총 742화/완결)</a>\
        </div>";
        assert!(parse_search(html).is_empty());
    }

    #[test]
    fn title_without_suffix_kept() {
        let html = "<div class=\"com_srch\">\
            <a href=\"/comic/detail.series?productNo=7\">제목</a>\
        </div>";
        let hits = parse_search(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "제목");
    }

    #[test]
    fn ignores_links_outside_container() {
        let html = "<a href=\"/comic/detail.series?productNo=9\">밖 (총 1화/완결)</a>\
            <div class=\"com_srch\"></div>";
        assert!(parse_search(html).is_empty());
    }

    #[test]
    fn empty_and_truncated_are_empty() {
        assert!(parse_search("").is_empty());
        let cut = FIXTURE.find("com_srch").unwrap_or(0);
        assert!(parse_search(&FIXTURE[..cut]).is_empty());
    }

    #[test]
    fn parse_volume_list_fixture() {
        let html = include_str!("../../../tests/fixtures/naver_volume_list.html");
        let rows = parse_volume_list(html);
        assert!(!rows.is_empty());
        // Numbers are assigned in `fetch_meta`; rows carry the episode name.
        assert!(rows.iter().all(|r| r.number_x100 == 0));
        assert!(
            rows.iter()
                .all(|r| r.title.as_deref().is_some_and(|t| !t.is_empty()))
        );
        // The preview row is a bonus, a `1부-1화` part row is not.
        let preview = rows
            .iter()
            .find(|r| r.title.as_deref() == Some("예고편"))
            .expect("preview row");
        assert!(preview.is_bonus);
        let part = rows
            .iter()
            .find(|r| r.title.as_deref() == Some("1부-1화"))
            .expect("part row");
        assert!(!part.is_bonus);
    }

    #[test]
    fn parse_naver_notice_fixture() {
        let html = include_str!("../../../tests/fixtures/naver_notice.html");
        let s = parse_naver_notice(html).expect("schedule");
        assert!(s.contains("매주"), "got {s}");
    }

    #[test]
    fn notice_without_schedule_is_none() {
        assert_eq!(
            parse_naver_notice("<li class=\"_notice\">이용안내</li>"),
            None
        );
    }

    #[test]
    fn truncated_volume_list_no_panic() {
        assert!(parse_volume_list("<li><a>hi").is_empty());
    }

    #[test]
    fn empty_volume_name_falls_back() {
        let body = r#"{"resultData":[{"volumnNameText":"","volumeName":"1화",
            "productName":"제목","lastVolumeUpdateDate":"2024-01-02 00:00:00"}]}"#;
        let rows = parse_volume_list(body);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title.as_deref(), Some("1화"));
        // An all-empty name chain drops the row.
        let body = r#"{"resultData":[{"volumnNameText":" ","volumeName":"","productName":""}]}"#;
        assert!(parse_volume_list(body).is_empty());
    }

    #[test]
    fn volume_page_caps_rows() {
        let items: Vec<String> = (0..(NAVER_ROWS_PER_PAGE + 10))
            .map(|i| format!(r#"{{"volumnNameText":"{i}화","productName":"제목"}}"#))
            .collect();
        let body = format!(r#"{{"resultData":[{}]}}"#, items.join(","));
        assert_eq!(parse_volume_list(&body).len(), NAVER_ROWS_PER_PAGE);
    }

    #[tokio::test]
    async fn non_numeric_product_no_is_empty() {
        let client = crate::services::external::http_client();
        let meta = fetch_meta(&client, "abc")
            .await
            .expect("a bad id must not hit the network");
        assert!(meta.chapters.is_empty());
        assert!(meta.count.is_none());
    }
}
