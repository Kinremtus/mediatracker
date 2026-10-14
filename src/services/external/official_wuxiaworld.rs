//! Wuxiaworld (lite.wuxiaworld.com) official-source novel search + chapter
//! metadata.
//!
//! The lite site is the plain server-rendered Wuxiaworld ("the lightweight
//! version … built for e-readers and older browsers"): `/novels?q=` renders
//! the search results as a novel grid, and `/novel/{slug}?toc=N` pages the
//! complete chapter list 100 rows at a time, with a `Page N of M` pager and an
//! `N chapters` header the walk must match. The full site carries the same
//! data but its app talks gRPC-web to api2.wuxiaworld.com; the lite pages
//! cover both needs on one host with no protobuf wire work.
//!
//! Chapter numbering is positional: the site mixes `Chapter N.`,
//! `Book N, Chapter M –` and unnumbered rows (`Prologue`, `Art Gallery`,
//! `Epilogue`) in one list, and its own chapter counter includes every row —
//! so every row is a main chapter numbered by position, exactly as the site
//! counts them (Coiling Dragon 807, A Will Eternal 1321).
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const LITE_BASE: &str = "https://lite.wuxiaworld.com";

/// Browser-like User-Agent; the site serves plain HTML to it.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Rows read from one TOC page (the site serves 100; a longer answer is a
/// broken one, not a reason to grow the walk).
const TOC_PAGE_ROWS: usize = 100;
/// Walk cap (100 pages x 100 rows = 10 000 chapters); a longer novel is an
/// error, never a truncated answer.
const MAX_PAGES: i64 = 100;
/// Pages fetched concurrently. The walk runs inline in the drawer request, so
/// a 7 000-chapter novel (74 pages) must not cost 74 sequential round-trips;
/// batches of 8 stay a polite load for the site.
const PAGE_BATCH: i64 = 8;
/// Defensive cap on one search answer.
const SEARCH_MAX_HITS: usize = 50;
/// Longest stored chapter title (a hostile answer must not bloat the DB).
const MAX_TITLE_CHARS: usize = 512;
/// How far a tag/attribute search may reach from its anchor.
const WINDOW: usize = 4096;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "wuxiaworld".to_string(),
        ..OfficialMeta::default()
    }
}

/// GET `/novels?q=<urlencoded>` and parse the novel grid.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(&format!("{LITE_BASE}/novels"))?;
    url.query_pairs_mut().append_pair("q", title);
    let body = get_html(client, url).await?;
    Ok(parse_search(&body))
}

/// Pure parser over the search page: every `<td class="novel-cell">` carries
/// its novel as `<p class="title"><a href="/novel/{slug}">{title}</a></p>`.
/// Malformed HTML yields an empty vector — never a panic.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("class=\"novel-cell\"") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(end_rel) = html[at..hi].find("</td>") else {
            break;
        };
        let cell = &html[at..at + end_rel];
        cursor = at + end_rel + "</td>".len();

        let anchor = "<p class=\"title\"><a href=\"/novel/";
        let Some(after) = cell.find(anchor) else {
            continue;
        };
        let rest = &cell[after + anchor.len()..];
        let Some(quote) = rest.find('"') else {
            continue;
        };
        let slug = &rest[..quote];
        if !official_meta::is_safe_path_id(slug) || hits.iter().any(|h| h.id == slug) {
            continue;
        }
        let tail = &rest[quote..];
        let Some(gt) = tail.find('>') else {
            continue;
        };
        let text = &tail[gt + 1..];
        let Some(end) = text.find("</a>") else {
            continue;
        };
        let title = official_meta::strip_tags(&text[..end]);
        if title.is_empty() {
            continue;
        }
        hits.push(OfficialHit {
            id: slug.to_string(),
            title: title.chars().take(MAX_TITLE_CHARS).collect(),
        });
        if hits.len() >= SEARCH_MAX_HITS {
            break;
        }
    }
    hits
}

/// One TOC page: the chapter titles in reading order plus the walk bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocPage {
    pub rows: Vec<String>,
    pub page: i64,
    pub total_pages: i64,
    /// The novel header's own chapter counter; the walk must match it.
    pub declared_count: i64,
}

/// Pure parser over one `/novel/{slug}?toc=N` page. `None` when the answer is
/// malformed or misses the walk bounds (pager or declared count) — a walk
/// without bounds must not run. Only `<li><a href="/novel/{slug}/…">` rows are
/// read, so the `Start reading` button (same href shape) is not a chapter.
pub fn parse_toc_page(html: &str, slug: &str) -> Option<TocPage> {
    let (page, total_pages) = parse_pager(html)?;
    let declared_count = parse_declared_count(html)?;
    let marker = format!("<li><a href=\"/novel/{slug}/");
    let mut rows: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find(&marker) {
        let at = cursor + relative + marker.len();
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(gt_rel) = html[at..hi].find('>') else {
            break;
        };
        let title_start = at + gt_rel + 1;
        let Some(end_rel) = html[title_start..hi].find("</a>") else {
            break;
        };
        let raw = &html[title_start..title_start + end_rel];
        cursor = title_start + end_rel + "</a>".len();
        let title = official_meta::strip_tags(raw);
        if title.is_empty() {
            continue;
        }
        rows.push(title.chars().take(MAX_TITLE_CHARS).collect());
        if rows.len() >= TOC_PAGE_ROWS {
            break;
        }
    }
    Some(TocPage {
        rows,
        page,
        total_pages,
        declared_count,
    })
}

