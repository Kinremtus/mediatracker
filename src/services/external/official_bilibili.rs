//! Bilibili Manga (CN): mobile SSR `vike_pageContext` JSON + a quota-limited
//! guest search (5/IP per hour-ish; HTTP 200 with `code:503` when exhausted).
//!
//! Detail fixture (`tests/fixtures/bilibili_detail.html`):
//! `data.seasonData.{renewal_time, ep_list[]}`; `ep_list` is newest-first
//! (first row `short_title:"518"`, last `"1"`), each item carries `title`,
//! `short_title` and `pub_time` (`"2026-05-30 12:56:01"` — date part only).
//! Chapter numbers come from `short_title`, never from the row position.
//! Recognized forms: plain integers and `第N话`; split releases `N-M`
//! (`479-1`, `469—2`) and bracketed `上/下` markers -> parts of chapter N
//! stored as `N*100 + part`; concatenated ranges (`"270273"` == `"270-273"`)
//! map to their first chapter. Everything else (`00` prologues, `番外NN`,
//! `49.5`, `第N坑`, notices) is a bonus row with `number_x100 == 0`.
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

/// One `ep_list` row reduced to the fields we keep.
struct EpisodeRow {
    title: Option<String>,
    release_date: Option<chrono::NaiveDate>,
    kind: ShortTitle,
}

/// How one `short_title` maps onto the chapter sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortTitle {
    /// A plain chapter number (`"281"`, `"第533话"`).
    Base(i32),
    /// One split release of a chapter (`"479-1"`, `"533（下）"`): `base` plus a
    /// `part` in `1..=99`. Parts become separate tickable rows
    /// (`base * 100 + part`) but count as the single chapter `base`.
    Part(i32, i32),
    /// Fractional special (`"49.5"` -> 4950) — always a bonus row.
    Fractional(i32),
    /// Prologues, notices and placeholders — bonus rows keyed 0.
    Unnumbered,
}

/// Strip one trailing part marker `（上）/（下）/(上)/(下)/【上】/【下】`:
/// `Some((head, part))`, part 1 for 上 and 2 for 下.
fn split_part_marker(s: &str) -> Option<(&str, i32)> {
    for (suffix, part) in [
        ("（上）", 1),
        ("（下）", 2),
        ("(上)", 1),
        ("(下)", 2),
        ("【上】", 1),
        ("【下】", 2),
    ] {
        if let Some(head) = s.strip_suffix(suffix) {
            return Some((head.trim_end(), part));
        }
    }
    None
}

/// `第N话` / `第N話` -> `Some(N)` for `N >= 1`.
fn parse_cn_chapter(s: &str) -> Option<i32> {
    let rest = s.strip_prefix('第')?;
    let digits = rest
        .strip_suffix('话')
        .or_else(|| rest.strip_suffix('話'))?;
    let n: i32 = digits.trim().parse().ok()?;
    (n >= 1).then_some(n)
}

/// `N-M` split-release marker (`"479-1"`, `"469—2"`, `"414－2"`) ->
/// `Some(Part(N, M))` for `M` in `1..=99`. A forward pair (`"421-422"`,
/// `M > N`) is a merged range of chapters and maps to its first chapter.
fn classify_dash(s: &str) -> Option<ShortTitle> {
    let normalized = s.replace(['—', '－', '–'], "-");
    let (base, part) = normalized.split_once('-')?;
    let base: i32 = base.trim().parse().ok()?;
    let part: i32 = part.trim().parse().ok()?;
    if base < 1 {
        return Some(ShortTitle::Unnumbered);
    }
    if part > base {
        return Some(ShortTitle::Base(base));
    }
    if (1..=99).contains(&part) {
        return Some(ShortTitle::Part(base, part));
    }
    Some(ShortTitle::Unnumbered)
}

