//! Toomics official-source search + chapter metadata (English catalogue).
//!
//! Search is a two-step handshake the site itself performs: `/webtoon/
//! get_search_data_v2?keyword=&page=` answers with an Elasticsearch query
//! template plus the (base64) endpoint, credentials and index, and the client
//! POSTs that query to the search host. The hits carry the English title,
//! authors, genres, tags, synopsis and the cover.
//!
//! The episode list is server-rendered on `/en/webtoon/episode/toon/{id}`:
//! `li.normal_ep` rows carry the site's episode number (`span.num`), the
//! episode title and its date. The page has no pager — it always lists the
//! whole run.
//!
//! All failures are non-fatal: the caller treats an empty result (or an `Err`)
//! as "no official match".

use chrono::NaiveDate;

use crate::services::external::chapter_title;
use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::{HitExtra, OfficialHit};

const SEARCH_TEMPLATE: &str = "https://toomics.com/webtoon/get_search_data_v2";
const SERIES_BASE: &str = "https://toomics.com/en/webtoon/episode/toon";

/// Browser-like User-Agent; the site rejects obvious bot clients.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Defensive cap on one search answer.
const SEARCH_MAX_HITS: usize = 50;
/// Rows read from one series page (the site has no pager; a hostile answer
/// must not blow up memory).
const EPISODE_CAP: usize = 5000;
/// Longest stored episode title.
const MAX_TITLE_CHARS: usize = 512;
/// Longest stored synopsis taken from a search answer.
const MAX_DESCRIPTION_CHARS: usize = 2000;
/// How far a tag search may reach from its anchor.
const WINDOW: usize = 4096;

/// Empty meta tagged with this source (non-fatal "nothing to report").
fn empty_meta() -> OfficialMeta {
    OfficialMeta {
        source: "toomics".to_string(),
        ..OfficialMeta::default()
    }
}

/// The decoded search handshake: where and how to run the query.
#[derive(Debug, Clone, PartialEq)]
pub struct EsRequest {
    pub endpoint: String,
    pub index: String,
    pub credentials: String,
    pub query: serde_json::Value,
}

/// Pure parser over the search template: the base64 fields decoded and the
/// query JSON parsed. `None` when the answer is malformed.
pub fn parse_search_request(json: &str) -> Option<EsRequest> {
    let value = serde_json::from_str::<serde_json::Value>(json).ok()?;
    let field = |key: &str| -> Option<String> {
        let raw = value.get(key)?.as_str()?;
        String::from_utf8(base64_decode(raw)?).ok()
    };
    let query = serde_json::from_str::<serde_json::Value>(value.get("sEsQuery")?.as_str()?).ok()?;
    Some(EsRequest {
        endpoint: field("sEsEndpoint")?,
        index: field("sEsIndex")?,
        credentials: field("sEsCredentials")?,
        query,
    })
}

/// Pure parser over the Elasticsearch answer: `hits.hits[]._source`.
pub fn parse_es_hits(json: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(items) = value.pointer("/hits/hits").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for item in items.iter().take(SEARCH_MAX_HITS) {
        let Some(source) = item.get("_source") else {
            continue;
        };
        let Some(id) = source.get("toon_idx").and_then(|v| v.as_i64()) else {
            continue;
        };
        let Some(title) = source
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        if hits.iter().any(|h| h.id == id.to_string()) {
            continue;
        }
        // Rich search-answer fields: cover (adult variant for adult rows),
        // authors, genres, tags, synopsis.
        let adult = source
            .get("adult_yn")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == "Y");
        let poster_url = source
            .get("image")
            .and_then(|image| {
                let key = if adult {
                    "adult_thumbnail"
                } else {
                    "general_thumbnail"
                };
                image.get(key).and_then(|v| v.as_str())
            })
            .map(str::to_string);
        let author = source.get("author").and_then(|author| {
            let writer = author.get("writer").and_then(|v| v.as_str());
            let painter = author.get("painter").and_then(|v| v.as_str());
            match (writer, painter) {
                (Some(w), Some(p)) if !w.is_empty() && !p.is_empty() && w != p => {
                    Some(format!("{w}, {p}"))
                }
                (Some(w), _) if !w.is_empty() => Some(w.to_string()),
                (_, Some(p)) if !p.is_empty() => Some(p.to_string()),
                _ => None,
            }
        });
        let genres = source
            .get("genre")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let description = source
            .get("summary")
            .and_then(|v| v.as_str())
            .map(|s| s.replace("<br>", " ").replace("<br/>", " "))
            .map(|s| official_meta::strip_tags(&s))
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(MAX_DESCRIPTION_CHARS).collect());
        hits.push(OfficialHit::with_extra(
            id.to_string(),
            title.chars().take(MAX_TITLE_CHARS).collect::<String>(),
            HitExtra {
                poster_url,
                description,
                author,
                genres,
                ..HitExtra::default()
            },
        ));
    }
    hits
}

