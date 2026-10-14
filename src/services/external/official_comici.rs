//! comici (コミチ) platform manga sources: Champion Cross (Akita Shoten),
//! Takecomic (Takeshobo), PASH! neo (Shufu to Seikatsusha), Young Champion Web
//! (Akita Shoten) and Big Comics (Shogakukan) — five publisher sites running
//! one platform, so one parser covers them all. A binding id carries both the
//! site and the series hash: `championcross.jp/452b0e17e1bf3`.
//!
//! Search and the episode list come from the platform's public JSON endpoints:
//! `/api/search?q=…` names one site's matching series, and
//! `/api/series?seriesHash=…` + `/api/episodes?seriesHash=…&episodeFrom=…&episodeTo=…`
//! report the episode count and every row's title and publication date. The
//! per-series `/series/{hash}/rss` feed is the platform's other public listing;
//! it carries only the 25 newest items, so it stands in for the episode
//! endpoint ONLY when the whole series fits that cap — a truncated list must
//! never reach `apply_meta`, whose orphan cleanup would drop rows the answer
//! never saw.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use chrono::NaiveDate;

use crate::services::external::chapter_title::{self, ComiciTitle, EpisodeRow, RawRow, ShortTitle};
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

/// The platform sites, in hit order. One parser, five catalogs: the same JSON
/// shape on every host.
pub const SITES: &[&str] = &[
    "championcross.jp",
    "takecomic.jp",
    "comicpash.jp",
    "youngchampion.jp",
    "bigcomics.jp",
];

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
/// Rows requested per search answer and per site (an exact title match is near
/// the top; anything past this cap is not worth binding).
const SEARCH_SIZE: usize = 10;
/// Episodes requested per window; the endpoint answers a full window in one
/// shot, so the walk is `ceil(numEpisodes / EPISODE_WINDOW)` requests.
const EPISODE_WINDOW: i64 = 500;
/// Largest series walked. Anything bigger would need an unbounded walk, so it
/// is treated as unsupported (the answer is an error, never a partial list).
const MAX_EPISODES: i64 = 5000;
/// The RSS feed is only a fallback for series that fit its item cap whole.
const RSS_MAX_ITEMS: i64 = 25;
/// Longest stored episode title (a hostile answer must not bloat the DB).
const MAX_TITLE_CHARS: usize = 512;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "comici".to_string(),
        ..OfficialMeta::default()
    }
}

/// One episode row before the numbering decision: the title, its date, and how
/// the title maps onto the chapter sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub title: String,
    pub release_date: Option<NaiveDate>,
    pub kind: ComiciTitle,
}

/// Search every platform site in parallel and merge the hits. A failing site
/// only drops its own hits (the others still answer); only when every site
/// fails is the search an error. Hits carry `site/series_hash` ids so a
/// binding records which catalog it points at.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut set = tokio::task::JoinSet::new();
    for &site in SITES {
        let client = client.clone();
        let title = title.to_string();
        set.spawn(async move { (site, fetch_site_hits(&client, site, &title).await) });
    }
    let mut hits: Vec<OfficialHit> = Vec::new();
    let mut failures = 0usize;
    let mut last_error: Option<anyhow::Error> = None;
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok((_, Ok(mut site_hits))) => hits.append(&mut site_hits),
            Ok((site, Err(e))) => {
                failures += 1;
                tracing::warn!(site, error = %e, "comici: site search failed");
                last_error = Some(e);
            }
            Err(e) => {
                failures += 1;
                tracing::warn!(error = %e, "comici: site search task failed");
                last_error = Some(anyhow::anyhow!(e));
            }
        }
    }
    if failures == SITES.len() {
        return Err(
            last_error.unwrap_or_else(|| anyhow::anyhow!("comici: all site searches failed"))
        );
    }
    Ok(hits)
}

/// GET `{site}/api/search` and parse it.
async fn fetch_site_hits(
    client: &reqwest::Client,
    site: &str,
    title: &str,
) -> anyhow::Result<Vec<OfficialHit>> {
    let body = fetch_json(
        client,
        site,
        "/api/search",
        &[
            ("q", title),
            ("page", "1"),
            ("size", &SEARCH_SIZE.to_string()),
        ],
    )
    .await?;
    Ok(parse_search(&body, site))
}

