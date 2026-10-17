//! Shared DTO and helpers for official-source detail enrichment
//! (chapter titles/dates + free-text schedule / exact next-update date).
//!
//! Parsers are unit-testable without network. All helpers are defensive: a
//! malformed input yields an empty/*None* value, never a panic.

use std::future::Future;
use std::pin::Pin;

use chrono::NaiveDate;

/// One chapter row read from an official source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterMeta {
    /// Chapter number scaled x100 (matches `series_chapters.chapter_number`).
    /// `0` when the source reports no number (bonus prologue / extra).
    pub number_x100: i32,
    pub title: Option<String>,
    pub release_date: Option<NaiveDate>,
    /// True for bonus rows (`00` prologues, non-numeric specials, repeated
    /// numbers, Korean `예고편`/`외전`, Japanese `番外編`/`特別編`, Chinese
    /// `番外`/`序章`, MangaPlus `ex` rows). Set by every source that classifies
    /// titles through
    /// [`chapter_title`](crate::services::external::chapter_title) (bilibili,
    /// kuaikan, kakao, ridibooks, mangaplus, comicwalker, mangaup, alphapolis,
    /// narou, kakuyomu, qidian, tappytoon, webtoons, acqq) or `is_kr_bonus_title`
    /// (naver); the remaining sources report their plain numbered rows as
    /// `false`.
    pub is_bonus: bool,
    /// Display label the source already knows (split parts `479-1`, `2-1`),
    /// stored verbatim into `series_chapters.chapter_label` by `apply_meta`.
    /// `None` lets `apply_meta` derive one (fractional number / counter).
    pub label: Option<String>,
}

/// Everything one official source can tell us about a title.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OfficialMeta {
    /// Which binding produced this (e.g. `"comicwalker"`).
    pub source: String,
    pub chapters: Vec<ChapterMeta>,
    /// Authoritative count when the source reports one (raise-only at write time).
    pub count: Option<i32>,
    pub update_schedule: Option<String>,
    pub next_update_at: Option<NaiveDate>,
}

/// Sources with a real arm in `fetch_source` (each source here has a working fetch).
/// Every chained source is meta-capable today; the list still guards against a
/// future arm-less source (such a binding must keep `chapter_key` on the MU key
/// instead of pointing at rows no fetch ever writes).
/// `kuaikan`, `kakao`, `mangaplus`, `tappytoon`, `webtoons`, `acqq`, `comici`,
/// `naverwebtoon` and `wuxiaworld` gained their arms after
/// `migrations/028_chapter_ownership.sql`; that migration's `supports_meta`
/// list is a frozen one-time-backfill snapshot, so this runtime list is the
/// source of truth for `chapter_key` and the resolve walk.
pub const SOURCES_WITH_META: &[&str] = &[
    "bilibili",
    "naver",
    "daum",
    "ridibooks",
    "comicwalker",
    "mangaup",
    "alphapolis",
    "syosetu",
    "kakuyomu",
    "qidian",
    "kuaikan",
    "kakao",
    "mangaplus",
    "tappytoon",
    "webtoons",
    "acqq",
    "comici",
    "naverwebtoon",
    "wuxiaworld",
    "bomtoon",
    "toomics",
    "manmanapp",
    "sundaywebry",
    "yanmaga",
];

