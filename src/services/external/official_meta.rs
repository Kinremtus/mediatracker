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
    pub number_x100: i32,
    pub title: Option<String>,
    pub release_date: Option<NaiveDate>,
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

impl ChapterMeta {
    /// Integer chapter n -> x100. `n = 10.5` -> `1050`.
    pub fn from_f64(n: f64) -> Option<Self> {
        if !n.is_finite() || n <= 0.0 {
            return None;
        }
        let scaled = (n * 100.0).round() as i32;
        (scaled > 0).then_some(Self {
            number_x100: scaled,
            title: None,
            release_date: None,
        })
    }
}

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
        "daum" => crate::services::external::official_daum::fetch_meta(client, source_id).await,
        "qidian" => crate::services::external::official_qidian::fetch_meta(client, source_id).await,
        "ridibooks" => {
            crate::services::external::official_ridibooks::fetch_meta(client, source_id).await
        }
        "mangaup" => crate::services::external::official_mangaup::fetch_meta(client, source_id).await,
        _ => Ok(OfficialMeta::default()),
    }
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
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Extract the string value of a JSON-ish key `"key":"value"` without a full
/// serde parse. Handles the common embedded-JSON case; returns `None` on any
/// structural surprise. Decodes only `\"` and `\\` (enough for schedule text).
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
                    Some('n') => value.push('\n'),
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
pub type MetaFuture<'a> =
    Pin<Box<dyn Future<Output = anyhow::Result<OfficialMeta>> + Send + 'a>>;

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
        assert_eq!(between("<li class=\"_notice\">A</li>", "class=\"_notice\">", "</li>"), Some("A"));
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
        assert_eq!(normalize_schedule(" 매주 월요일 "), Some("매주 월요일".to_string()));
        assert_eq!(normalize_schedule("未定"), None);
        assert_eq!(normalize_schedule(""), None);
    }

    #[test]
    fn chapter_meta_from_f64_scales() {
        assert_eq!(ChapterMeta::from_f64(10.5).unwrap().number_x100, 1050);
        assert_eq!(ChapterMeta::from_f64(1.0).unwrap().number_x100, 100);
        assert!(ChapterMeta::from_f64(0.0).is_none());
        assert!(ChapterMeta::from_f64(-3.0).is_none());
    }
}
