//! Naver Series (series.naver.com) official-source title search.
//!
//! The search endpoint is public and keyless (no Referer/Origin needed; a
//! browser User-Agent is enough). The parser keeps comic links
//! (`/comic/detail.series?productNo=`, bare id) and web-novel links
//! (`/novel/detail.series?productNo=`, `novel/`-prefixed id); the kind is
//! filtered downstream by `official_resolve::hit_matches_media_type`.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use crate::services::external::chapter_title::{is_kr_bonus_title, strip_catalog_markers};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

pub const NAVER_SEARCH_BASE: &str = "https://series.naver.com/search/search.series";

const NAVER_VOLUME_LIST: &str = "https://series.naver.com/comic/volumeList.series";
const NAVER_NOVEL_VOLUME_LIST: &str = "https://series.naver.com/novel/volumeList.series";

/// Browser-like User-Agent; the endpoint rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Volume-list pages to walk (the endpoint serves 30 rows per page) plus a
/// defensive per-page row cap, so a
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

/// GET https://series.naver.com/search/search.series?fs=<kind>&q=<urlencoded>
/// for both `comic` and `novel`; the parser prefixes novel ids with `novel/`
/// so the resolve walk can tell the kinds apart.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut hits = search_kind(client, title, "comic").await?;
    hits.extend(search_kind(client, title, "novel").await?);
    Ok(hits)
}

/// One `fs=<kind>` search page.
async fn search_kind(
    client: &reqwest::Client,
    title: &str,
    kind: &str,
) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(NAVER_SEARCH_BASE)?;
    url.query_pairs_mut()
        .append_pair("fs", kind)
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
/// Keeps `/comic/detail.series?productNo=<digits>` (bare id) and
/// `/novel/detail.series?productNo=<digits>` (`novel/`-prefixed id) anchors
/// with non-empty link text; strips the parasitic ` (총 N화/…)` suffix; dedups
/// by id. Malformed / truncated HTML yields an empty vector - never a panic.
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
        // The cover link (`class="pic"`) wraps the thumbnail and may carry a
        // reward badge such as `광고보고무료` as its text; the real title anchor
        // follows inside `div.cont`. Skipping the thumbnail wrapper by its
        // class (not by the `<img` it contains) lets the title anchor win the
        // per-id dedup below without risking a title anchor that embeds an
        // inline icon.
        if tag.contains("class=\"pic") {
            continue;
        }
        if hits.iter().any(|h| h.id == id) {
            continue;
        }
        let title = clean_title(inner);
        if title.is_empty() {
            continue;
        }
        hits.push(OfficialHit::new(id, title));
    }
    hits
}

