//! Manta (manta.net) official-source search + chapter metadata.
//!
//! Search is the site's own JSON API: `GET /manta/v1/search/series?q=` answers
//! with rows carrying the series id, the English title, the cover
//! (`image.title.downloadUrl`), tags, creators, `isCompleted` and the episode
//! count. The full episode list is server-rendered into
//! `/en/series/{id}`'s `__NEXT_DATA__` (`pageProps.initialSeries.episodes`:
//! `ord`, `data.title`, `openAt`) — Solo Leveling: all 201.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use chrono::NaiveDate;

use crate::services::external::chapter_title;
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

const SEARCH_BASE: &str = "https://manta.net/manta/v1/search/series";
const SERIES_BASE: &str = "https://manta.net/en/series";

/// Browser-like User-Agent; the site rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Defensive cap on one search answer.
const SEARCH_MAX_HITS: usize = 50;
/// Rows read from one series page (a hostile answer must not blow up memory).
const EPISODE_CAP: usize = 5000;
/// Longest stored episode title.
const MAX_TITLE_CHARS: usize = 512;
/// Longest stored synopsis taken from a search answer.
const MAX_DESCRIPTION_CHARS: usize = 2000;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "manta".to_string(),
        ..OfficialMeta::default()
    }
}

/// Search the English catalogue.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_BASE)?;
    url.query_pairs_mut().append_pair("q", title);
    let body = get_text(client, url).await?;
    Ok(parse_search(&body))
}

/// Pure parser over the search answer: `data[]` rows with the series id,
/// `data.title.en`, the cover, tags, creators and the completion state.
/// Malformed JSON yields an empty vector — never a panic.
pub fn parse_search(json: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.get("data").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for item in items.iter().take(SEARCH_MAX_HITS) {
        let Some(id) = item.get("id").and_then(|v| v.as_i64()) else {
            continue;
        };
        let Some(title) = item
            .pointer("/data/title/en")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        if hits.iter().any(|h| h.id == id.to_string()) {
            continue;
        }
        let poster_url = item
            .pointer("/image/title/downloadUrl")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let author = item
            .pointer("/data/creators")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|c| c.get("name").and_then(|v| v.as_str()))
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|a| !a.is_empty());
        let genres = item
            .pointer("/data/tags")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|t| t.pointer("/name/en").and_then(|v| v.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let description = item
            .pointer("/data/description/long")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(|d| d.chars().take(MAX_DESCRIPTION_CHARS).collect());
        let status = item
            .pointer("/data/isCompleted")
            .and_then(|v| v.as_bool())
            .map(|done| if done { "completed" } else { "ongoing" }.to_string());
        let chapters = item
            .get("episodeCount")
            .and_then(|v| v.as_i64())
            .and_then(|n| i32::try_from(n).ok());
        hits.push(OfficialHit::with_extra(
            id.to_string(),
            title.chars().take(MAX_TITLE_CHARS).collect::<String>(),
            HitExtra {
                poster_url,
                description,
                author,
                status,
                genres,
                chapters,
                ..HitExtra::default()
            },
        ));
    }
    hits
}

/// One episode row of the series page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MantaEpisode {
    pub ord: i32,
    pub title: String,
    pub release_date: Option<NaiveDate>,
}

