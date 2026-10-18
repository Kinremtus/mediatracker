//! Jinjiang (晋江文学城) official-source search + chapter metadata.
//!
//! The site serves GBK: the query is GBK bytes percent-encoded into
//! `/search.php?kw=&t=1` and every answer (search results, novel pages) is
//! decoded from GBK. Search results are `h3.title` blocks (the `novelid`
//! link, the title, the date) followed by an `intro` line and an `info` line
//! (`作者：… ┃ 进度：… ┃ 文章类型：…`). The chapter list is the novel page's
//! `chapterid=` links (`楔子`, `第N章 …`), classified through the shared CN
//! counter.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use encoding_rs::GBK;

use crate::services::external::chapter_title::{self, RawRow, is_cn_bonus_title};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

const SEARCH_BASE: &str = "https://www.jjwxc.net/search.php";
const BOOK_BASE: &str = "https://www.jjwxc.net/onebook.php";

/// Browser-like User-Agent; the site rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Defensive cap on one search answer.
const SEARCH_MAX_HITS: usize = 50;
/// Rows read from one novel page (a hostile answer must not blow up memory).
const CHAPTER_CAP: usize = 5000;
/// Longest stored episode title.
const MAX_TITLE_CHARS: usize = 512;
/// Longest stored synopsis taken from a search answer.
const MAX_DESCRIPTION_CHARS: usize = 2000;
/// Hard cap on one GBK page.
const MAX_PAGE_BYTES: usize = 8 * 1024 * 1024;
/// How far a tag search may reach from its anchor.
const WINDOW: usize = 8192;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "jinjiang".to_string(),
        ..OfficialMeta::default()
    }
}

/// Search the Chinese catalogue (the query travels as GBK bytes).
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let (encoded, _, _) = GBK.encode(title);
    let kw: String = url::form_urlencoded::byte_serialize(&encoded).collect();
    let url = format!("{SEARCH_BASE}?kw={kw}&t=1");
    let html = get_gbk(client, &url).await?;
    Ok(parse_search(&html))
}

/// Pure parser over the search page: `h3.title` blocks with the novelid link,
/// the title and the date, plus the `intro` and `info` lines. Malformed HTML
/// yields an empty vector — never a panic.
pub fn parse_search(html: &str) -> Vec<OfficialHit> {
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("<h3 class=\"title\">") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW * 2);
        let window = &html[at..hi];
        let Some(h3_end) = window.find("</h3>") else {
            break;
        };
        let h3 = &window[..h3_end];
        // The block runs to the end of the `info` line (the h3, intro and info
        // are sibling divs).
        cursor = at
            + official_meta::between(window, "class=\"info\">", "</div>")
                .map(|info| {
                    window
                        .find("class=\"info\">")
                        .map(|start| start + "class=\"info\">".len() + info.len() + "</div>".len())
                        .unwrap_or(h3_end)
                })
                .unwrap_or(h3_end);
        let Some(id) = official_meta::between(h3, "novelid=", "\"")
            .map(str::to_string)
            .filter(|id| official_meta::is_digit_id(id))
        else {
            continue;
        };
        if hits.iter().any(|h| h.id == id) {
            continue;
        }
        let Some(title) = official_meta::between(h3, ">", "</a>")
            .map(official_meta::strip_tags)
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let description = official_meta::between(window, "class=\"intro\">", "</div>")
            .map(official_meta::strip_tags)
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .map(|d| d.chars().take(MAX_DESCRIPTION_CHARS).collect());
        let info = official_meta::between(window, "class=\"info\">", "</div>")
            .map(official_meta::strip_tags)
            .unwrap_or_default();
        let author = official_meta::between(&info, "作者：", "┃")
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty());
        let status = official_meta::between(&info, "进度：", "┃")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| match s.as_str() {
                "完结" => "completed".to_string(),
                "连载中" | "连载" => "ongoing".to_string(),
                "暂停" => "hiatus".to_string(),
                other => other.to_string(),
            });
        hits.push(OfficialHit::with_extra(
            id,
            title.chars().take(MAX_TITLE_CHARS).collect::<String>(),
            HitExtra {
                description,
                author,
                status,
                ..HitExtra::default()
            },
        ));
        if hits.len() >= SEARCH_MAX_HITS {
            break;
        }
    }
    hits
}

/// One chapter row of the novel page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JinjiangChapter {
    pub title: String,
}