/// `Page N of M` inside the pager block.
fn parse_pager(html: &str) -> Option<(i64, i64)> {
    let pager_at = html.find("class=\"pager cf\"")?;
    let window = &html[pager_at..official_meta::floor_boundary(html, pager_at + WINDOW)];
    let at = window.find("Page ")? + "Page ".len();
    let rest = &window[at..];
    let end = rest.find('<')?;
    let (page, total) = rest[..end].split_once(" of ")?;
    Some((page.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// `N chapters` from the novel header line (`… · 807 chapters</p>`). Anchored
/// to the header's own class so a synopsis mentioning chapters cannot match.
fn parse_declared_count(html: &str) -> Option<i64> {
    let head_at = html.find("class=\"muted small\">")?;
    let window = &html[head_at..official_meta::floor_boundary(html, head_at + WINDOW)];
    for marker in [" chapters", " chapter"] {
        if let Some(at) = window.find(marker) {
            let before = &window[..at];
            let digits: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !digits.is_empty() {
                return digits.parse().ok();
            }
        }
    }
    None
}

/// Number the chapter rows by position: every row is a main chapter, exactly
/// as the site's own counter treats them (specials included).
pub fn chapters_from_rows(rows: Vec<String>) -> Vec<ChapterMeta> {
    rows.into_iter()
        .enumerate()
        .map(|(index, title)| ChapterMeta {
            number_x100: ((index as i32) + 1) * 100,
            title: Some(title),
            release_date: None,
            is_bonus: false,
            label: None,
        })
        .collect()
}

/// Walk the full chapter list and map it onto the shared chapter model. The
/// walk is accepted only when it returned exactly the header's chapter count —
/// a partial answer must never reach `apply_meta`, whose orphan cleanup would
/// drop rows the answer never saw. Pages are fetched in batches of
/// [`PAGE_BATCH`]; a failed page fails the whole walk.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    // The slug lands in the URL path: plain alphanumeric tokens only.
    if !official_meta::is_safe_path_id(source_id) {
        return Ok(empty_meta());
    }
    let first = fetch_toc_page(client, source_id, 1).await?;
    if first.declared_count <= 0 {
        return Ok(empty_meta());
    }
    if first.total_pages > MAX_PAGES {
        anyhow::bail!("wuxiaworld: novel too long ({} pages)", first.total_pages);
    }
    let mut rows = first.rows;
    let mut page = 2i64;
    while page <= first.total_pages {
        let last = (page + PAGE_BATCH - 1).min(first.total_pages);
        let mut set = tokio::task::JoinSet::new();
        for batch_page in page..=last {
            let client = client.clone();
            let slug = source_id.to_string();
            set.spawn(
                async move { (batch_page, fetch_toc_page(&client, &slug, batch_page).await) },
            );
        }
        let mut batch: Vec<(i64, TocPage)> = Vec::new();
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((batch_page, Ok(parsed))) => batch.push((batch_page, parsed)),
                Ok((batch_page, Err(e))) => {
                    anyhow::bail!("wuxiaworld: page {batch_page} failed: {e}");
                }
                Err(e) => anyhow::bail!("wuxiaworld: page task failed: {e}"),
            }
        }
        // The join order is arbitrary; the rows must stay in reading order.
        batch.sort_by_key(|(batch_page, _)| *batch_page);
        for (_, parsed) in batch {
            rows.extend(parsed.rows);
        }
        page = last + 1;
    }
    if rows.len() as i64 != first.declared_count {
        anyhow::bail!(
            "wuxiaworld: chapter walk incomplete ({}/{} rows)",
            rows.len(),
            first.declared_count
        );
    }
    let chapters = chapters_from_rows(rows);
    let count = chapters.len() as i32;
    Ok(OfficialMeta {
        source: "wuxiaworld".to_string(),
        count: (count > 0).then_some(count),
        chapters,
        ..OfficialMeta::default()
    })
}

/// One TOC page of one novel.
async fn fetch_toc_page(
    client: &reqwest::Client,
    slug: &str,
    page: i64,
) -> anyhow::Result<TocPage> {
    let mut url = url::Url::parse(&format!("{LITE_BASE}/novel/{slug}"))?;
    url.query_pairs_mut().append_pair("toc", &page.to_string());
    let body = get_html(client, url).await?;
    parse_toc_page(&body, slug)
        .ok_or_else(|| anyhow::anyhow!("wuxiaworld: unparseable TOC page {page}"))
}

