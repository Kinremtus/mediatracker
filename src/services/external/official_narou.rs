//! Narō (ncode.syosetu.com) detail enrichment: episode titles/dates + status.
//! The main-only count written here supersedes the coarse
//! `chapter_count::syosetu` provider (which counts bonuses too); once the
//! official stamp is set that provider is skipped.

use crate::services::external::chapter_title::{self, RawRow, main_count};
use crate::services::external::official_meta::{self, OfficialMeta};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Hard cap on rows accumulated across the paged walk.
const MAX_ROWS: usize = 20_000;

pub async fn fetch_meta(client: &reqwest::Client, ncode: &str) -> anyhow::Result<OfficialMeta> {
    // The bound id lands in a URL path: only the ncode alphabet may pass, so a
    // crafted binding cannot steer the request to another path.
    if !crate::services::external::official_meta::is_safe_path_id(ncode) {
        return Ok(OfficialMeta::default());
    }
    // Rows are numbered once, after every page is collected: numbering each
    // page on its own would restart at 1 and collide across pages.
    let mut raw: Vec<RawRow> = Vec::new();
    for page in 1..=50 {
        let url = format!("https://ncode.syosetu.com/{ncode}/?p={page}");
        let resp = client
            .get(&url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?;
        let body = official_meta::body_capped(resp, official_meta::MAX_DETAIL_BYTES).await?;
        let rows = parse_episode_rows(&body);
        if rows.is_empty() {
            break;
        }
        raw.extend(rows);
        if raw.len() >= MAX_ROWS {
            break;
        }
    }
    let chapters = chapter_title::positional_rows(raw);

    let schedule = fetch_status(client, ncode).await.unwrap_or(None);
    Ok(OfficialMeta {
        source: "syosetu".to_string(),
        count: (!chapters.is_empty()).then_some(main_count(&chapters) as i32),
        chapters,
        update_schedule: schedule,
        next_update_at: None,
    })
}

async fn fetch_status(client: &reqwest::Client, ncode: &str) -> anyhow::Result<Option<String>> {
    let url = format!("https://ncode.syosetu.com/novelview/infotop/ncode/{ncode}/");
    let resp = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?;
    let body = official_meta::body_capped(resp, official_meta::MAX_DETAIL_BYTES).await?;
    Ok(parse_status(&body))
}

/// Rows carry `p-eplist__subtitle` (title) and `p-eplist__update` (date).
/// Narō's own episode index counts bonuses (a `プロローグ` is episode 1), so
/// the rows are flagged here and numbered later by
/// [`chapter_title::positional_rows`].
pub fn parse_episode_rows(html: &str) -> Vec<RawRow> {
    let mut raw: Vec<RawRow> = Vec::new();
    let mut rest = html;
    while let Some(pos) = rest.find("p-eplist__subtitle") {
        let after = &rest[pos..];
        let Some(gt) = after.find('>') else { break };
        let seg = &after[gt + 1..];
        let Some(end) = seg.find("</a>") else { break };
        let title = official_meta::strip_tags(&seg[..end]);
        rest = &seg[end + 4..];

        // The update span follows the subtitle cell within the same row.
        // Its text may carry a time (and a revision `<span>`) after the date,
        // so feed only the first whitespace-separated token to the parser.
        let date = rest.find("p-eplist__update").and_then(|i| {
            let s = &rest[i..];
            let s = s.get(s.find('>')? + 1..)?;
            let date_part = s.split_whitespace().next()?;
            official_meta::parse_date(date_part)
        });
        let is_bonus = chapter_title::is_jp_positional_bonus(&title);
        raw.push(RawRow {
            title: (!title.is_empty()).then_some(title),
            release_date: date,
            is_bonus,
        });
    }
    raw
}

/// Status badge carries `連載中` (ongoing) or `完結` (completed). The badge is
/// read in isolation because the description can mention the other state
/// ("完結後の番外編はこちらで連載中です"); on the page-wide fallback `完結`
/// wins. There is no free-text frequency on Narō.
pub fn parse_status(html: &str) -> Option<String> {
    let text = official_meta::between(html, "p-infotop-type__type", "</span>")
        .map(official_meta::strip_tags)
        .unwrap_or_else(|| official_meta::strip_tags(html));
    if text.contains("完結") {
        Some("完結".to_string())
    } else if text.contains("連載中") {
        Some("連載中".to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_narou_fixture() {
        let html = include_str!("../../../tests/fixtures/narou_episodes.html");
        let rows = parse_episode_rows(html);
        assert!(!rows.is_empty());
        assert!(rows[0].title.is_some());
        assert!(rows[0].release_date.is_some());
    }

    #[test]
    fn status_ongoing_and_complete() {
        assert_eq!(parse_status("<div>連載中</div>").as_deref(), Some("連載中"));
        assert_eq!(parse_status("<div>完結</div>").as_deref(), Some("完結"));
        assert_eq!(parse_status("<div>x</div>"), None);
    }
    #[tokio::test]
    async fn unsafe_source_ids_are_rejected_without_network() {
        let client = crate::services::external::http_client();
        for id in ["", "../x", "a/b", "n1?x=1"] {
            let meta = fetch_meta(&client, id)
                .await
                .expect("a bad id must not hit the network");
            assert!(meta.chapters.is_empty(), "id {id:?} must be rejected");
        }
    }
}
