//! Bilibili Manga (CN): mobile SSR `vike_pageContext` JSON + a quota-limited
//! guest search (5/IP per hour-ish; HTTP 200 with `code:503` when exhausted).
//!
//! Detail fixture (`tests/fixtures/bilibili_detail.html`):
//! `data.seasonData.{renewal_time, ep_list[]}`; `ep_list` is newest-first
//! (first row `short_title:"518"`, last `"1"`), each item carries `title`,
//! `short_title` and `pub_time` (`"2026-05-30 12:56:01"` — date part only).
//! Chapter numbers come from `short_title`, never from the row position:
//! a plain integer `>= 1` is a main chapter, everything else (`00` prologues,
//! `番外NN`, `49.5`, `第N坑`) is a bonus row with `number_x100 == 0`.
//!
//! Search is wrapped in a module-local 7-day cache + throttle (Decision #8):
//! `code 503` sets a 1.5 h cooldown and negative-caches the query, ≤4 live
//! calls per rolling hour; locks are never held across `.await`.
//!
//! Optional account mode: `BILIBILI_COOKIE` replaces the guest `buvid3=infoc`
//! Cookie header (guest quota does not apply to an account). If an account
//! search still comes back `code 503` (stale cookie / exhausted quota), one
//! guest retry is made and its outcome is absorbed as usual. The cookie value
//! is never logged.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::services::external::official_meta::{self, ChapterMeta, OfficialMeta};
use crate::services::official_types::OfficialHit;

const UA: &str = "Mozilla/5.0 (Linux; Android 13) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Mobile Safari/537.36";
const SEARCH_URL: &str =
    "https://manga.bilibili.com/twirp/comic.v1.Comic/Search?device=pc&platform=web&nov=27&a=810";

/// Decision #8 numbers.
const CACHE_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const COOLDOWN: Duration = Duration::from_secs(90 * 60);
const MAX_CALLS_PER_HOUR: usize = 4;

/// Guest Cookie header (required by search, value ignored). Used whenever
/// `BILIBILI_COOKIE` is unset, and for the single account->guest retry.
const GUEST_COOKIE: &str = "buvid3=infoc";

/// GET `https://manga.bilibili.com/m/detail/mc<id>` and parse the SSR page.
pub async fn fetch_meta(client: &reqwest::Client, comic_id: &str) -> anyhow::Result<OfficialMeta> {
    let url = format!("https://manga.bilibili.com/m/detail/mc{comic_id}");
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

/// Pure parser: `vike_pageContext` JSON → schedule from `renewal_time` and
/// chapter rows from `ep_list` (reversed to oldest-first). `count` is the
/// number of **main** chapters, not `len(ep_list)`. Absent/broken JSON
/// degrades to an empty meta.
pub fn parse_detail(html: &str) -> OfficialMeta {
    let json = page_context_json(html);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return OfficialMeta {
            source: "bilibili".to_string(),
            ..OfficialMeta::default()
        };
    };
    let season = value.pointer("/data/seasonData");
    let schedule = season
        .and_then(|s| s.get("renewal_time"))
        .and_then(|v| v.as_str())
        .and_then(official_meta::normalize_schedule);
    let chapters = season
        .and_then(|s| s.get("ep_list"))
        .and_then(|v| v.as_array())
        .map(|items| episode_rows(items))
        .unwrap_or_default();
    let main = main_count(&chapters);
    if !chapters.is_empty() {
        // Duplicate numbers are kept (as bonus rows), never dropped.
        tracing::debug!(
            source = "bilibili",
            rows = chapters.len(),
            main,
            bonus = chapters.len() - main,
            "ep_list parsed"
        );
    }
    OfficialMeta {
        source: "bilibili".to_string(),
        count: (main > 0).then_some(main as i32),
        chapters,
        update_schedule: schedule,
        next_update_at: None,
    }
}

/// Slice the `<script id="vike_pageContext" …>{…}</script>` payload down to
/// the opening `{`. Missing script -> "".
fn page_context_json(html: &str) -> &str {
    let seg = official_meta::between(html, "id=\"vike_pageContext\"", "</script>").unwrap_or("");
    match seg.find('{') {
        Some(i) => &seg[i..],
        None => "",
    }
}