/// Search every platform catalogue via the two-step handshake.
pub async fn search(client: &reqwest::Client, title: &str) -> anyhow::Result<Vec<OfficialHit>> {
    let mut url = url::Url::parse(SEARCH_TEMPLATE)?;
    url.query_pairs_mut()
        .append_pair("keyword", title)
        .append_pair("page", "1");
    let body = get_text(client, url).await?;
    let Some(request) = parse_search_request(&body) else {
        return Ok(Vec::new());
    };
    let es_body = run_es_search(client, &request).await?;
    Ok(parse_es_hits(&es_body))
}

/// POST the handed-out query to the handed-out search host.
async fn run_es_search(client: &reqwest::Client, request: &EsRequest) -> anyhow::Result<String> {
    let url = format!(
        "{}/{}",
        request.endpoint.trim_end_matches('/'),
        request.index
    );
    let (user, password) = request
        .credentials
        .split_once(':')
        .unwrap_or((request.credentials.as_str(), ""));
    let response = client
        .post(&url)
        .basic_auth(user, Some(password))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, UA)
        .body(serde_json::to_vec(&request.query)?)
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

/// One episode row of the server-rendered list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToomicsEpisode {
    pub number: i32,
    pub title: String,
    pub release_date: Option<NaiveDate>,
}

/// Pure parser over the series page: `li.normal_ep` rows with their
/// `span.num`, `strong.line-3` title and `time datetime` date.
pub fn parse_episodes(html: &str) -> Vec<ToomicsEpisode> {
    let mut rows = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = html[cursor..].find("class=\"normal_ep") {
        let at = cursor + relative;
        let hi = official_meta::floor_boundary(html, at + WINDOW * 4);
        let Some(end_rel) = html[at..hi].find("</li>") else {
            break;
        };
        let item = &html[at..at + end_rel];
        cursor = at + end_rel + "</li>".len();
        let Some(number) = official_meta::between(item, "<span class=\"num\">", "</span>")
            .and_then(|raw| raw.trim().parse::<i32>().ok())
        else {
            continue;
        };
        if number < 1 {
            continue;
        }
        let Some(title) = official_meta::between(item, "<strong class=\"line-3\">", "</strong>")
            .map(official_meta::strip_tags)
            .filter(|t| !t.is_empty())
        else {
            continue;
        };
        let release_date =
            official_meta::between(item, "<time datetime=\"", "\"").and_then(parse_episode_date);
        rows.push(ToomicsEpisode {
            number,
            title: title.chars().take(MAX_TITLE_CHARS).collect(),
            release_date,
        });
        if rows.len() >= EPISODE_CAP {
            break;
        }
    }
    rows
}

/// `Oct 25, 2019` -> the date.
fn parse_episode_date(raw: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(raw.trim(), "%b %d, %Y").ok()
}

/// Number the rows: the site's own episode number, side-content markers
/// (`Epilogue`, `Special`, …) into bonuses.
pub fn chapters_from_rows(rows: Vec<ToomicsEpisode>) -> Vec<ChapterMeta> {
    rows.into_iter()
        .map(|row| {
            let is_bonus = is_bonus_title(&row.title);
            ChapterMeta {
                number_x100: if is_bonus { 0 } else { row.number * 100 },
                title: Some(row.title),
                release_date: row.release_date,
                is_bonus,
                label: None,
            }
        })
        .collect()
}

