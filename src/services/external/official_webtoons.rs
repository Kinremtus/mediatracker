//! WEBTOON (webtoons.com) official-source title search + chapter metadata.
//!
//! The site search page is server-rendered: every matching series is a card
//! (`data-title-no`, `data-webtoon-type`). The episode list comes from the
//! public mobile JSON API (`m.webtoons.com/api/v1/webtoon/{title_no}/episodes`),
//! which paginates by cursor (`nextCursor` = the last `episodeNo` seen) and
//! needs no login. Only `WEBTOON` cards are kept — the `CHALLENGE` (CANVAS)
//! entries are user uploads, not licensed content. A walk that cannot finish
//! (budget, page cap, a cursor that does not advance) is an error: a partial
//! answer must never reach `apply_meta`.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use std::collections::HashSet;
use std::time::Duration;

use crate::services::external::chapter_title::{self, EpisodeRow, ShortTitle};
use crate::services::external::official_meta::{self, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

const SEARCH_BASE: &str = "https://www.webtoons.com/en/search";
const API_BASE: &str = "https://m.webtoons.com/api/v1/webtoon";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
/// Episodes requested per API page.
const PAGE_SIZE: usize = 30;
/// Defensive cap on pages walked (3000 episodes).
const MAX_PAGES: usize = 100;
/// Total wall-clock budget for one walk: the fetch runs inline in the drawer
/// request, so a slow or hostile upstream must not hold the worker for minutes.
const WALK_BUDGET: Duration = Duration::from_secs(20);
/// Upper bound on a plausible episode number; anything larger is a broken or
/// hostile answer and must not inflate the chapter counter.
const MAX_EPISODE_NO: i64 = 100_000;
/// How far a tag/attribute search may reach from its anchor.
const WINDOW: usize = 4096;
/// Longest stored episode title (a hostile answer must not bloat the DB).
const MAX_TITLE_CHARS: usize = 512;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "webtoons".to_string(),
        ..OfficialMeta::default()
    }
}

/// GET https://www.webtoons.com/en/search?keyword=<urlencoded>
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.query_pairs_mut().append_pair("keyword", title);
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?;
    let body = official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await?;
    Ok(parse_search(&body))
}

/// Pure parser over the search page: keeps licensed `WEBTOON` cards and drops
/// the user-upload ones (`CHALLENGE`, the site's CANVAS section). Each card is
/// `<a class="link _card_item" data-title-no="2154" data-webtoon-type="WEBTOON">
/// … <strong class="title">Name</strong>`. Malformed HTML yields an empty
/// vector - never a panic.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut cursor = 0usize;
    while let Some(idx) = html[cursor..].find("_card_item") {
        let at = cursor + idx;
        cursor = at + "_card_item".len();
        // The tag starts before the class token; take the whole anchor tag. The
        // window keeps a hostile page from stretching the tag across the
        // document (every hit would otherwise clone it).
        let lo = official_meta::floor_boundary(html, at.saturating_sub(WINDOW));
        let Some(rel_start) = html[lo..at].rfind('<') else {
            continue;
        };
        let tag_start = lo + rel_start;
        // A `>` in between means the token sits outside a tag (hostile or
        // broken HTML): the attributes would come from neighbouring tags.
        if html[tag_start..at].contains('>') {
            continue;
        }
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(rel_end) = html[at..hi].find('>') else {
            continue;
        };
        let tag = &html[tag_start..at + rel_end];
        if official_meta::attr(tag, "data-webtoon-type").as_deref() != Some("WEBTOON") {
            continue;
        }
        let Some(id) = official_meta::attr(tag, "data-title-no") else {
            continue;
        };
        if !official_meta::is_digit_id(&id) || seen.contains(&id) {
            continue;
        }
        // The title belongs to this card: bound the lookup by the anchor end.
        let card_hi = official_meta::floor_boundary(html, cursor + WINDOW);
        let Some(anchor_end) = html[cursor..card_hi].find("</a>") else {
            continue;
        };
        let card = &html[cursor..cursor + anchor_end];
        let Some(raw) = official_meta::between(card, "<strong class=\"title\">", "</strong>")
        else {
            continue;
        };
        let title = official_meta::strip_tags(raw);
        let title: String = title.chars().take(MAX_TITLE_CHARS).collect();
        if title.is_empty() {
            continue;
        }
        // Rich search-answer fields: the card's poster and author line.
        let poster_url = official_meta::between(card, "<img src=\"", "\"").map(str::to_string);
        let author = official_meta::between(card, "<div class=\"author\">", "</div>")
            .map(official_meta::strip_tags)
            .filter(|a| !a.is_empty());
        seen.insert(id.clone());
        hits.push(OfficialHit::with_extra(
            id,
            title,
            HitExtra {
                poster_url,
                author,
                ..HitExtra::default()
            },
        ));
    }
    hits
}