/// One `ep_list` row reduced to the three fields we keep. `number` is the
/// parsed `short_title` (`None` for prologues and non-numeric specials).
struct EpisodeRow {
    title: Option<String>,
    release_date: Option<chrono::NaiveDate>,
    number: Option<i32>,
}

/// `short_title` -> `Some(n)` for a plain integer `>= 1`. Everything else —
/// `"00"`, `"0"`, `"番外01"`, `"49.5"`, `"第N坑"`, blanks, missing — is `None`
/// and becomes a bonus row with `number_x100 == 0`.
fn episode_number(item: &serde_json::Value) -> Option<i32> {
    let raw = item.get("short_title")?.as_str()?.trim();
    let n: i32 = raw.parse().ok()?;
    (n >= 1).then_some(n)
}

/// `ep_list` (newest-first) -> oldest-first `ChapterMeta` rows. Title falls
/// back to `short_title` when `title` is blank (some rows store `" "`).
///
/// Numbering comes from `short_title`, not from the row position. The first
/// sighting of a number is a main chapter; every repeat of an already-seen
/// number is a bonus duplicate. Duplicates are **kept** (as bonus rows), never
/// dropped, so the caller can decide how to store them.
fn episode_rows(items: &[serde_json::Value]) -> Vec<ChapterMeta> {
    let mut rows: Vec<EpisodeRow> = items
        .iter()
        .map(|item| {
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    item.get("short_title")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                });
            let date = item
                .get("pub_time")
                .and_then(|v| v.as_str())
                .and_then(|s| s.get(..10))
                .and_then(official_meta::parse_date);
            EpisodeRow {
                title,
                release_date: date,
                number: episode_number(item),
            }
        })
        .collect();
    rows.reverse();
    let mut seen: HashSet<i32> = HashSet::new();
    rows.into_iter()
        .map(|row| {
            let is_bonus = match row.number {
                Some(n) => !seen.insert(n),
                None => true,
            };
            ChapterMeta {
                number_x100: row.number.unwrap_or(0) * 100,
                title: row.title,
                release_date: row.release_date,
                is_bonus,
            }
        })
        .collect()
}

/// Number of main (non-bonus) chapters — the authoritative `count` for
/// `media_items.chapters`.
fn main_count(chapters: &[ChapterMeta]) -> usize {
    chapters.iter().filter(|c| !c.is_bonus).count()
}

// --- guest search: cache + throttle (Decision #8) --------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheEntry {
    hits: Vec<OfficialHit>,
    inserted: Instant,
    negative: bool,
}

#[derive(Debug, Default)]
struct ThrottleState {
    /// Timestamps of the live calls inside the rolling hour.
    calls: VecDeque<Instant>,
    /// Active `code 503` cooldown end.
    cooldown_until: Option<Instant>,
}

impl ThrottleState {
    /// Blocked while a cooldown runs or the rolling hour is full.
    fn blocked(&self, now: Instant) -> bool {
        if let Some(until) = self.cooldown_until
            && now < until
        {
            return true;
        }
        self.calls
            .iter()
            .filter(|c| now.duration_since(**c) < Duration::from_secs(3600))
            .count()
            >= MAX_CALLS_PER_HOUR
    }

    /// Record one live call (pruning the rolling window).
    fn record_call(&mut self, now: Instant) {
        while let Some(front) = self.calls.front()
            && now.duration_since(*front) >= Duration::from_secs(3600)
        {
            self.calls.pop_front();
        }
        self.calls.push_back(now);
    }

    /// `code 503` -> start the cooldown.
    fn start_cooldown(&mut self, now: Instant) {
        self.cooldown_until = Some(now + COOLDOWN);
    }
}

/// Classified search response body (HTTP is 200 even when rate-limited).
#[derive(Debug, PartialEq, Eq)]
enum SearchOutcome {
    Hits(Vec<OfficialHit>),
    /// `code 503` — quota exhausted; caller must cool down.
    Limited,
}

/// Inspect a raw search body: `code 503` -> `Limited`, otherwise parse
/// `data.list[]`. Pure; malformed bodies degrade to empty hits.
fn classify(body: &str) -> SearchOutcome {
    if official_meta::json_number_field(body, "code") == Some(503) {
        return SearchOutcome::Limited;
    }
    SearchOutcome::Hits(parse_search(body))
}