/// Fetch one source's detail, dispatched by the binding name. Unknown/absent
/// source -> empty `OfficialMeta` (never an error, so callers stay silent).
pub async fn fetch_source(
    source: &str,
    client: &reqwest::Client,
    source_id: &str,
) -> anyhow::Result<OfficialMeta> {
    match source {
        "naver" => crate::services::external::official_naver::fetch_meta(client, source_id).await,
        "syosetu" => crate::services::external::official_narou::fetch_meta(client, source_id).await,
        "comicwalker" => {
            crate::services::external::official_comicwalker::fetch_meta(client, source_id).await
        }
        "kakuyomu" => {
            crate::services::external::official_kakuyomu::fetch_meta(client, source_id).await
        }
        "alphapolis" => {
            crate::services::external::official_alphapolis::fetch_meta(client, source_id).await
        }
        "bilibili" => {
            crate::services::external::official_bilibili::fetch_meta(client, source_id).await
        }
        "kuaikan" => {
            crate::services::external::official_kuaikan::fetch_meta(client, source_id).await
        }
        // MangaPlus needs the app registration handshake, so it builds its own
        // client instead of taking the shared one.
        "mangaplus" => crate::services::external::official_mangaplus::fetch_meta(source_id).await,
        "kakao" => crate::services::external::official_kakao::fetch_meta(client, source_id).await,
        "daum" => crate::services::external::official_daum::fetch_meta(client, source_id).await,
        "qidian" => crate::services::external::official_qidian::fetch_meta(client, source_id).await,
        "ridibooks" => {
            crate::services::external::official_ridibooks::fetch_meta(client, source_id).await
        }
        "mangaup" => {
            crate::services::external::official_mangaup::fetch_meta(client, source_id).await
        }
        "tappytoon" => {
            crate::services::external::official_tappytoon::fetch_meta(client, source_id).await
        }
        "webtoons" => {
            crate::services::external::official_webtoons::fetch_meta(client, source_id).await
        }
        "acqq" => crate::services::external::official_acqq::fetch_meta(client, source_id).await,
        "comici" => crate::services::external::official_comici::fetch_meta(client, source_id).await,
        "naverwebtoon" => {
            crate::services::external::official_naverwebtoon::fetch_meta(client, source_id).await
        }
        "wuxiaworld" => {
            crate::services::external::official_wuxiaworld::fetch_meta(client, source_id).await
        }
        "bomtoon" => {
            crate::services::external::official_bomtoon::fetch_meta(client, source_id).await
        }
        "toomics" => {
            crate::services::external::official_toomics::fetch_meta(client, source_id).await
        }
        "manmanapp" => {
            crate::services::external::official_manmanapp::fetch_meta(client, source_id).await
        }
        "sundaywebry" => {
            crate::services::external::official_sundaywebry::fetch_meta(client, source_id).await
        }
        "yanmaga" => {
            crate::services::external::official_yanmaga::fetch_meta(client, source_id).await
        }
        _ => Ok(OfficialMeta::default()),
    }
}

/// Largest detail body a meta fetcher will read (4 MiB). Fetchers check the
/// response `content_length` against this before buffering the body: a hostile
/// or broken upstream must not exhaust memory. Chunked responses carry no
/// `content_length` and stay bounded only by the shared client timeout.
pub const MAX_DETAIL_BYTES: u64 = 4 * 1024 * 1024;

/// True when a bound source id is a plain digit run — the only valid shape for
/// the ids that land in a fetcher's URL path or query string.
pub fn is_digit_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}

/// True when a bound source id is safe to embed in a URL path: non-empty, at
/// most 64 chars, and only ASCII alphanumerics, `_` or `-` (Comic Walker work
/// codes look like `KC_000001_S`).
pub fn is_safe_path_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Read a response body with a hard size cap. `content_length` is checked
/// before buffering, then every chunk is counted as it arrives — a chunked
/// response (no `content_length`) is cut off mid-stream instead of being
/// buffered whole. The body is decoded lossily as UTF-8.
pub async fn body_capped(mut resp: reqwest::Response, cap: u64) -> anyhow::Result<String> {
    if let Some(len) = resp.content_length()
        && len > cap
    {
        anyhow::bail!("response too large: {len} bytes (cap {cap})");
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() as u64 + chunk.len() as u64 > cap {
            anyhow::bail!("response too large: over {cap} bytes");
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The embedded `__NEXT_DATA__` JSON of a Next.js SSR page, starting at its
/// opening `{`. `None` when the script tag or the JSON start is missing.
pub fn next_data_str(html: &str) -> Option<&str> {
    let seg = between(html, "__NEXT_DATA__", "</script>")?;
    let start = seg.find('{')?;
    Some(&seg[start..])
}

/// The largest index `<= idx` that is a char boundary, so a byte window can be
/// sliced without panicking on a multi-byte character. Parser helpers use it to
/// bound tag/attribute searches to a small window.
pub fn floor_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `name="value"` of one attribute inside a tag. The name must start a word, so
/// `title` never matches inside `data-title`. `<a data-x="1" class="y">` yields
/// `Some("y")` for `class`. Shared by the HTML sources whose hits live in tag
/// attributes (webtoons, acqq).
pub fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let mut from = 0usize;
    while let Some(rel) = tag[from..].find(&key) {
        let at = from + rel;
        let boundary = at == 0
            || tag[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || c == '<');
        if boundary {
            let start = at + key.len();
            let rest = &tag[start..];
            let end = rest.find('"')?;
            return Some(rest[..end].to_string());
        }
        from = at + key.len();
    }
    None
}

/// Return the substring strictly between `start` and the next `end` after it.
/// `None` when either marker is missing.
pub fn between<'a>(haystack: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let s = haystack.find(start)? + start.len();
    let rest = &haystack[s..];
    let e = rest.find(end)?;
    Some(&rest[..e])
}

/// Strip HTML tags, collapse whitespace, trim. Never panics.
pub fn strip_tags(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            // Control characters (a NUL breaks the Postgres insert) are dropped;
            // whitespace keeps its separating effect as a space.
            _ if !in_tag && !c.is_control() => out.push(c),
            _ if !in_tag && c.is_whitespace() => out.push(' '),
            _ => {}
        }
    }
    // Titles compare through `normalize_title`, which keeps the digits of an
    // undecoded `&#39;` and would then miss the real series name.
    let decoded = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the string value of a JSON-ish key `"key":"value"` without a full