/// Walk the episode API by cursor and map the rows onto the shared chapter
/// model. The API reports the source's own sequential `episodeNo` (the display
/// titles run one lower), so those numbers are kept verbatim; rows whose title
/// marks side content (`Preview`, `Special`, …) become bonuses.
pub async fn fetch_meta(client: &reqwest::Client, title_no: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(title_no) {
        return Ok(empty_meta());
    }
    let mut rows: Vec<EpisodeRow> = Vec::new();
    let mut cursor: Option<i64> = None;
    let started = std::time::Instant::now();
    let endpoint = format!("{API_BASE}/{title_no}/episodes");
    let mut complete = false;
    for _ in 0..MAX_PAGES {
        if started.elapsed() > WALK_BUDGET {
            break;
        }
        let mut url = url::Url::parse(&endpoint)?;
        url.query_pairs_mut()
            .append_pair("pageSize", &PAGE_SIZE.to_string());
        if let Some(c) = cursor {
            url.query_pairs_mut().append_pair("cursor", &c.to_string());
        }
        let response = client
            .get(url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?;
        let body = official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await?;
        let (page_rows, next) = parse_episodes(&body);
        if page_rows.is_empty() {
            complete = next.is_none();
            break;
        }
        rows.extend(page_rows);
        match next {
            // The cursor must advance; anything else is a broken answer.
            Some(c) if cursor.is_none_or(|cur| c > cur) => cursor = Some(c),
            Some(_) => anyhow::bail!("webtoons: episode cursor did not advance"),
            None => {
                complete = true;
                break;
            }
        }
    }
    if !complete {
        // A partial answer must not reach `apply_meta`: its orphan cleanup
        // would drop rows the walk simply never saw.
        anyhow::bail!("webtoons: episode list incomplete (budget or page cap)");
    }
    let chapters = chapter_title::rows_to_chapters(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "webtoons".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    })
}

/// Pure parser over one episode page: the rows plus the cursor for the next
/// page. The cursor is the last `episodeNo` the page showed — not the server's
/// own `nextCursor`, which points past rows a shortened page never handed out —
/// and it is only returned while the answer reports more pages. Malformed or
/// truncated JSON yields `(vec![], None)` - never a panic.
pub fn parse_episodes(json: &str) -> (Vec<EpisodeRow>, Option<i64>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return (Vec::new(), None);
    };
    let Some(result) = value.pointer("/result") else {
        return (Vec::new(), None);
    };
    let Some(items) = result.get("episodeList").and_then(|v| v.as_array()) else {
        return (Vec::new(), None);
    };
    let mut rows = Vec::new();
    let mut last_shown: Option<i64> = None;
    // The API honours `pageSize`; a longer answer is a broken one, not a reason
    // to grow the walk.
    for item in items.iter().take(PAGE_SIZE) {
        let Some(no) = item.get("episodeNo").and_then(|v| v.as_i64()) else {
            continue;
        };
        last_shown = Some(no);
        if !(1..=MAX_EPISODE_NO).contains(&no) {
            continue;
        }
        let title = item
            .get("episodeTitle")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| t.chars().take(MAX_TITLE_CHARS).collect::<String>());
        let release_date = item
            .get("exposureDateMillis")
            .and_then(|v| v.as_i64())
            .and_then(date_from_millis);
        let kind = match i32::try_from(no) {
            Ok(n) if !title.as_deref().is_some_and(is_bonus_title) => ShortTitle::Base(n),
            _ => ShortTitle::Unnumbered,
        };
        rows.push(EpisodeRow {
            title,
            release_date,
            kind,
        });
    }
    let more = result.get("nextCursor").and_then(|v| v.as_i64()).is_some();
    let next = if more { last_shown } else { None };
    (rows, next)
}

/// Epoch milliseconds -> the UTC day.
fn date_from_millis(ms: i64) -> Option<chrono::NaiveDate> {
    chrono::DateTime::from_timestamp_millis(ms).map(|dt| dt.date_naive())
}