/// Split a concatenated range (`"270273"` == `"270-273"`): both halves parse,
/// the second is ahead of the first by a small step. Only an even-length 6+
/// digit string can be two numbers of equal width — real chapter numbers are
/// shorter and odd lengths have no clean split.
fn split_concatenated_range(s: &str) -> Option<i32> {
    if s.len() < 6 || !s.len().is_multiple_of(2) {
        return None;
    }
    let mid = s.len() / 2;
    let first: i32 = s[..mid].parse().ok()?;
    let last: i32 = s[mid..].parse().ok()?;
    (first >= 1 && last > first && last - first <= 20).then_some(first)
}

/// Classify one `short_title`. Bracketed `上/下` markers are stripped first
/// (iteratively: a hostile page could chain many, and the marker nearest the
/// number wins), then `第N话`, plain integers (incl. concatenated ranges),
/// `N-M` parts/ranges, and finally fractional specials.
fn classify_short_title(raw: &str) -> ShortTitle {
    let mut s = raw.trim();
    let mut part: Option<i32> = None;
    while let Some((head, p)) = split_part_marker(s) {
        part = Some(p);
        s = head;
    }
    // A marker only refines a plain number; if the number already carries its
    // own part (`479-1（下）`) that part wins.
    match (classify_number(s), part) {
        (ShortTitle::Base(n), Some(p)) => ShortTitle::Part(n, p),
        (other, _) => other,
    }
}

/// Classify a `short_title` with every bracketed marker already stripped.
fn classify_number(s: &str) -> ShortTitle {
    if let Some(n) = parse_cn_chapter(s) {
        return ShortTitle::Base(n);
    }
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        let Ok(n) = s.parse::<i32>() else {
            return ShortTitle::Unnumbered;
        };
        if n < 1 {
            return ShortTitle::Unnumbered;
        }
        return ShortTitle::Base(split_concatenated_range(s).unwrap_or(n));
    }
    if let Some(kind) = classify_dash(s) {
        return kind;
    }
    match crate::services::chapters::parse_chapter(s) {
        Some(x) if x > 0 => ShortTitle::Fractional(x),
        _ => ShortTitle::Unnumbered,
    }
}

/// `ep_list` (newest-first) -> oldest-first `ChapterMeta` rows. Title falls
/// back to `short_title` when `title` is blank (some rows store `" "`).
///
/// Numbering comes from `short_title`, not from the row position. A split part
/// is its own tickable row (`479-1` -> 47901, `第533话（下）` -> 53302); the
/// first sighting of a key is a main chapter, every repeat a bonus duplicate.
/// Duplicates are **kept** (as bonus rows), never dropped.
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
            let kind = item
                .get("short_title")
                .and_then(|v| v.as_str())
                .map(classify_short_title)
                .unwrap_or(ShortTitle::Unnumbered);
            EpisodeRow {
                title,
                release_date: date,
                kind,
            }
        })
        .collect();
    rows.reverse();
    let mut seen: HashSet<i32> = HashSet::new();
    rows.into_iter()
        .map(|row| {
            let (number_x100, is_bonus, label) = match row.kind {
                ShortTitle::Base(n) => match n.checked_mul(100) {
                    Some(key) => (key, !seen.insert(key), None),
                    None => (0, true, None),
                },
                ShortTitle::Part(n, part) => {
                    match n.checked_mul(100).and_then(|k| k.checked_add(part)) {
                        // Parts read `479-1`; the label also rides on a
                        // duplicate part, which becomes a bonus row.
                        Some(key) => (key, !seen.insert(key), Some(format!("{n}-{part}"))),
                        None => (0, true, None),
                    }
                }
                // Fractional specials keep their original x100; everything else
                // is 0 (unnumbered). `apply_meta` turns this into the display
                // label; both are always bonus rows.
                ShortTitle::Fractional(x) => (x, true, None),
                ShortTitle::Unnumbered => (0, true, None),
            };
            ChapterMeta {
                number_x100,
                title: row.title,
                release_date: row.release_date,
                is_bonus,
                label,
            }
        })
        .collect()
}