/// serde parse. Handles the common embedded-JSON case; returns `None` on any
/// structural surprise. Decodes the standard escapes (`\"`, `\\`, `\/`,
/// `\n`, `\t`, `\r`, `\b`, `\f`) and `\uXXXX` (surrogate pairs included).
pub fn json_string_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut from = 0usize;
    while let Some(rel) = body[from..].find(&needle) {
        let idx = from + rel + needle.len();
        let rest = body[idx..].trim_start();
        let Some(rest) = rest.strip_prefix(':') else {
            from = idx;
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('"') else {
            from = idx;
            continue;
        };
        let mut value = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('"') => value.push('"'),
                    Some('\\') => value.push('\\'),
                    Some('/') => value.push('/'),
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('r') => value.push('\r'),
                    Some('b') => value.push('\u{0008}'),
                    Some('f') => value.push('\u{000C}'),
                    Some('u') => value.push(decode_unicode_escape(&mut chars)?),
                    Some(other) => value.push(other),
                    None => return None,
                },
                '"' => return Some(value),
                other => value.push(other),
            }
        }
        return None;
    }
    None
}

/// One `\uXXXX` escape (the `\u` is already consumed): four hex digits, plus
/// the low half of a surrogate pair when the code is a high surrogate.
fn decode_unicode_escape(chars: &mut std::str::Chars<'_>) -> Option<char> {
    let mut code = 0u32;
    for _ in 0..4 {
        code = code * 16 + chars.next()?.to_digit(16)?;
    }
    if (0xD800..=0xDBFF).contains(&code) {
        // A high surrogate must be followed by the low half of the pair.
        if chars.next()? != '\\' || chars.next()? != 'u' {
            return None;
        }
        let mut low = 0u32;
        for _ in 0..4 {
            low = low * 16 + chars.next()?.to_digit(16)?;
        }
        if !(0xDC00..=0xDFFF).contains(&low) {
            return None;
        }
        return char::from_u32(0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00));
    }
    char::from_u32(code)
}

/// Extract the integer value of a JSON-ish key `"key":123` without a full
/// serde parse. Skips whitespace after `:`, then parses the leading optional
/// `-` digit run as `i64`. Returns `None` on any structural surprise (missing
/// key, non-numeric value, overflow); never panics.
pub fn json_number_field(body: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{key}\"");
    let mut from = 0usize;
    while let Some(rel) = body[from..].find(&needle) {
        let idx = from + rel + needle.len();
        let rest = body[idx..].trim_start();
        let Some(rest) = rest.strip_prefix(':') else {
            from = idx;
            continue;
        };
        let rest = rest.trim_start();
        let bytes = rest.as_bytes();
        let mut i = 0usize;
        let negative = bytes.first() == Some(&b'-');
        if negative {
            i = 1;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return None;
        }
        let value: i64 = rest[start..i].parse().ok()?;
        return Some(if negative { -value } else { value });
    }
    None
}