/// Pure parser over `/api/search`: the matching series of one site, `manga`
/// entries only (the platform also lists novels and columns). Malformed JSON
/// yields an empty vector — never a panic.
pub fn parse_search(json: &str, site: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value
        .pointer("/searchResult/series/series")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    let mut hits = Vec::with_capacity(items.len());
    for item in items.iter().take(SEARCH_SIZE) {
        if item
            .get("category")
            .and_then(|v| v.as_str())
            .is_some_and(|category| category != "manga")
        {
            continue;
        }
        let Some(hash) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if !is_series_hash(hash) {
            continue;
        }
        let Some(name) = item
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        hits.push(OfficialHit {
            id: format!("{site}/{hash}"),
            title: name.chars().take(MAX_TITLE_CHARS).collect(),
        });
    }
    hits
}

/// Pure parser over `/api/series`: the reported episode count (`numEpisodes`),
/// the walk size. `None` for a malformed answer or a summary without it.
pub fn parse_series(json: &str) -> Option<i64> {
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    value
        .pointer("/series/summary/numEpisodes")
        .and_then(|v| v.as_i64())
}

/// Pure parser over one `/api/episodes` window: every row with a usable title
/// (classification comes from [`chapter_title::classify_comici_title`]) and
/// its publication day. Malformed JSON yields an empty vector.
pub fn parse_episodes(json: &str) -> Vec<Row> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.pointer("/series/episodes").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .take(EPISODE_WINDOW as usize)
        .filter_map(row_from_episode)
        .collect()
}

/// One episode object -> a row. A title-less row is dropped (nothing to
/// classify or display).
fn row_from_episode(item: &serde_json::Value) -> Option<Row> {
    let title = item
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())?;
    let title: String = title.chars().take(MAX_TITLE_CHARS).collect();
    let release_date = item
        .get("datePublished")
        .and_then(|v| v.as_i64())
        .and_then(date_from_epoch);
    let kind = chapter_title::classify_comici_title(&title);
    Some(Row {
        title,
        release_date,
        kind,
    })
}

/// Number the rows.
///
/// A series whose titles carry real numbers keeps them (`Karte.1` -> 1,
/// `第39話` -> 39, split parts `第16話後編` -> 16-20); marker rows and
/// numberless non-marker rows follow as unnumbered bonus rows. A series
/// without usable numbers (volume-scoped `10巻－第1歩`, gag counters
/// `1ミャオ`, text-only labels) is numbered by position, like the other
/// numberless sources — a bonus marker never consumes a number there either.
pub fn chapters_from_rows(rows: Vec<Row>) -> Vec<ChapterMeta> {
    let numbered = rows
        .iter()
        .filter(|row| {
            matches!(
                row.kind,
                ComiciTitle::Number(ShortTitle::Base(_) | ShortTitle::Part(..))
            )
        })
        .count();
    let plain = rows
        .iter()
        .filter(|row| row.kind == ComiciTitle::Plain)
        .count();
    if numbered > 0 && numbered >= plain {
        let typed: Vec<EpisodeRow> = rows
            .into_iter()
            .map(|row| EpisodeRow {
                title: Some(row.title),
                release_date: row.release_date,
                kind: match row.kind {
                    ComiciTitle::Number(kind) => kind,
                    ComiciTitle::Bonus | ComiciTitle::Plain => ShortTitle::Unnumbered,
                },
            })
            .collect();
        chapter_title::rows_to_chapters(typed)
    } else {
        let raw: Vec<RawRow> = rows
            .into_iter()
            .map(|row| RawRow {
                title: Some(row.title),
                release_date: row.release_date,
                is_bonus: matches!(
                    row.kind,
                    ComiciTitle::Bonus | ComiciTitle::Number(ShortTitle::Fractional(_))
                ),
            })
            .collect();
        chapter_title::positional_rows(raw)
    }
}

/// Fetch one series' full episode list and map it onto the shared chapter
/// model. The counter is the highest main chapter number.
pub async fn fetch_meta(client: &reqwest::Client, source_id: &str) -> anyhow::Result<OfficialMeta> {
    let Some((site, hash)) = split_source_id(source_id) else {
        return Ok(empty_meta());
    };
    let series = fetch_json(client, site, "/api/series", &[("seriesHash", hash)]).await?;
    let Some(num_episodes) = parse_series(&series) else {
        return Ok(empty_meta());
    };
    if num_episodes <= 0 {
        return Ok(empty_meta());
    }
    if num_episodes > MAX_EPISODES {
        anyhow::bail!("comici: series too large ({num_episodes} episodes)");
    }
    let rows = fetch_rows(client, site, hash, num_episodes).await?;
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "comici".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        ..OfficialMeta::default()
    })
}

