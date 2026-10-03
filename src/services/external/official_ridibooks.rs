//! Ridibooks KR detail (static SSR HTML) + guest search.
//!
//! **Detail degrades to a documented no-op on the fixture**: `ridibooks_detail.html`
//! has zero matches for `episode` / `class="title"` / `ridi_cat` (the page's
//! "이 책의 시리즈" module lists *volumes*, not chapters — numbering them would
//! fabricate chapter rows) and zero schedule markers (`매주`/`요일`/`정기`), so
//! `chapters`, `update_schedule` and `next_update_at` all stay empty/`None`.
//! The `parse_episodes` scan below stays for pages that do carry the row list.
//!
//! Search walks the SSR `__NEXT_DATA__` JSON (never the markup): `cells[*]
//! .cell__SearchBookListWithTab.books[*]` -> `book.id` + `book.title.main`.

use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// GET `https://ridibooks.com/books/{book_id}` (up to ~8 MB; text() is
/// acceptable for a detail page).
pub async fn fetch_meta(client: &reqwest::Client, book_id: &str) -> anyhow::Result<OfficialMeta> {
    let url = format!("https://ridibooks.com/books/{book_id}");
    let html = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_detail(&html))
}

/// Pure parser. No schedule field exists on Ridibooks; only chapter rows
/// when the page carries the `episode` row list (see module docs).
pub fn parse_detail(html: &str) -> OfficialMeta {
    OfficialMeta {
        source: "ridibooks".to_string(),
        count: None,
        chapters: parse_episodes(html),
        update_schedule: None,
        next_update_at: None,
    }
}

/// Scan `episode` row items (`class="title"` + optional `class="date"`).
/// Total: no rows / truncated HTML -> `Vec::new()`, never a panic.
pub fn parse_episodes(html: &str) -> Vec<ChapterMeta> {
    let mut rows: Vec<ChapterMeta> = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("episode") {
        let seg = &rest[pos..];
        let Some(end) = seg.find("</li>") else { break };
        let row = &seg[..end];
        rest = &seg[end + 5..];
        // `between` stops at the first `</`, so skip past the opening `>`
        // of the element before stripping tags.
        let title = official_meta::between(row, "class=\"title\"", "</")
            .and_then(|s| s.find('>').map(|i| &s[i + 1..]))
            .map(official_meta::strip_tags)
            .filter(|s| !s.is_empty());
        let date = official_meta::between(row, "class=\"date\"", "</")
            .and_then(|s| s.find('>').map(|i| &s[i + 1..]))
            .map(official_meta::strip_tags)
            .filter(|s| !s.is_empty())
            .and_then(|s| official_meta::parse_date(&s));
        if title.is_some() || date.is_some() {
            rows.push(ChapterMeta {
                number_x100: ((rows.len() + 1) * 100) as i32,
                title,
                release_date: date,
            });
        }
    }
    rows
}

/// GET `https://ridibooks.com/search?q={kw}` (anonymous; CF cookies ok).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse("https://ridibooks.com/search")?;
    url.query_pairs_mut().append_pair("q", title);
    let html = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_search(&html))
}

/// Walk `cells[*].cell__SearchBookListWithTab.books[*]` in the embedded
/// `__NEXT_DATA__` JSON. Ids arrive as JSON numbers *or* strings, so both are
/// normalised to `String`. Cells without the block / books missing id or
/// title are skipped; malformed JSON -> empty vector.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let seg = official_meta::between(html, "__NEXT_DATA__", "</script>").unwrap_or("");
    let json = match seg.find('{') {
        Some(i) => &seg[i..],
        None => return Vec::new(),
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(cells) = value
        .pointer("/props/pageProps/gridData/riGrid/grid/cells")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for cell in cells {
        let Some(books) = cell
            .get("cell__SearchBookListWithTab")
            .and_then(|b| b.get("books"))
            .and_then(|b| b.as_array())
        else {
            continue;
        };
        for entry in books {
            let book = entry.get("book").unwrap_or(entry);
            let Some(id) = id_string(book.get("id")).or_else(|| id_string(entry.get("id")))
            else {
                continue;
            };
            let Some(title) = book
                .pointer("/title/main")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !hits.iter().any(|h| h.id == id) {
                hits.push(OfficialHit {
                    id,
                    title: title.to_string(),
                });
            }
        }
    }
    hits
}

/// JSON id (number or string) -> `String`; anything else -> `None`.
fn id_string(value: Option<&serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_fixture_degrades_to_noop() {
        // Documented degradation: no episode rows and no schedule on this page.
        let html = include_str!("../../../tests/fixtures/ridibooks_detail.html");
        let m = parse_detail(html);
        assert_eq!(m.source, "ridibooks");
        assert!(m.chapters.is_empty(), "fixture carries no chapter rows");
        assert!(m.update_schedule.is_none(), "ridibooks has no schedule field");
        assert!(m.next_update_at.is_none());
        assert!(m.count.is_none());
    }

    #[test]
    fn empty_input() {
        assert!(parse_episodes("").is_empty());
        let m = parse_detail("<html></html>");
        assert!(m.chapters.is_empty());
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn episode_rows_when_markup_present() {
        let html = r#"<ul>
            <li class="episode"><span class="title">제1화</span><span class="date">2026-01-05</span></li>
            <li class="episode"><span class="title">제2화</span></li>
        </ul>"#;
        let rows = parse_episodes(html);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].number_x100, 100);
        assert_eq!(rows[0].title.as_deref(), Some("제1화"));
        assert_eq!(
            rows[0].release_date.map(|d| d.to_string()).as_deref(),
            Some("2026-01-05")
        );
        assert_eq!(rows[1].number_x100, 200);
        assert!(rows[1].release_date.is_none());
    }

    #[test]
    fn parses_search_fixture() {
        let html = include_str!("../../../tests/fixtures/ridibooks_search.html");
        let hits = parse_search(html);
        // 24 books of 142 matches are on the first page of the fixture.
        assert_eq!(hits.len(), 24);
        assert!(hits.iter().all(|h| !h.id.is_empty() && !h.title.is_empty()));
        assert_eq!(hits[0].id, "777035894");
        assert_eq!(hits[0].title, "나 혼자만 레벨업 1권");
    }
}