/// Cookie header for one search call: the account cookie verbatim when
/// configured, otherwise the guest placeholder. Pure (no env access).
fn search_cookie(account: Option<&str>) -> &str {
    account.unwrap_or(GUEST_COOKIE)
}

/// Thin wrapper over `BILIBILI_COOKIE`: a set, non-blank (after trim) value
/// enables account mode. The value itself must NEVER be logged.
fn account_cookie_env() -> Option<String> {
    let Ok(raw) = std::env::var("BILIBILI_COOKIE") else {
        return None;
    };
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Auto-fallback rule (Decision #8): retry as guest exactly once when the
/// *account* attempt came back quota-limited. Guest mode never retries.
/// Pure, so tests need no env mutation.
fn retry_as_guest(account_mode: bool, outcome: &SearchOutcome) -> bool {
    account_mode && matches!(outcome, SearchOutcome::Limited)
}

/// Fold one response into throttle + cache. Sync on purpose: testable without
/// a network, and both locks stay scoped (never held across `.await`).
fn absorb(
    outcome: &SearchOutcome,
    key: &str,
    cache: &mut HashMap<String, CacheEntry>,
    throttle: &mut ThrottleState,
    now: Instant,
) -> Vec<OfficialHit> {
    throttle.record_call(now);
    match outcome {
        SearchOutcome::Limited => {
            throttle.start_cooldown(now);
            cache.insert(
                key.to_string(),
                CacheEntry {
                    hits: Vec::new(),
                    inserted: now,
                    negative: true,
                },
            );
            Vec::new()
        }
        SearchOutcome::Hits(hits) => {
            cache.insert(
                key.to_string(),
                CacheEntry {
                    hits: hits.clone(),
                    inserted: now,
                    negative: false,
                },
            );
            hits.clone()
        }
    }
}

fn cache_lookup(
    cache: &HashMap<String, CacheEntry>,
    key: &str,
    now: Instant,
) -> Option<Vec<OfficialHit>> {
    let entry = cache.get(key)?;
    // Fresh entry -> serve; expired -> miss (caller refetches and overwrites).
    if now.duration_since(entry.inserted) < CACHE_TTL {
        Some(entry.hits.clone())
    } else {
        None
    }
}

fn cache_cell() -> &'static Mutex<HashMap<String, CacheEntry>> {
    static CELL: OnceLock<Mutex<HashMap<String, CacheEntry>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashMap::new()))
}

fn throttle_cell() -> &'static Mutex<ThrottleState> {
    static CELL: OnceLock<Mutex<ThrottleState>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(ThrottleState::default()))
}

/// One live search POST. The Cookie header is an explicit argument so the
/// account->guest fallback is just a second call with a different cookie.
/// No `Origin` header — a wrong one yields 400.
async fn post_search(
    client: &reqwest::Client,
    title: &str,
    cookie: &str,
) -> anyhow::Result<String> {
    let body = client
        .post(SEARCH_URL)
        .header(reqwest::header::USER_AGENT, UA)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::COOKIE, cookie)
        .body(serde_json::json!({ "key_word": title, "page_num": 1, "page_size": 10 }).to_string())
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body)
}

/// Cached + throttled wrapper wired into `HttpOfficialSearch` for `"bilibili"`.
pub async fn search_cached(
    client: &reqwest::Client,
    title: &str,
) -> anyhow::Result<Vec<OfficialHit>> {
    let key = title.to_string();
    let now = Instant::now();

    // 1) Fresh cache entry (hit AND negative) short-circuits.
    let cached = {
        let guard = cache_cell().lock().expect("search cache poisoned");
        cache_lookup(&guard, &key, now)
    };
    if let Some(hits) = cached {
        return Ok(hits);
    }

    // 2) Cooldown / hourly budget -> silent no-op (never surfaces as an error).
    {
        let guard = throttle_cell().lock().expect("throttle poisoned");
        if guard.blocked(now) {
            return Ok(Vec::new());
        }
    }

    // 3) Live call. Account mode sends `BILIBILI_COOKIE`, guest mode sends
    // `buvid3=infoc`. The response body (not the status) carries `code 503`.
    let account = account_cookie_env();
    let mut outcome =
        classify(&post_search(client, title, search_cookie(account.as_deref())).await?);

    // 3b) Stale account cookie / exhausted quota -> ONE guest retry. Locks are
    // not held here (no await while held); the guest outcome is absorbed
    // below exactly like a first-try result.
    if retry_as_guest(account.is_some(), &outcome) {
        tracing::warn!("bilibili: search limit with account cookie, falling back to guest");
        outcome = classify(&post_search(client, title, GUEST_COOKIE).await?);
    }

    // 4) Fold the outcome in; locks stay scoped, no await while held.
    let mut cache = cache_cell().lock().expect("search cache poisoned");
    let mut throttle = throttle_cell().lock().expect("throttle poisoned");
    Ok(absorb(&outcome, &key, &mut cache, &mut throttle, now))
}