/// Pure parser over `/en/series/{id}`: the `__NEXT_DATA__`
/// `pageProps.initialSeries.episodes` list (`ord`, `data.title`, `openAt`).
pub fn parse_episodes(html: &str) -> Option<Vec<MantaEpisode>> {
    let json = official_meta::between(
        html,
        "<script id=\"__NEXT_DATA__\" type=\"application/json\">",
        "</script>",
    )
    .or_else(|| official_meta::between(html, "<script id=\"__NEXT_DATA__\">", "</script>"))?;
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let items = value
        .pointer("/props/pageProps/initialSeries/episodes")
        .and_then(|v| v.as_array())?;
    let mut rows = Vec::new();
    for item in items.iter().take(EPISODE_CAP) {
        let Some(ord) = item.get("ord").and_then(|v| v.as_i64()) else {
            continue;
        };
        let Some(title) = item
            .pointer("/data/title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let release_date = item
            .get("openAt")
            .and_then(|v| v.as_str())
            .and_then(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok())
            .map(|dt| dt.date_naive());
        rows.push(MantaEpisode {
            ord: i32::try_from(ord).ok()?,
            title: title.chars().take(MAX_TITLE_CHARS).collect(),
            release_date,
        });
    }
    Some(rows)
}

/// Number the rows: the site's own `ord`; English side content (`Prologue`,
/// `Bonus Episode …`) becomes bonuses.
pub fn chapters_from_rows(rows: Vec<MantaEpisode>) -> Vec<ChapterMeta> {
    rows.into_iter()
        .map(|row| {
            let is_bonus = is_bonus_title(&row.title);
            ChapterMeta {
                number_x100: if is_bonus {
                    0
                } else {
                    // A hostile `ord` must not overflow the x100 encoding.
                    row.ord.checked_mul(100).unwrap_or(0)
                },
                title: Some(row.title),
                release_date: row.release_date,
                is_bonus,
                label: None,
            }
        })
        .collect()
}

/// English side-content markers (word-boundary at the start), so
/// `Bonus Episode 21` is a bonus while `Episode 5: The Special One` stays a
/// chapter.
fn is_bonus_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    [
        "prologue",
        "epilogue",
        "bonus",
        "special",
        "side story",
        "extra",
        "notice",
        "preview",
        "afterword",
    ]
    .iter()
    .any(|marker| chapter_title::starts_with_word(&lower, marker))
}

/// Fetch one series' episode list and map it onto the shared chapter model.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    if !official_meta::is_digit_id(source_id) {
        return Ok(empty_meta());
    }
    let url = url::Url::parse(&format!("{SERIES_BASE}/{source_id}"))?;
    let html = get_text(client, url).await?;
    let Some(rows) = parse_episodes(&html) else {
        return Ok(empty_meta());
    };
    if rows.is_empty() {
        return Ok(empty_meta());
    }
    if rows.len() >= EPISODE_CAP {
        // The cap cut the list off; `apply_meta`'s orphan cleanup would drop
        // rows the walk never saw, so refuse the truncated list.
        anyhow::bail!("manta: episode row cap hit, refusing a truncated list");
    }
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "manta".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    })
}

/// GET a page and read the body with a hard size cap.
async fn get_text(client: &reqwest::Client, url: url::Url) -> anyhow::Result<String> {
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT, "application/json, text/html")
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/manta_search.json");
    const SERIES_FIXTURE: &str = include_str!("../../../tests/fixtures/manta_series.html");

    #[test]
    fn search_fixture_parses_rows() {
        let hits = parse_search(SEARCH_FIXTURE);
        assert!(!hits.is_empty());
        let solo = hits
            .iter()
            .find(|h| h.title == "Solo Leveling")
            .expect("the Solo Leveling row");
        assert_eq!(solo.id, "2729");
        assert_eq!(solo.extra.status.as_deref(), Some("completed"));
        assert_eq!(solo.extra.chapters, Some(201));
        assert!(
            solo.extra
                .poster_url
                .as_deref()
                .is_some_and(|u| u.contains("static.mantacdn.net")),
            "{:?}",
            solo.extra.poster_url
        );
        assert!(
            solo.extra.genres.iter().any(|g| g == "Fantasy/Sci-Fi"),
            "{:?}",
            solo.extra.genres
        );
        // The search answer carries no creators/description — those live on
        // the series page (and the add flow can read them from the catalog).
        assert!(solo.extra.author.is_none());
        // Malformed answers are empty, never a panic.
        assert!(parse_search("").is_empty());
        assert!(parse_search("{}").is_empty());
    }

    #[test]
    fn series_fixture_parses_the_full_run() {
        // The fixture keeps the first two and the last two of the 201 episodes.
        let rows = parse_episodes(SERIES_FIXTURE).expect("fixture has __NEXT_DATA__");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].ord, 1);
        assert_eq!(rows[0].title, "Prologue");
        assert!(rows[0].release_date.is_some());
        assert_eq!(rows.last().map(|r| r.ord), Some(201));
        let chapters = chapters_from_rows(rows);
        assert!(chapters[0].is_bonus, "Prologue is side content");
        assert_eq!(chapters[0].number_x100, 0);
        assert_eq!(chapters[1].number_x100, 200);
        assert!(chapters[3].is_bonus, "Bonus Episode … is side content");
        // Malformed pages are None, never a panic.
        assert!(parse_episodes("").is_none());
        assert!(parse_episodes("<html></html>").is_none());
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