/// Parse `YYYY-MM-DD`, `YYYY/MM/DD`, `YYYY.MM.DD`, `YYYY年MM月DD日`.
/// Returns `None` for empty, `未定`, `未定`, `TBD` and anything malformed.
pub fn parse_date(raw: &str) -> Option<NaiveDate> {
    let cleaned: String = raw
        .trim()
        .replace(['年', '月'], "-")
        .replace("日", "")
        .replace(['/', '.'], "-");
    let cleaned = cleaned.trim();
    NaiveDate::parse_from_str(cleaned, "%Y-%m-%d").ok()
}

/// Normalize a free-text schedule: strip tags, collapse whitespace, drop
/// empties and the well-known "unknown" markers. `None` when nothing remains.
pub fn normalize_schedule(raw: &str) -> Option<String> {
    let text = strip_tags(raw);
    let text = text.trim();
    if text.is_empty()
        || matches!(text, "未定" | "不定" | "TBD" | "N/A" | "-")
        || text.contains("未定")
    {
        return None;
    }
    Some(text.to_string())
}

/// Boxed future returned by [`MetaSource::fetch`] (mirrors `SearchFuture`).
pub type MetaFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<OfficialMeta>> + Send + 'a>>;

/// A guest-accessible official source of chapter metadata.
pub trait MetaSource: Send + Sync {
    fn name(&self) -> &'static str;
    fn media_types(&self) -> &'static [&'static str];
    fn fetch<'a>(&'a self, client: &'a reqwest::Client, source_id: &'a str) -> MetaFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_extracts() {
        assert_eq!(
            between(
                "<li class=\"_notice\">A</li>",
                "class=\"_notice\">",
                "</li>"
            ),
            Some("A")
        );
        assert_eq!(between("no markers", "x", "y"), None);
    }

    #[test]
    fn strip_tags_collapses() {
        assert_eq!(strip_tags("<b>a</b>  <i>b</i>"), "a b");
    }

    #[test]
    fn json_field_reads_string() {
        assert_eq!(
            json_string_field(r#"{"renewal_time":"每周四14点更新","x":1}"#, "renewal_time"),
            Some("每周四14点更新".to_string())
        );
        assert_eq!(json_string_field(r#"{"a":1}"#, "renewal_time"), None);
    }

    #[test]
    fn json_field_decodes_escapes() {
        // Standard escapes and `\uXXXX` (the mangaup cover URLs carry `\u0026`).
        assert_eq!(
            json_string_field(r#"{"url":"/asset/x.webp?h=AbC\u0026e=123"}"#, "url"),
            Some("/asset/x.webp?h=AbC&e=123".to_string())
        );
        assert_eq!(
            json_string_field(r#"{"t":"\u65e5\u672c"}"#, "t"),
            Some("日本".to_string())
        );
        // A surrogate pair decodes into one astral character.
        assert_eq!(
            json_string_field(r#"{"t":"\ud83d\ude00"}"#, "t"),
            Some("😀".to_string())
        );
        // A malformed escape is a structural surprise.
        assert_eq!(json_string_field(r#"{"t":"\u12"}"#, "t"), None);
        assert_eq!(json_string_field(r#"{"t":"\ud83d x"}"#, "t"), None);
    }

    #[test]
    fn json_number_field_reads_int() {
        let body = r#"{"code":503,"msg":"search limit! need login."}"#;
        assert_eq!(json_number_field(body, "code"), Some(503));
        assert_eq!(json_number_field(r#"{"code": -7}"#, "code"), Some(-7));
        assert_eq!(json_number_field(r#"{"code":"x"}"#, "code"), None);
        assert_eq!(json_number_field(r#"{"a":1}"#, "code"), None);
    }

    #[test]
    fn parse_date_accepts_formats() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 2).unwrap();
        assert_eq!(parse_date("2026-10-02"), Some(d));
        assert_eq!(parse_date("2026/10/02"), Some(d));
        assert_eq!(parse_date("2026.10.02"), Some(d));
        assert_eq!(parse_date("2026年10月2日"), Some(d));
        assert_eq!(parse_date("未定"), None);
        assert_eq!(parse_date(""), None);
    }

    #[test]
    fn normalize_schedule_drops_unknown() {
        assert_eq!(
            normalize_schedule(" 매주 월요일 "),
            Some("매주 월요일".to_string())
        );
        assert_eq!(normalize_schedule("未定"), None);
        assert_eq!(normalize_schedule(""), None);
    }
}