/// Side-content markers on episode titles, matched on a word boundary at the
/// start: `Preview 2` is a bonus, `Previewing the End` is a chapter. `Prologue`
/// is not a marker, so `Episode 0 - Prologue` stays the first episode.
fn is_bonus_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    ["preview", "special", "notice", "q&a", "trailer"]
        .iter()
        .any(|marker| chapter_title::starts_with_word(&lower, marker))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/webtoons_search.html");
    const API_FIXTURE: &str = include_str!("../../../tests/fixtures/webtoons_api_p1.json");
    const API_FIXTURE_P2: &str = include_str!("../../../tests/fixtures/webtoons_api_p2.json");

    #[test]
    fn search_fixture_keeps_originals_only() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert_eq!(
            hits.len(),
            10,
            "the fixture holds 10 WEBTOON cards and 12 user-upload ones"
        );
        assert!(
            hits.iter()
                .any(|h| h.title == "Omniscient Reader" && h.id == "2154"),
            "the licensed card is kept: {hits:?}"
        );
        assert!(hits.iter().all(|h| official_meta::is_digit_id(&h.id)));
        assert!(
            !hits.iter().any(|h| h.id == "1175282"),
            "user-upload entries are not official sources"
        );
        // Rich search-answer fields: the card's cover and author line.
        let orv = hits.iter().find(|h| h.id == "2154").expect("ORV card");
        assert_eq!(orv.extra.author.as_deref(), Some("singNsong / UMI"));
        assert!(
            orv.extra
                .poster_url
                .as_deref()
                .is_some_and(|u| u.starts_with("https://webtoon-phinf.pstatic.net/")),
            "the card cover is kept: {:?}",
            orv.extra.poster_url
        );
    }

    #[test]
    fn search_malformed_is_empty() {
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html>no cards</html>").is_empty());
    }

    #[test]
    fn episodes_fixture_keeps_source_numbers_and_dates() {
        let (rows, next) = parse_episodes(API_FIXTURE);
        assert_eq!(rows.len(), 30);
        assert_eq!(next, Some(30), "the cursor is the last episodeNo seen");
        // The source's own numbering is kept verbatim.
        match rows[0].kind {
            ShortTitle::Base(1) => {}
            other => panic!("first row must be chapter 1, got {other:?}"),
        }
        assert_eq!(rows[0].title.as_deref(), Some("Episode 0 - Prologue"));
        assert_eq!(
            rows[0].release_date,
            official_meta::parse_date("2020-08-19"),
            "exposureDateMillis is epoch ms"
        );
        assert!(rows.iter().all(|r| matches!(r.kind, ShortTitle::Base(_))));
    }

    #[test]
    fn episodes_malformed_is_empty() {
        assert_eq!(parse_episodes("").0.len(), 0);
        assert_eq!(parse_episodes("{}").0.len(), 0);
        assert_eq!(parse_episodes(r#"{"result":{}}"#).1, None);
    }

    #[test]
    fn bonus_markers_are_word_bounded() {
        assert!(is_bonus_title("Special 3"));
        assert!(is_bonus_title("Preview"));
        assert!(!is_bonus_title("Previewing the End"));
        assert!(!is_bonus_title("Episode 0 - Prologue"));
    }

    #[test]
    fn episodes_paginate_by_cursor_and_clamp() {
        // Page two (cursor=30) continues where page one ended.
        let (p1, next1) = parse_episodes(API_FIXTURE);
        let (p2, next2) = parse_episodes(API_FIXTURE_P2);
        assert_eq!((next1, next2), (Some(30), Some(60)));
        assert_eq!(p1.len(), 30);
        assert_eq!(p2.len(), 30);
        match (p1.last().map(|r| r.kind), p2.first().map(|r| r.kind)) {
            (Some(ShortTitle::Base(a)), Some(ShortTitle::Base(b))) => assert_eq!((a, b), (30, 31)),
            other => panic!("expected an ascending run, got {other:?}"),
        }

        // Broken numbers are dropped, oversized titles clamped.
        let long = "y".repeat(600);
        let body = format!(
            r#"{{"result":{{"episodeList":[
                {{"episodeNo": 0, "episodeTitle": "Episode 0"}},
                {{"episodeNo": 100001, "episodeTitle": "Episode X"}},
                {{"episodeNo": 5, "episodeTitle": "{long}"}}
            ]}}}}"#
        );
        let (rows, _) = parse_episodes(&body);
        assert_eq!(rows.len(), 1, "out-of-range numbers are dropped");
        assert_eq!(rows[0].title.as_deref().map(str::len), Some(512));
    }
}