/// Number of main chapters — the highest main base number
/// (`MAX(chapter_number) / 100`): split parts (`479-1`, `479-2`) count as the
/// single chapter 479. Feeds `media_items.chapters` and the drawer badge.
fn main_count(chapters: &[ChapterMeta]) -> usize {
    chapters
        .iter()
        .filter(|c| !c.is_bonus)
        .map(|c| c.number_x100 / 100)
        .max()
        .unwrap_or(0) as usize
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
        // ... but `count` is the highest main base number (`MAX / 100`), not
        // the row count: split parts and merged ranges collapse to one chapter.
        // Пересчитано по факту фикстуры: 401 main rows (380 distinct plain
        // numbers + 19 concatenated ranges `"270273"` == `"270-273"` + 2 dash
        // ranges `"421-422"`) + 195 bonus rows (番外NN / `.5` chapters /
        // `第N坑` placeholders / notices).
        assert_eq!(main_count(&m.chapters), 518);
        assert_eq!(m.chapters.iter().filter(|c| !c.is_bonus).count(), 401);
        assert_eq!(m.chapters.iter().filter(|c| c.is_bonus).count(), 195);
        assert_eq!(m.count, Some(518));
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
    fn classify_plain_cn_parts_ranges_and_specials() {
        let c = classify_short_title;
        // Plain integers.
        assert_eq!(c("1"), ShortTitle::Base(1));
        assert_eq!(c(" 518 "), ShortTitle::Base(518));
        // `第N话` / `第N話`.
        assert_eq!(c("第533话"), ShortTitle::Base(533));
        assert_eq!(c("第533話"), ShortTitle::Base(533));
        // Split parts: dash variants and bracketed markers.
        assert_eq!(c("479-1"), ShortTitle::Part(479, 1));
        assert_eq!(c("479-2"), ShortTitle::Part(479, 2));
        assert_eq!(c("469—2"), ShortTitle::Part(469, 2));
        assert_eq!(c("414－2"), ShortTitle::Part(414, 2));
        assert_eq!(c("第533话（上）"), ShortTitle::Part(533, 1));
        assert_eq!(c("第533话（下）"), ShortTitle::Part(533, 2));
        assert_eq!(c("480【下】"), ShortTitle::Part(480, 2));
        // A head that already carries a part wins over the marker.
        assert_eq!(c("479-1（下）"), ShortTitle::Part(479, 1));
        // A marker chain resolves to the innermost marker (nearest the number).
        assert_eq!(c("5（上）（下）"), ShortTitle::Part(5, 1));
        // Merged ranges collapse to their first chapter.
        assert_eq!(c("421-422"), ShortTitle::Base(421));
        assert_eq!(c("270273"), ShortTitle::Base(270));
        // Odd-length / short digit runs are NOT ranges.
        assert_eq!(c("10011"), ShortTitle::Base(10011));
        assert_eq!(c("13031"), ShortTitle::Base(13031));
        // Fractional specials keep their x100 value.
        assert_eq!(c("49.5"), ShortTitle::Fractional(4950));
        assert_eq!(c("199.51"), ShortTitle::Fractional(19951));
        // Prologues, placeholders and notices are unnumbered.
        for raw in ["00", "0", "-3", "番外01", "第500坑", "公告", "", " "] {
            assert_eq!(c(raw), ShortTitle::Unnumbered, "{raw:?}");
        }
        // A wide second number that is not a forward range stays unnumbered.
        assert_eq!(c("900-100"), ShortTitle::Unnumbered);
    }

    #[test]
    fn parts_are_separate_rows_but_one_chapter() {
        let items = vec![
            serde_json::json!({ "short_title": "480" }),
            serde_json::json!({ "short_title": "479-2" }),
            serde_json::json!({ "short_title": "479-1" }),
        ];
        let chapters = episode_rows(&items);
        // Oldest-first: two tickable part rows, then the next chapter.
        let keys: Vec<i32> = chapters.iter().map(|c| c.number_x100).collect();
        assert_eq!(keys, vec![47901, 47902, 48000]);
        assert!(chapters.iter().all(|c| !c.is_bonus));
        // `count` collapses both parts into the single chapter 480.
        assert_eq!(main_count(&chapters), 480);
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