/// `data.list[]` -> `{ id (mc id), real_title }` (`title` carries `<em>` and
/// is deliberately ignored). Malformed bodies degrade to an empty vector.
pub fn parse_search(json: &str) -> Vec<OfficialHit> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(list) = value.pointer("/data/list").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut hits: Vec<OfficialHit> = Vec::new();
    for item in list {
        let Some(id) = item.get("id").and_then(|v| v.as_u64()).or_else(|| {
            item.get("id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
        }) else {
            continue;
        };
        let Some(title) = item
            .get("real_title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let id = id.to_string();
        if !hits.iter().any(|h| h.id == id) {
            hits.push(OfficialHit {
                id,
                title: title.to_string(),
            });
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_fixture_schedule_and_chapters() {
        let html = include_str!("../../../tests/fixtures/bilibili_detail.html");
        let m = parse_detail(html);
        assert_eq!(m.source, "bilibili");
        // renewal_time "不定时更新" is a real free-text schedule.
        assert_eq!(m.update_schedule.as_deref(), Some("不定时更新"));
        // 596 ep_list rows, reversed to oldest-first. Every row is stored,
        // so len == 596 ...
        assert_eq!(m.chapters.len(), 596);
        // ... but `count` is the number of MAIN chapters only.
        // Пересчитано по факту фикстуры: 596 rows = 399 distinct numeric
        // short_titles >= 1 (main) + 197 bonus rows (番外NN / `.5` chapters /
        // `第N坑` placeholders / notices). The fixture has no duplicate
        // numeric short_title, so all 197 bonus rows are non-numeric specials.
        assert_eq!(main_count(&m.chapters), 399);
        assert_eq!(m.chapters.iter().filter(|c| !c.is_bonus).count(), 399);
        assert_eq!(m.chapters.iter().filter(|c| c.is_bonus).count(), 197);
        assert_eq!(m.count, Some(399));
        // Oldest row is `short_title:"1"` -> numbering from short_title, not
        // from the row position (which would also have been 100).
        assert_eq!(m.chapters[0].number_x100, 100);
        assert!(!m.chapters[0].is_bonus);
        // Oldest row (ord 1) is dated 2018-11-13.
        assert_eq!(
            m.chapters[0].release_date,
            official_meta::parse_date("2018-11-13")
        );
        // Newest row (`short_title:"518"`) ends the sequence -> 51800, NOT the
        // positional 59600 the old parser produced.
        assert_eq!(m.chapters[595].number_x100, 51800);
        assert_ne!(m.chapters[595].number_x100, 59600);
        assert!(!m.chapters[595].is_bonus);
        assert_eq!(
            m.chapters[595].release_date,
            official_meta::parse_date("2026-05-30")
        );
        // Blank `title` (" ") falls back to `short_title`.
        assert!(
            m.chapters[0]
                .title
                .as_deref()
                .is_some_and(|t| !t.is_empty())
        );
        assert!(m.next_update_at.is_none());
    }

    /// Synthetic `ep_list` (newest-first) exercising the classification:
    /// `1`, `00`, `2`, `2` (duplicate), `番外01`, `3`.
    fn mixed_ep_list_html() -> &'static str {
        r#"<script id="vike_pageContext" type="application/json">{"data":{"seasonData":{
            "renewal_time":"每周五更新",
            "ep_list":[
                {"title":"第三话","short_title":"3","pub_time":"2018-03-02 10:00:00"},
                {"title":"番外01","short_title":"番外01","pub_time":"2018-02-20 10:00:00"},
                {"title":"第二话重发","short_title":"2","pub_time":"2018-02-11 10:00:00"},
                {"title":"第二话","short_title":"2","pub_time":"2018-02-10 10:00:00"},
                {"title":"","short_title":"00","pub_time":"2018-01-20 10:00:00"},
                {"title":"第一话","short_title":"1","pub_time":"2018-01-13 10:00:00"}
            ]}}}</script>"#
    }

    #[test]
    fn numbering_and_bonus_classification() {
        let m = parse_detail(mixed_ep_list_html());
        assert_eq!(m.update_schedule.as_deref(), Some("每周五更新"));
        // All rows are kept, in oldest-first order.
        assert_eq!(m.chapters.len(), 6);
        // Numbering comes from short_title; prologues/specials get 0.
        let numbers: Vec<i32> = m.chapters.iter().map(|c| c.number_x100).collect();
        assert_eq!(numbers, vec![100, 0, 200, 200, 0, 300]);
        // First sighting of a number is main, every repeat is a bonus duplicate.
        let bonus: Vec<bool> = m.chapters.iter().map(|c| c.is_bonus).collect();
        assert_eq!(bonus, vec![false, true, false, true, true, false]);
        // `count` counts main chapters only (3), never len (6).
        assert_eq!(main_count(&m.chapters), 3);
        assert_eq!(m.count, Some(3));
        assert_ne!(m.count, Some(m.chapters.len() as i32));
        // The duplicate row keeps its own title, so it is not silently dropped.
        assert_eq!(m.chapters[3].title.as_deref(), Some("第二话重发"));
        // The `00` prologue falls back to `short_title` for its title.
        assert_eq!(m.chapters[1].title.as_deref(), Some("00"));
        // Rows stay oldest-first (ep_list is reversed on the way in).
        let dates: Vec<_> = m.chapters.iter().filter_map(|c| c.release_date).collect();
        assert_eq!(
            dates.first().copied(),
            official_meta::parse_date("2018-01-13")
        );
        assert_eq!(
            dates.last().copied(),
            official_meta::parse_date("2018-03-02")
        );
    }

    #[test]
    fn episode_number_rejects_non_numeric_and_zero() {
        let item = |raw: &str| serde_json::json!({ "short_title": raw });
        assert_eq!(episode_number(&item("1")), Some(1));
        assert_eq!(episode_number(&item("518")), Some(518));
        assert_eq!(episode_number(&item(" 42 ")), Some(42));
        // `00`, `0`, negatives and floats are bonuses.
        assert_eq!(episode_number(&item("00")), None);
        assert_eq!(episode_number(&item("0")), None);
        assert_eq!(episode_number(&item("-3")), None);
        assert_eq!(episode_number(&item("49.5")), None);
        assert_eq!(episode_number(&item("番外01")), None);
        assert_eq!(episode_number(&item("第500坑")), None);
        assert_eq!(episode_number(&item("")), None);
        // A missing / non-string short_title degrades to a bonus row.
        assert_eq!(episode_number(&serde_json::json!({})), None);
        assert_eq!(
            episode_number(&serde_json::json!({ "short_title": 7 })),
            None
        );
    }

    #[test]
    fn main_count_zero_when_all_rows_are_bonus() {
        let items = vec![
            serde_json::json!({ "short_title": "番外01" }),
            serde_json::json!({ "short_title": "00" }),
        ];
        let chapters = episode_rows(&items);
        assert_eq!(chapters.len(), 2);
        assert_eq!(main_count(&chapters), 0);
        assert!(chapters.iter().all(|c| c.is_bonus && c.number_x100 == 0));
    }

    #[test]
    fn empty_input_degrades() {
        let m = parse_detail("");
        assert_eq!(m.source, "bilibili");
        assert!(m.chapters.is_empty() && m.count.is_none());
        assert!(m.update_schedule.is_none());
        assert!(parse_search("").is_empty());
        assert!(parse_search("<html></html>").is_empty());
    }

    #[test]
    fn search_fixture_503_is_limited() {
        // The captured guest-quota body: HTTP 200 + code 503.
        let body = include_str!("../../../tests/fixtures/bilibili_search.json");
        assert_eq!(classify(body), SearchOutcome::Limited);
        // Malformed / unrelated bodies never look rate-limited.
        assert_eq!(classify("{broken"), SearchOutcome::Hits(Vec::new()));
    }

    #[test]
    fn search_positive_inline_json() {
        let body = r#"{"code":0,"data":{"list":[
            {"id":25506,"real_title":"ワンピース","title":"<em class=\"keyword\">ワン</em>ピース"},
            {"id":"25507","real_title":"ナルト"},
            {"id":25506,"real_title":"dup"},
            {"id":9,"real_title":""}
        ]}}"#;
        let hits = parse_search(body);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "25506");
        assert_eq!(hits[0].title, "ワンピース");
        assert_eq!(hits[1].id, "25507");
    }

    #[test]
    fn cookie_choice_account_vs_guest() {
        // No env value -> guest placeholder (current behaviour preserved).
        assert_eq!(search_cookie(None), "buvid3=infoc");
        assert_eq!(search_cookie(None), GUEST_COOKIE);
        // Account value is passed through verbatim (no trimming/mangling).
        let account = "SESSDATA=abc; bili_jct=def; buvid3=real";
        assert_eq!(search_cookie(Some(account)), account);
    }

    #[test]
    fn guest_retry_only_for_account_limit() {
        let limited = SearchOutcome::Limited;
        let hits = SearchOutcome::Hits(Vec::new());
        // Account + quota exhausted -> exactly one guest retry.
        assert!(retry_as_guest(true, &limited));
        // Guest mode never retries, even on 503 (cooldown absorbs it).
        assert!(!retry_as_guest(false, &limited));
        // Successful account responses never retry.
        assert!(!retry_as_guest(true, &hits));
        assert!(!retry_as_guest(false, &hits));
    }

    #[test]
    fn limited_body_cools_down_and_negative_caches() {
        let body = include_str!("../../../tests/fixtures/bilibili_search.json");
        let outcome = classify(body);
        let mut cache = HashMap::new();
        let mut throttle = ThrottleState::default();
        let now = Instant::now();
        // 503 -> no hits, cooldown started, query negatively cached.
        let hits = absorb(&outcome, "ワンピース", &mut cache, &mut throttle, now);
        assert!(hits.is_empty());
        assert!(throttle.blocked(now + Duration::from_secs(60)));
        assert!(throttle.blocked(now + COOLDOWN - Duration::from_secs(1)));
        let cached = cache_lookup(&cache, "ワンピース", now).expect("negative entry");
        assert!(cached.is_empty());
        // After the cooldown the throttle opens again.
        assert!(!throttle.blocked(now + COOLDOWN + Duration::from_secs(1)));
    }

    #[test]
    fn hourly_budget_blocks_fifth_call() {
        let mut throttle = ThrottleState::default();
        let now = Instant::now();
        for i in 0..MAX_CALLS_PER_HOUR {
            assert!(!throttle.blocked(now), "call {i} must be allowed");
            throttle.record_call(now);
        }
        assert!(throttle.blocked(now), "fifth call in the hour is blocked");
        // An hour later the old calls fall out of the rolling window:
        // `record_call` prunes them and the budget opens up again.
        let later = now + Duration::from_secs(3601);
        assert!(!throttle.blocked(later), "window rolled over");
        throttle.record_call(later);
        assert!(!throttle.blocked(later));
    }

    #[test]
    fn positive_cache_is_served_within_ttl() {
        let mut cache = HashMap::new();
        let now = Instant::now();
        let hits = vec![OfficialHit {
            id: "1".into(),
            title: "T".into(),
        }];
        cache.insert(
            "q".to_string(),
            CacheEntry {
                hits: hits.clone(),
                inserted: now,
                negative: false,
            },
        );
        assert_eq!(
            cache_lookup(&cache, "q", now + Duration::from_secs(10)),
            Some(hits)
        );
        // Expired -> miss.
        assert_eq!(
            cache_lookup(&cache, "q", now + CACHE_TTL + Duration::from_secs(1)),
            None
        );
    }
}
