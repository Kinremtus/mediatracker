//! AC.QQ (ac.qq.com, Tencent) official-source title search + chapter metadata.
//!
//! Both pages are plain server-rendered HTML and need no login. The search page
//! lists hits as `/Comic/comicInfo/id/{id}` anchors carrying the series name in
//! the `title` attribute; a series page carries the full chapter list as
//! `works-chapter-item` blocks whose anchor text is the chapter label
//! (`01`, `96（上）`, `第190回 魂殿的秘密`, `番外02 …`). Only the resolved numeric
//! id is bound; the chapter list needs no pagination walk.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use std::collections::HashSet;

use crate::services::external::chapter_title::{self, EpisodeRow};
use crate::services::external::official_meta::{self, OfficialMeta};
use crate::services::official_types::OfficialHit;

const SEARCH_BASE: &str = "https://ac.qq.com/Comic/searchList/search";
const COMIC_BASE: &str = "https://ac.qq.com/Comic/comicInfo/id";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
/// Defensive cap on rows taken from one answer.
const MAX_ROWS: usize = 5000;
/// Defensive cap on search hits (a hostile page must not multiply them).
const MAX_HITS: usize = 200;
/// How far a tag/attribute search may reach from its anchor: tag windows must
/// not span the whole document, or every hit would clone it.
const WINDOW: usize = 4096;
/// Longest stored chapter title (a hostile answer must not bloat the DB).
const MAX_TITLE_CHARS: usize = 512;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "acqq".to_string(),
        ..OfficialMeta::default()
    }
}

/// GET https://ac.qq.com/Comic/searchList/search/<urlencoded>
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("acqq: search base cannot be a base"))?
        .push(title);
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?;
    let body = official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await?;
    Ok(parse_search(&body))
}

/// Pure parser over the search page: keeps `/Comic/comicInfo/id/{id}` anchors
/// with a non-empty `title` attribute, dedups by id. Both the tag window and
/// the number/length of hits are capped - a hostile page must not turn one
/// answer into gigabytes. Malformed HTML yields an empty vector, never a panic.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    const MARKER: &str = "/Comic/comicInfo/id/";
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut cursor = 0usize;
    while let Some(idx) = html[cursor..].find(MARKER) {
        if hits.len() >= MAX_HITS {
            break;
        }
        let at = cursor + idx + MARKER.len();
        cursor = at;
        let id: String = html[at..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if !official_meta::is_digit_id(&id) || seen.contains(&id) {
            continue;
        }
        // The enclosing anchor tag carries the series name; the window keeps a
        // hostile page from stretching the tag across the document.
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
        let Some(title) = official_meta::attr(tag, "title") else {
            continue;
        };
        let title: String = official_meta::strip_tags(&title)
            .chars()
            .take(MAX_TITLE_CHARS)
            .collect();
        if title.is_empty() {
            continue;
        }
        seen.insert(id.clone());
        hits.push(OfficialHit::new(id, title));
    }
    hits
}

/// Fetch the full chapter list. Every row is a `works-chapter-item` block whose
/// anchor text is the chapter label; numbering and bonus classification come
/// from [`chapter_title::classify_acqq_title`].
pub async fn fetch_meta(client: &reqwest::Client, comic_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(comic_id) {
        return Ok(empty_meta());
    }
    let url = url::Url::parse(&format!("{COMIC_BASE}/{comic_id}"))?;
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?;
    let body = official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await?;
    Ok(parse_chapters(&body))
}

/// Pure parser over the series page: the label of every chapter row, in the
/// page's own order (oldest first). Malformed HTML yields an empty meta - never
/// a panic.
pub fn parse_chapters(html: &str) -> OfficialMeta {
    const BLOCK: &str = "works-chapter-item";
    let mut rows: Vec<EpisodeRow> = Vec::new();
    let mut cursor = 0usize;
    while let Some(idx) = html[cursor..].find(BLOCK) {
        if rows.len() >= MAX_ROWS {
            break;
        }
        let at = cursor + idx + BLOCK.len();
        cursor = at;
        // Bound the row to its own block: an anchor-less row must not borrow
        // the anchor of the next one.
        let block_end = html[at..].find(BLOCK).map_or(html.len(), |rel| at + rel);
        let hi = official_meta::floor_boundary(html, (at + WINDOW).min(block_end));
        let Some(a_rel) = html[at..hi].find("<a ") else {
            continue;
        };
        let a_start = at + a_rel;
        let a_hi = official_meta::floor_boundary(html, (a_start + WINDOW).min(block_end));
        let Some(a_end) = html[a_start..a_hi].find('>') else {
            continue;
        };
        let text_start = a_start + a_end + 1;
        let t_hi = official_meta::floor_boundary(html, text_start + WINDOW);
        let Some(t_rel) = html[text_start..t_hi].find("</a>") else {
            continue;
        };
        // The label window is bounded, so one hostile row cannot cost minutes.
        let label: String = official_meta::strip_tags(&html[text_start..text_start + t_rel])
            .chars()
            .take(MAX_TITLE_CHARS)
            .collect();
        if label.is_empty() {
            continue;
        }
        rows.push(EpisodeRow {
            title: Some(label.clone()),
            release_date: None,
            kind: chapter_title::classify_acqq_title(&label),
        });
    }
    let chapters = chapter_title::rows_to_chapters(rows);
    let main = chapter_title::main_count(&chapters);
    OfficialMeta {
        source: "acqq".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/acqq_search.html");
    const TOC_FIXTURE: &str = include_str!("../../../tests/fixtures/acqq_toc.html");

    #[test]
    fn search_fixture_yields_hits() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert_eq!(hits.len(), 27, "the capture holds 27 unique ids");
        assert!(
            hits.iter()
                .any(|h| h.id == "531040" && h.title == "斗破苍穹"),
            "the first hit: {hits:?}"
        );
        assert!(hits.iter().all(|h| official_meta::is_digit_id(&h.id)));
        // The series name comes from the `title` attribute, not the clipped
        // anchor text.
        assert!(
            hits.iter()
                .any(|h| h.id == "544429" && h.title == "斗破苍穹·药老传奇"),
            "attribute title for 544429: {hits:?}"
        );
    }

    #[test]
    fn search_malformed_is_empty() {
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html>no hits</html>").is_empty());
    }

    #[test]
    fn chapters_fixture_classifies_labels() {
        let meta = parse_chapters(TOC_FIXTURE);
        assert_eq!(meta.source, "acqq");
        // The trimmed fixture: 42 numbered rows (01..40 plus 第520回/第521回)
        // and 4 side-story rows (番外01/02 with their halves).
        assert_eq!(meta.count, Some(521));
        assert_eq!(meta.chapters.len(), 46);
        let mains: Vec<i32> = meta
            .chapters
            .iter()
            .filter(|c| !c.is_bonus)
            .map(|c| c.number_x100)
            .collect();
        assert_eq!(mains.first(), Some(&100));
        // The tail: 第520回/第521回 are chapters, 番外… rows are bonuses.
        assert!(meta.chapters.iter().any(|c| c.number_x100 == 52000));
        assert!(meta.chapters.iter().any(|c| c.number_x100 == 52100));
        assert_eq!(meta.chapters.iter().filter(|c| c.is_bonus).count(), 4);
        assert!(
            meta.chapters
                .iter()
                .filter(|c| c.is_bonus)
                .all(|c| c.title.as_deref().is_some_and(|t| t.starts_with("番外")))
        );
    }

    #[test]
    fn chapters_malformed_is_empty() {
        assert!(parse_chapters("").chapters.is_empty());
        assert!(parse_chapters("<html></html>").chapters.is_empty());
    }
}