/// Pure parser over a novel page: the `chapterid=` links with their titles.
pub fn parse_chapters(html: &str) -> Vec<JinjiangChapter> {
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("chapterid=") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW);
        let Some(gt_rel) = html[at..hi].find('>') else {
            break;
        };
        let title_start = at + gt_rel + 1;
        let Some(end_rel) = html[title_start..hi].find("</a>") else {
            break;
        };
        let title = official_meta::strip_tags(&html[title_start..title_start + end_rel]);
        let title = title.trim();
        cursor = title_start + end_rel;
        if title.is_empty() {
            continue;
        }
        rows.push(JinjiangChapter {
            title: title.chars().take(MAX_TITLE_CHARS).collect(),
        });
        if rows.len() >= CHAPTER_CAP {
            break;
        }
    }
    rows
}

/// Number the rows by position, oldest first: Jinjiang chapter titles are
/// free-form (`重生第一`, `第1章 …`), so the site's own list order is the
/// numbering. Side content (`楔子`, `番外`, notices) becomes bonuses and never
/// consumes a number.
pub fn chapters_from_rows(rows: Vec<JinjiangChapter>) -> Vec<ChapterMeta> {
    let raw: Vec<RawRow> = rows
        .into_iter()
        .map(|row| RawRow {
            is_bonus: is_cn_bonus_title(&row.title),
            title: Some(row.title),
            release_date: None,
        })
        .collect();
    chapter_title::positional_rows(raw)
}

/// Fetch one novel's chapter list and map it onto the shared chapter model.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(source_id) {
        return Ok(empty_meta());
    }
    let html = get_gbk(client, &format!("{BOOK_BASE}?novelid={source_id}")).await?;
    let rows = parse_chapters(&html);
    if rows.is_empty() {
        return Ok(empty_meta());
    }
    if rows.len() >= CHAPTER_CAP {
        // The cap cut the list off; `apply_meta`'s orphan cleanup would drop
        // rows the walk never saw, so refuse the truncated list.
        anyhow::bail!("jinjiang: chapter row cap hit, refusing a truncated list");
    }
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "jinjiang".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    })
}

/// GET a page, cap its size and decode it from GBK.
async fn get_gbk(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?;
    if let Some(len) = response.content_length()
        && len > MAX_PAGE_BYTES as u64
    {
        anyhow::bail!("response too large: {len} bytes");
    }
    let bytes = response.bytes().await?;
    if bytes.len() > MAX_PAGE_BYTES {
        anyhow::bail!("response too large: over {MAX_PAGE_BYTES} bytes");
    }
    let (text, _, _) = GBK.decode(&bytes);
    Ok(text.into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/jinjiang_search.html");
    const BOOK_FIXTURE: &str = include_str!("../../../tests/fixtures/jinjiang_book.html");

    #[test]
    fn search_fixture_parses_results() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert_eq!(hits.len(), 2, "the trimmed fixture keeps two results");
        assert_eq!(hits[0].id, "2368172");
        assert_eq!(hits[0].title, "魔道祖师");
        assert_eq!(hits[0].extra.author.as_deref(), Some("墨香铜臭"));
        assert_eq!(hits[0].extra.status.as_deref(), Some("completed"));
        assert!(
            hits[0]
                .extra
                .description
                .as_deref()
                .is_some_and(|d| d.contains("魏无羡")),
            "{:?}",
            hits[0].extra.description
        );
        // Malformed pages are empty, never a panic.
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn book_fixture_parses_chapters() {
        let rows = parse_chapters(BOOK_FIXTURE);
        assert!(rows.len() >= 3, "got {} rows", rows.len());
        assert_eq!(rows[0].title, "重生第一");
        let chapters = chapters_from_rows(rows);
        assert!(
            !chapters[0].is_bonus,
            "a free-form chapter title is a main row"
        );
        assert_eq!(chapters[0].number_x100, 100, "positional numbering");
    }

    #[test]
    fn chapters_number_by_position_around_bonuses() {
        let rows = vec![
            JinjiangChapter {
                title: "楔子".to_string(),
            },
            JinjiangChapter {
                title: "重生第一".to_string(),
            },
            JinjiangChapter {
                title: "重生第二".to_string(),
            },
            JinjiangChapter {
                title: "番外 小剧场".to_string(),
            },
            JinjiangChapter {
                title: "再遇第一".to_string(),
            },
        ];
        let chapters = chapters_from_rows(rows);
        assert!(chapters[0].is_bonus, "楔子");
        assert_eq!(chapters[0].number_x100, 0);
        assert_eq!(chapters[1].number_x100, 100);
        assert_eq!(chapters[2].number_x100, 200);
        assert!(chapters[3].is_bonus, "番外 never consumes a number");
        assert_eq!(chapters[4].number_x100, 300);
        assert_eq!(chapter_title::main_count(&chapters), 3);
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