/// GET an HTML page and read the body with a hard size cap.
async fn get_html(client: &reqwest::Client, url: url::Url) -> anyhow::Result<String> {
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

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/wuxiaworld_search.html");
    const TOC_FIXTURE: &str = include_str!("../../../tests/fixtures/wuxiaworld_toc.html");
    const SPECIALS_FIXTURE: &str =
        include_str!("../../../tests/fixtures/wuxiaworld_toc_specials.html");

    #[test]
    fn search_fixture_yields_slug_hits() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert_eq!(hits.len(), 6);
        assert_eq!(hits[0].id, "dragon-canon");
        assert_eq!(hits[0].title, "Dragon Canon");
        assert!(
            hits.iter()
                .any(|h| h.id == "coiling-dragon" && h.title == "Coiling Dragon")
        );
    }

    #[test]
    fn search_drops_malformed_cells() {
        let html = "<td class=\"novel-cell\">\
            <p class=\"title\"><a href=\"/novel/good-one\">Good One</a></p></td>\
            <td class=\"novel-cell\"><p class=\"title\"><a href=\"/novel/../bad\">Bad</a></p></td>\
            <td class=\"novel-cell\"><p class=\"title\"><a href=\"/novel/\">Empty</a></p></td>\
            <td class=\"novel-cell\"><p class=\"title\"><a href=\"/novel/dup\">Dup</a></p></td>\
            <td class=\"novel-cell\"><p class=\"title\"><a href=\"/novel/dup\">Dup again</a></p></td>";
        let hits = parse_search(html);
        assert_eq!(hits.len(), 2, "bad slugs and duplicates are dropped");
        assert_eq!(hits[0].id, "good-one");
        assert_eq!(hits[1].id, "dup");
        // Malformed and unrelated answers are empty, never a panic.
        assert!(parse_search("").is_empty());
        assert!(parse_search("<table></table>").is_empty());
        assert!(parse_search("<td class=\"novel-cell\">").is_empty());
    }

    #[test]
    fn toc_fixture_parses_rows_and_bounds() {
        let page = parse_toc_page(TOC_FIXTURE, "coiling-dragon").expect("fixture parses");
        assert_eq!(page.page, 1);
        assert_eq!(page.total_pages, 9);
        assert_eq!(page.declared_count, 807);
        assert_eq!(page.rows.len(), 12, "the trimmed fixture holds 12 rows");
        assert_eq!(
            page.rows[0],
            "Book 1, Chapter 1 – Early Morning at a Township"
        );
        assert_eq!(
            page.rows[11],
            "Book 1, Chapter 12 – The Will of the Mighty (part 1)"
        );
        // The `Start reading` button is not a chapter.
        assert!(page.rows.iter().all(|r| r != "Start reading"));
        // Malformed pages are None, never a panic.
        assert_eq!(parse_toc_page("", "coiling-dragon"), None);
        assert_eq!(parse_toc_page("<html></html>", "coiling-dragon"), None);
    }

    #[test]
    fn toc_specials_are_ordinary_rows() {
        // The last page of A Will Eternal ends with `Epilogue` and the
        // `Blood Ancestor` side-story parts; the site counts them as chapters.
        let page = parse_toc_page(SPECIALS_FIXTURE, "a-will-eternal").expect("fixture parses");
        assert_eq!(page.page, 14);
        assert_eq!(page.total_pages, 14);
        assert_eq!(page.declared_count, 1321);
        assert_eq!(page.rows.len(), 21);
        assert!(page.rows.iter().any(|r| r == "Epilogue"));
        assert!(
            page.rows
                .iter()
                .any(|r| r == "A Will Eternal: Blood Ancestor [Part 4]")
        );
    }

    #[test]
    fn chapters_number_by_position_including_specials() {
        let page = parse_toc_page(SPECIALS_FIXTURE, "a-will-eternal").expect("fixture parses");
        let chapters = chapters_from_rows(page.rows);
        let numbers: Vec<i32> = chapters.iter().map(|c| c.number_x100).collect();
        assert_eq!(numbers, (1..=21).map(|n| n * 100).collect::<Vec<_>>());
        assert!(chapters.iter().all(|c| !c.is_bonus));
        // The count matches the page's row count; `fetch_meta` sums the pages.
        assert_eq!(chapters.len(), 21);
    }

    #[test]
    fn declared_count_is_anchored_to_the_header() {
        // A synopsis mentioning "5 chapters" must not shadow the real header.
        let html = "<p class=\"syn\">Read the first 5 chapters now.</p>\
            <p class=\"muted small\">Completed · Author: X · 42 chapters</p>";
        assert_eq!(parse_declared_count(html), Some(42));
        assert_eq!(parse_declared_count("<html></html>"), None);
        assert_eq!(
            parse_declared_count("<p class=\"muted small\">X · 7 chapters</p>"),
            Some(7)
        );
    }

    #[tokio::test]
    async fn unsafe_slug_is_empty() {
        let client = crate::services::external::http_client();
        let meta = fetch_meta(&client, "../etc/passwd")
            .await
            .expect("a bad slug must not hit the network");
        assert!(meta.chapters.is_empty());
        assert!(meta.count.is_none());
    }
}