/// The window walk, or — when it cannot deliver the full list — the complete
/// RSS feed. A truncated list never escapes this function: the walk is
/// accepted only when it returned exactly `num_episodes` rows, and the RSS
/// only when its item count matches the same number.
async fn fetch_rows(
    client: &reqwest::Client,
    site: &str,
    hash: &str,
    num_episodes: i64,
) -> anyhow::Result<Vec<Row>> {
    let walk_error = match walk_episodes(client, site, hash, num_episodes).await {
        Ok(rows) if rows.len() as i64 == num_episodes => return Ok(rows),
        Ok(rows) => anyhow::anyhow!(
            "comici: episode walk incomplete ({}/{} rows)",
            rows.len(),
            num_episodes
        ),
        Err(e) => e,
    };
    if num_episodes > RSS_MAX_ITEMS {
        return Err(walk_error);
    }
    match fetch_rss_rows(client, site, hash).await {
        Ok(rows) => accept_rss(rows, num_episodes),
        Err(rss_error) => {
            tracing::warn!(site, error = %rss_error, "comici: rss fallback failed");
            Err(walk_error)
        }
    }
}

/// The RSS fallback is trusted only when the feed covered the whole series.
fn accept_rss(rows: Vec<Row>, num_episodes: i64) -> anyhow::Result<Vec<Row>> {
    if rows.len() as i64 == num_episodes {
        Ok(rows)
    } else {
        anyhow::bail!(
            "comici: rss fallback incomplete ({}/{} rows)",
            rows.len(),
            num_episodes
        )
    }
}

/// Walk `/api/episodes` in windows; any failed window fails the whole walk
/// (a partial answer must not be applied).
async fn walk_episodes(
    client: &reqwest::Client,
    site: &str,
    hash: &str,
    num_episodes: i64,
) -> anyhow::Result<Vec<Row>> {
    let mut rows: Vec<Row> = Vec::new();
    let mut from = 1i64;
    while from <= num_episodes {
        let to = (from + EPISODE_WINDOW - 1).min(num_episodes);
        let from_s = from.to_string();
        let to_s = to.to_string();
        let body = fetch_json(
            client,
            site,
            "/api/episodes",
            &[
                ("seriesHash", hash),
                ("episodeFrom", &from_s),
                ("episodeTo", &to_s),
            ],
        )
        .await?;
        rows.extend(parse_episodes(&body));
        from = to + 1;
    }
    Ok(rows)
}

/// Fetch the per-series RSS feed and parse it.
async fn fetch_rss_rows(
    client: &reqwest::Client,
    site: &str,
    hash: &str,
) -> anyhow::Result<Vec<Row>> {
    let url = url::Url::parse(&format!("https://{site}/series/{hash}/rss"))?;
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await?
        .error_for_status()?;
    let body = official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await?;
    Ok(parse_rss(&body))
}

/// Pure parser over the per-series RSS feed, oldest item first (the feed lists
/// the newest item first). Only `title` and `pubDate` are read; the item's
/// `guid` is the episode hash, which nothing here needs. Malformed XML yields
/// an empty vector — never a panic.
pub fn parse_rss(xml: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = xml[cursor..].find("<item>") {
        let start = cursor + relative + "<item>".len();
        let Some(end_rel) = xml[start..].find("</item>") else {
            break;
        };
        let item = &xml[start..start + end_rel];
        cursor = start + end_rel + "</item>".len();
        let Some(raw_title) = rss_tag(item, "title") else {
            continue;
        };
        let title = raw_title.trim();
        if title.is_empty() {
            continue;
        }
        let title: String = title.chars().take(MAX_TITLE_CHARS).collect();
        let release_date = rss_tag(item, "pubDate").and_then(parse_rss_date);
        let kind = chapter_title::classify_comici_title(&title);
        rows.push(Row {
            title,
            release_date,
            kind,
        });
    }
    rows.reverse();
    rows
}

/// The text of one `<tag>…</tag>` inside an RSS item, with an optional CDATA
/// wrapper stripped.
fn rss_tag<'a>(item: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let raw = official_meta::between(item, &open, &close)?;
    Some(
        raw.strip_prefix("<![CDATA[")
            .and_then(|s| s.strip_suffix("]]>"))
            .unwrap_or(raw),
    )
}

/// RFC 2822 (`Mon, 05 Oct 2026 15:00:00 GMT`) -> the UTC day.
fn parse_rss_date(raw: &str) -> Option<NaiveDate> {
    chrono::DateTime::parse_from_rfc2822(raw.trim())
        .ok()
        .map(|dt| dt.date_naive())
}