/// `/comic/detail.series?productNo=10319749` -> `Some("10319749")`;
/// `/novel/detail.series?productNo=3400123` -> `Some("novel/3400123")`.
/// The kind prefix keeps a novel hit from binding a comic title (and vice
/// versa); any other path is not a series link.
fn href_product_no(tag: &str) -> Option<String> {
    let start = tag.find("href=\"")? + "href=\"".len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    let href = &rest[..end];
    const MARKER: &str = "/detail.series?productNo=";
    let idx = href.find(MARKER)? + MARKER.len();
    let digits: String = href[idx..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    let path = &href[..idx - MARKER.len()];
    if path.ends_with("/comic") {
        Some(digits)
    } else if path.ends_with("/novel") {
        Some(format!("novel/{digits}"))
    } else {
        None
    }
}

/// Strip tags, collapse runs of whitespace, then cut the ` (총 N화/…)` suffix
/// and trailing catalog markers (`[독점]`, `(완결)`).
fn clean_title(inner: &str) -> String {
    let collapsed = official_meta::strip_tags(inner);
    let cut = match collapsed.find("(총 ") {
        Some(i) => collapsed[..i].trim_end(),
        None => collapsed.as_str(),
    };
    // Catalog markers (`[독점]`, `(완결)`) are not part of the title and would
    // defeat `titles_match` against the clean series title.
    strip_catalog_markers(cut).to_string()
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
    // The id lands in the query string: only digits are ever valid; a novel
    // binding carries a `novel/` prefix that picks the novel endpoints.
    let (kind, product_no) = match product_no.strip_prefix("novel/") {
        Some(id) => ("novel", id),
        None => ("comic", product_no),
    };
    if !official_meta::is_digit_id(product_no) {
        return Ok(empty_meta());
    }
    let mut chapters: Vec<ChapterMeta> = Vec::new();
    let mut schedule: Option<String> = None;

    let volume_base = if kind == "novel" {
        NAVER_NOVEL_VOLUME_LIST
    } else {
        NAVER_VOLUME_LIST
    };
    for page in 1..=NAVER_VOLUME_PAGES {
        let mut url = url::Url::parse(volume_base)?;
        url.query_pairs_mut()
            .append_pair("productNo", product_no)
            .append_pair("page", &page.to_string());
        let response = client
            .get(url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?;
        let Ok(body) = official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
        else {
            tracing::warn!(product_no, "naver: volume page too large, stopping");
            break;
        };
        let rows = parse_volume_list(&body, kind);
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

    let mut url = url::Url::parse(&format!(
        "https://m.series.naver.com/{kind}/moreNotiDetail.series"
    ))?;
    url.query_pairs_mut().append_pair("productNo", product_no);
    let notice = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await
        .ok()
        .and_then(|r| r.error_for_status().ok());
    if let Some(resp) = notice
        && let Ok(body) = official_meta::body_capped(resp, official_meta::MAX_DETAIL_BYTES).await
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
/// The title is the episode name ([`episode_title`]); the release date is the
/// `YYYY-MM-DD` prefix of `lastVolumeUpdateDate`. Numbers are assigned
/// globally in [`fetch_meta`] - rows here carry `number_x100 = 0`. At most
/// `NAVER_ROWS_PER_PAGE` rows are read. Malformed or truncated input yields an
/// empty vector - never a panic.
pub fn parse_volume_list(json: &str, kind: &str) -> Vec<ChapterMeta> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.get("resultData").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for item in items.iter().take(NAVER_ROWS_PER_PAGE) {
        let Some(title) = episode_title(item, kind) else {
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
/// `volumnNameText` (the real comic episode name: `1화`, `1부-1화`, `예고편`),
/// `subProductName` (the novel episode name: `Episode 1. …`), `volumeName`,
/// then the series name `productName`. The emptiness check runs per candidate —
/// an empty `volumnNameText` must not shadow the fallback.
/// Episode title for one volume row. Novels carry a generic `volumnNameText`
/// (`1화`) and the real episode name in `subProductName` (`Episode 1. …`), so
/// novels read `subProductName` first; comics keep the volume name first.
fn episode_title(item: &serde_json::Value, kind: &str) -> Option<String> {
    let order: &[&str] = if kind == "novel" {
        &[
            "subProductName",
            "volumnNameText",
            "volumeName",
            "productName",
        ]
    } else {
        &[
            "volumnNameText",
            "volumeName",
            "subProductName",
            "productName",
        ]
    };
    order.iter().find_map(|key| {
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
    fn parses_fixture_with_kind_prefixed_ids() {
        let hits = parse_search(FIXTURE);
        // One comic result plus the novel section, each with its kind encoded
        // in the id (`novel/{n}` vs bare digits).
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].id, "10319749");
        assert_eq!(
            hits.iter().filter(|h| h.id.starts_with("novel/")).count(),
            2
        );
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
    fn skips_thumbnail_wrappers_but_keeps_the_title_anchor() {
        // Live Naver markup: the cover link (`class="pic"`) carries a reward
        // badge (`광고보고무료`) or nothing at all; the real title anchor
        // follows. Both wrapper shapes must be skipped.
        let with_badge = "<div class=\"com_srch\"><li>\
            <a href=\"/comic/detail.series?productNo=6448122\" class=\"pic N=a:com.img\">\
                <em class=\"ico ico_reword_video\">광고보고무료</em>\
                <img src=\"x.jpg\" alt=\"역대급 영지 설계사 [독점]\">\
            </a>\
            <div class=\"cont\"><h3>\
                <a href=\"/comic/detail.series?productNo=6448122\">역대급 영지 설계사 [독점]</a>\
            </h3></div>\
        </li></div>";
        let hits = parse_search(with_badge);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "6448122");
        assert_eq!(hits[0].title, "역대급 영지 설계사");

        // A badge without any `<img>` (lazy cover) must be skipped too.
        let badge_no_img = "<div class=\"com_srch\">\
            <a href=\"/comic/detail.series?productNo=7\" class=\"pic\">광고보고무료</a>\
            <a href=\"/comic/detail.series?productNo=7\" class=\"t\">진짜 제목</a>\
        </div>";
        let hits = parse_search(badge_no_img);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "진짜 제목");
    }

    #[test]
    fn novel_ids_are_prefixed() {
        let html = "<div class=\"com_srch\">\
            <a href=\"/novel/detail.series?productNo=5108800\">소설 (총 742화/완결)</a>\
        </div>";
        let hits = parse_search(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "novel/5108800");
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
        let rows = parse_volume_list(html, "comic");
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
        assert!(parse_volume_list("<li><a>hi", "comic").is_empty());
    }

    #[test]
    fn empty_volume_name_falls_back() {
        let body = r#"{"resultData":[{"volumnNameText":"","volumeName":"1화",
            "productName":"제목","lastVolumeUpdateDate":"2024-01-02 00:00:00"}]}"#;
        let rows = parse_volume_list(body, "comic");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title.as_deref(), Some("1화"));
        // An all-empty name chain drops the row.
        let body = r#"{"resultData":[{"volumnNameText":" ","volumeName":"","productName":""}]}"#;
        assert!(parse_volume_list(body, "comic").is_empty());
    }

    #[test]
    fn volume_page_caps_rows() {
        let items: Vec<String> = (0..(NAVER_ROWS_PER_PAGE + 10))
            .map(|i| format!(r#"{{"volumnNameText":"{i}화","productName":"제목"}}"#))
            .collect();
        let body = format!(r#"{{"resultData":[{}]}}"#, items.join(","));
        assert_eq!(parse_volume_list(&body, "comic").len(), NAVER_ROWS_PER_PAGE);
    }

    #[test]
    fn novel_rows_prefer_the_sub_product_name() {
        // Novels carry a generic `volumnNameText` (`1화`) and the real episode
        // name in `subProductName`; comics keep `volumnNameText` first.
        let body = r#"{"resultData":[{"volumnNameText":"1화",
            "subProductName":"Episode 1. 유료 서비스 시작","productName":"제목"}]}"#;
        let novel = parse_volume_list(body, "novel");
        assert_eq!(
            novel[0].title.as_deref(),
            Some("Episode 1. 유료 서비스 시작")
        );
        let comic = parse_volume_list(body, "comic");
        assert_eq!(comic[0].title.as_deref(), Some("1화"));
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
