//! Narō (ncode.syosetu.com) detail enrichment: episode titles/dates + status.
//! Counts are owned by the existing `chapter_count::syosetu` provider.

use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub async fn fetch_meta(client: &reqwest::Client, ncode: &str) -> anyhow::Result<OfficialMeta> {
    let mut chapters: Vec<ChapterMeta> = Vec::new();
    for page in 1..=50 {
        let url = format!("https://ncode.syosetu.com/{ncode}/?p={page}");
        let body = client
            .get(&url)
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let rows = parse_episode_list(&body);
        if rows.is_empty() {
            break;
        }
        chapters.extend(rows);
    }

    let schedule = fetch_status(client, ncode).await.unwrap_or(None);
    Ok(OfficialMeta {
        source: "syosetu".to_string(),
        count: (!chapters.is_empty()).then_some(chapters.len() as i32),
        chapters,
        update_schedule: schedule,
        next_update_at: None,
    })
}

async fn fetch_status(client: &reqwest::Client, ncode: &str) -> anyhow::Result<Option<String>> {
    let url = format!("https://ncode.syosetu.com/novelview/infotop/ncode/{ncode}/");
    let body = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(parse_status(&body))
}

/// Rows carry `p-eplist__subtitle` (title) and `p-eplist__update` (date).
pub fn parse_episode_list(html: &str) -> Vec<ChapterMeta> {
    let mut rows = Vec::new();
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
        let number = rows.len() as f64 + 1.0;
        if let Some(mut c) = ChapterMeta::from_f64(number) {
            c.title = (!title.is_empty()).then_some(title);
            c.release_date = date;
            rows.push(c);
        }
    }
    rows
}

/// Status line contains `連載中` (ongoing) or `完結` (completed). Return a short
/// human label; there is no free-text frequency on Narō.
pub fn parse_status(html: &str) -> Option<String> {
    let text = official_meta::strip_tags(html);
    if text.contains("連載中") {
        Some("連載中".to_string())
    } else if text.contains("完結") {
        Some("完結".to_string())
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
        let rows = parse_episode_list(html);
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
}