/// English side-content prefixes (the tappytoon marker set): a word-boundary
/// match at the start only, so `Episode 5: The Special One` stays a chapter.
/// A bracketed completion notice (`[End]`) is side content too.
fn is_bonus_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    let bare = lower.trim_start_matches('[').trim_end_matches(']').trim();
    if bare == "end" || bare == "the end" {
        return true;
    }
    [
        "epilogue",
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
    let rows = parse_episodes(&html);
    if rows.is_empty() {
        return Ok(empty_meta());
    }
    if rows.len() >= EPISODE_CAP {
        // The cap cut the list off; `apply_meta`'s orphan cleanup would drop
        // rows the walk never saw, so refuse the truncated list.
        anyhow::bail!("toomics: episode row cap hit, refusing a truncated list");
    }
    let chapters = chapters_from_rows(rows);
    let main = chapter_title::main_count(&chapters);
    Ok(OfficialMeta {
        source: "toomics".to_string(),
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
        .send()
        .await?
        .error_for_status()?;
    official_meta::body_capped(response, official_meta::MAX_DETAIL_BYTES).await
}

/// Standard-alphabet base64 (`=` padding allowed) — the handshake fields are
/// base64 strings; a malformed value yields `None`.
fn base64_decode(raw: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(raw.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in raw.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERIES_FIXTURE: &str = include_str!("../../../tests/fixtures/toomics_series.html");

    #[test]
    fn base64_round_trips_known_values() {
        assert_eq!(
            base64_decode("aHR0cHM6Ly9vc2gudG9vbWljcy5jb20=").as_deref(),
            Some(b"https://osh.toomics.com".as_slice())
        );
        assert_eq!(
            base64_decode("dG9vbnNfcm86c2VjcmV0").as_deref(),
            Some(b"toons_ro:secret".as_slice())
        );
        assert_eq!(
            base64_decode("NTE2Mg==").as_deref(),
            Some(b"5162".as_slice())
        );
        assert_eq!(base64_decode("!!!"), None);
    }

    #[test]
    fn search_request_decodes_the_handshake() {
        let json = r#"{"sEsQuery":"{\"query\":{\"match_all\":{}}}","sEsEndpoint":"aHR0cHM6Ly9vc2gudG9vbWljcy5jb20=","sEsCredentials":"dG9vbnNfcm86c2VjcmV0","sEsIndex":"dG9vbnNfZW5fcHJvZC9fc2VhcmNo"}"#;
        let request = parse_search_request(json).expect("handshake parses");
        assert_eq!(request.endpoint, "https://osh.toomics.com");
        assert_eq!(request.index, "toons_en_prod/_search");
        assert_eq!(request.credentials, "toons_ro:secret");
        assert_eq!(request.query["query"]["match_all"], serde_json::json!({}));
        // Malformed answers are None, never a panic.
        assert_eq!(parse_search_request("{}"), None);
        assert_eq!(parse_search_request("not json"), None);
    }

    #[test]
    fn es_hits_parse_into_rich_hits() {
        let json = r#"{"hits":{"hits":[
            {"_source":{"toon_idx":5162,"title":"You're No Good","adult_yn":"Y",
             "image":{"adult_thumbnail":"https://thumb-g2.toomics.com/a.jpg"},
             "author":{"writer":"AREM","painter":"AREM"},
             "genre":["Drama","Romance"],"tags":[{"tag":"College"}],
             "summary":"Why does Alfred<br>insist on living as an outcast?"}},
            {"_source":{"toon_idx":1,"title":"General Title","adult_yn":"N",
             "image":{"general_thumbnail":"https://thumb-g2.toomics.com/g.jpg"}}}
        ]}}"#;
        let hits = parse_es_hits(json);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "5162");
        assert_eq!(
            hits[0].extra.poster_url.as_deref(),
            Some("https://thumb-g2.toomics.com/a.jpg")
        );
        assert_eq!(hits[0].extra.author.as_deref(), Some("AREM"));
        assert_eq!(
            hits[0].extra.genres,
            vec!["Drama".to_string(), "Romance".to_string()]
        );
        assert!(
            hits[0]
                .extra
                .description
                .as_deref()
                .is_some_and(|d| d.contains("Alfred insist"))
        );
        assert_eq!(
            hits[1].extra.poster_url.as_deref(),
            Some("https://thumb-g2.toomics.com/g.jpg")
        );
        // Malformed answers are empty, never a panic.
        assert!(parse_es_hits("").is_empty());
        assert!(parse_es_hits("{}").is_empty());
    }

    #[test]
    fn episode_fixture_parses_numbers_titles_and_dates() {
        // The fixture keeps the first ten and the last three rows of the
        // 104-episode run.
        let rows = parse_episodes(SERIES_FIXTURE);
        assert_eq!(rows.len(), 13);
        assert_eq!(rows[0].number, 1);
        assert_eq!(rows[0].title, "Season 1");
        assert_eq!(
            rows[0].release_date,
            official_meta::parse_date("2019-10-25"),
            "the datetime attribute is `Oct 25, 2019`"
        );
        assert_eq!(rows.last().map(|r| r.number), Some(103));
        // Malformed pages are empty, never a panic.
        assert!(parse_episodes("").is_empty());
        assert!(parse_episodes("<html></html>").is_empty());
    }

    #[test]
    fn bonus_markers_are_english_word_boundaries() {
        let rows = vec![
            ToomicsEpisode {
                number: 1,
                title: "Season 1".to_string(),
                release_date: None,
            },
            ToomicsEpisode {
                number: 2,
                title: "Episode 5: The Special One".to_string(),
                release_date: None,
            },
            ToomicsEpisode {
                number: 3,
                title: "Epilogue".to_string(),
                release_date: None,
            },
        ];
        let chapters = chapters_from_rows(rows);
        assert_eq!(chapters[0].number_x100, 100);
        assert!(!chapters[0].is_bonus);
        assert_eq!(
            chapters[1].number_x100, 200,
            "`Special` mid-title stays a chapter"
        );
        assert!(!chapters[1].is_bonus);
        assert!(chapters[2].is_bonus);
        assert_eq!(chapters[2].number_x100, 0);
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