/// GET one site's JSON endpoint and read the body with a hard size cap.
async fn fetch_json(
    client: &reqwest::Client,
    site: &str,
    path: &str,
    query: &[(&str, &str)],
) -> anyhow::Result<String> {
    let mut url = url::Url::parse(&format!("https://{site}{path}"))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

/// Split a `site/series_hash` binding id. The site must be one of [`SITES`]
/// (exact match — the id lands in a URL) and the hash a short alphanumeric
/// token.
fn split_source_id(id: &str) -> Option<(&'static str, &str)> {
    let (site, hash) = id.split_once('/')?;
    let site = SITES.iter().copied().find(|known| *known == site)?;
    is_series_hash(hash).then_some((site, hash))
}

/// True when `hash` is the plain alphanumeric series token the platform uses
/// (`452b0e17e1bf3`).
fn is_series_hash(hash: &str) -> bool {
    !hash.is_empty() && hash.len() <= 32 && hash.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Epoch seconds -> the UTC day.
fn date_from_epoch(secs: i64) -> Option<NaiveDate> {
    chrono::DateTime::from_timestamp(secs, 0).map(|dt| dt.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_FIXTURE: &str = include_str!("../../../tests/fixtures/comici_search.json");
    const SERIES_FIXTURE: &str = include_str!("../../../tests/fixtures/comici_series.json");
    const EPISODES_FIXTURE: &str = include_str!("../../../tests/fixtures/comici_episodes.json");
    const HIKYOSAI_FIXTURE: &str =
        include_str!("../../../tests/fixtures/comici_episodes_hikyosai.json");
    const VOLUME_FIXTURE: &str =
        include_str!("../../../tests/fixtures/comici_episodes_volume.json");
    const RSS_FIXTURE: &str = include_str!("../../../tests/fixtures/comici_rss.xml");

    #[test]
    fn search_fixture_prefixes_hits_with_the_site() {
        let hits = parse_search(SEARCH_FIXTURE, "championcross.jp");
        assert_eq!(hits.len(), 3);
        assert!(
            hits.iter().all(|h| h.id.starts_with("championcross.jp/")),
            "every hit carries its catalog: {hits:?}"
        );
        assert!(
            hits.iter()
                .any(|h| h.title == "僕の心のヤバイやつ ラブコメディが始まらない"
                    && h.id == "championcross.jp/452b0e17e1bf3")
        );
    }

    #[test]
    fn search_drops_non_manga_and_malformed_entries() {
        let json = r#"{"searchResult":{"series":{"total":5,"series":[
            {"id":"aaaa1111","name":"Манга","category":"manga"},
            {"id":"bbbb2222","name":"Роман","category":"novel"},
            {"id":"bad/id","name":"Slash","category":"manga"},
            {"id":"cccc3333","name":"  ","category":"manga"},
            {"name":"No id","category":"manga"}
        ]}}}"#;
        let hits = parse_search(json, "bigcomics.jp");
        assert_eq!(
            hits.len(),
            1,
            "only the manga entry with a valid id remains"
        );
        assert_eq!(hits[0].id, "bigcomics.jp/aaaa1111");
        // Malformed and unrelated answers are empty, never a panic.
        assert!(parse_search("", "takecomic.jp").is_empty());
        assert!(parse_search("{}", "takecomic.jp").is_empty());
        assert!(parse_search("<html>", "takecomic.jp").is_empty());
        assert!(parse_search(r#"{"searchResult":{"series":{"series":[]}}}"#, "x").is_empty());
    }

    #[test]
    fn series_fixture_reports_the_episode_count() {
        assert_eq!(parse_series(SERIES_FIXTURE), Some(201));
        assert_eq!(parse_series(""), None);
        assert_eq!(parse_series(r#"{"series":{"summary":{}}}"#), None);
    }

    #[test]
    fn episodes_fixture_parses_titles_dates_and_numbers() {
        let rows = parse_episodes(EPISODES_FIXTURE);
        assert_eq!(rows.len(), 12);
        assert_eq!(rows[0].title, "Score.01　市川香菜と陣宗太");
        assert_eq!(
            rows[0].kind,
            ComiciTitle::Number(ShortTitle::Base(1)),
            "the Latin counter numbers the row"
        );
        assert_eq!(
            rows[0].release_date,
            official_meta::parse_date("2025-07-21"),
            "datePublished is epoch seconds"
        );
        assert_eq!(rows[11].kind, ComiciTitle::Number(ShortTitle::Base(12)));
        // Malformed windows answer empty, never a panic.
        assert!(parse_episodes("").is_empty());
        assert!(parse_episodes("{}").is_empty());
        assert!(parse_episodes(r#"{"series":{"episodes":[]}}"#).is_empty());
        assert!(parse_episodes(r#"{"series":{"episodes":[{"id":"x"}]}}"#).is_empty());
    }

    #[test]
    fn numbered_series_keeps_numbers_and_files_notices_as_bonus() {
        let rows = parse_episodes(HIKYOSAI_FIXTURE);
        assert_eq!(rows.len(), 20);
        let chapters = chapters_from_rows(rows);
        // 第23話..第35話 with 5 休載 notices and 2 番外編 rows in between.
        let mains: Vec<i32> = chapters
            .iter()
            .filter(|c| !c.is_bonus)
            .map(|c| c.number_x100)
            .collect();
        assert_eq!(mains, (23..=35).map(|n| n * 100).collect::<Vec<_>>());
        let bonuses: Vec<&ChapterMeta> = chapters.iter().filter(|c| c.is_bonus).collect();
        assert_eq!(bonuses.len(), 7, "5 notices + 2 side stories");
        assert!(bonuses.iter().all(|c| c.number_x100 == 0));
        assert!(
            bonuses
                .iter()
                .any(|c| c.title.as_deref().is_some_and(|t| t.contains("休載"))),
            "the hiatus notice is a bonus row: {bonuses:?}"
        );
        assert_eq!(chapter_title::main_count(&chapters), 35);
    }

    #[test]
    fn volume_labelled_series_numbers_by_position() {
        let rows = parse_episodes(VOLUME_FIXTURE);
        assert_eq!(rows.len(), 12);
        assert!(
            rows.iter().all(|r| r.kind == ComiciTitle::Plain),
            "volume-scoped labels carry no usable chapter number"
        );
        let chapters = chapters_from_rows(rows);
        let mains: Vec<i32> = chapters
            .iter()
            .filter(|c| !c.is_bonus)
            .map(|c| c.number_x100)
            .collect();
        assert_eq!(mains, (1..=12).map(|n| n * 100).collect::<Vec<_>>());
        assert_eq!(chapter_title::main_count(&chapters), 12);
    }

    #[test]
    fn rss_fixture_is_oldest_first_with_dates() {
        let rows = parse_rss(RSS_FIXTURE);
        assert_eq!(rows.len(), 5, "only <item> rows, not the channel title");
        assert_eq!(
            rows[0].title, "Score.23　陣くんちへお見舞い",
            "the feed is newest-first, the rows must come back oldest-first"
        );
        assert_eq!(rows[4].kind, ComiciTitle::Number(ShortTitle::Base(27)));
        assert_eq!(
            rows[0].release_date,
            official_meta::parse_date("2026-07-20"),
            "pubDate is RFC 2822"
        );
        assert_eq!(
            rows[4].release_date,
            official_meta::parse_date("2026-10-05")
        );
        // Malformed XML answers empty, never a panic.
        assert!(parse_rss("").is_empty());
        assert!(parse_rss("<rss><channel></channel></rss>").is_empty());
        assert!(parse_rss("<item><title></title></item>").is_empty());
    }

    #[test]
    fn rss_fallback_accepts_only_complete_feeds() {
        let row = Row {
            title: "第1話".to_string(),
            release_date: None,
            kind: ComiciTitle::Number(ShortTitle::Base(1)),
        };
        assert!(accept_rss(vec![row.clone()], 1).is_ok());
        let err = accept_rss(vec![row], 2).unwrap_err().to_string();
        assert!(err.contains("incomplete"), "{err}");
    }

    #[test]
    fn split_source_id_accepts_known_sites_only() {
        assert_eq!(
            split_source_id("championcross.jp/452b0e17e1bf3"),
            Some(("championcross.jp", "452b0e17e1bf3"))
        );
        assert_eq!(
            split_source_id("bigcomics.jp/48eef350d364a"),
            Some(("bigcomics.jp", "48eef350d364a"))
        );
        // Unknown host, path traversal, over-long or non-alphanumeric hashes.
        for id in [
            "evil.example/452b0e17e1bf3",
            "championcross.jp/../secret",
            "championcross.jp/",
            "championcross.jp",
            "championcross.jp/aa bb",
            "comicpash.jp/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(split_source_id(id), None, "{id}");
        }
    }
}
