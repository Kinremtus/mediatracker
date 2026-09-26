pub mod activity_calendar;

use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue};
use axum::response::Response;
use regex::Regex;
use std::net::IpAddr;
use std::sync::OnceLock;

pub fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn clean_description(text: Option<String>) -> Option<String> {
    let raw = text?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let cleaned = strip_footer(trimmed);
    let cleaned = strip_markdown_links(cleaned);
    let cleaned = collapse_whitespace(&cleaned);
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

fn strip_footer(text: &str) -> &str {
    const FOOTERS: &[&str] = &[
        "[Written by MAL Rewrite]",
        "(Written by MAL Rewrite)",
        "[Written by ShikimoriRewrite]",
        "(Source: MAL)",
    ];

    let mut current = text;
    for footer in FOOTERS {
        if let Some(stripped) = current
            .strip_suffix(footer)
            .or_else(|| current.strip_suffix(&footer.to_lowercase()))
        {
            current = stripped;
        }
    }
    current
}

fn markdown_link_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap())
}

fn strip_markdown_links(text: &str) -> String {
    markdown_link_regex().replace_all(text, "$1").into_owned()
}

fn collapse_whitespace(text: &str) -> String {
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let spaces = SPACES.get_or_init(|| Regex::new(r"[ \t]+").unwrap());
    let mut result = spaces.replace_all(text, " ").into_owned();

    static NEWLINES: OnceLock<Regex> = OnceLock::new();
    let newlines = NEWLINES.get_or_init(|| Regex::new(r"\n{3,}").unwrap());
    result = newlines.replace_all(&result, "\n\n").into_owned();

    result
}

/// Sanitize a user-supplied redirect target to prevent open redirects.
///
/// Returns `candidate` unchanged only when it is a safe, site-local path.
/// A value is accepted exclusively when ALL of the following hold:
///
/// * non-empty and not whitespace-only;
/// * starts with exactly one `/` — so `//evil.com` (protocol-relative) and
///   `/\evil` (backslash variant) are rejected;
/// * contains no backslash and no control characters — a control char in a
///   `Location` header enables response/header splitting;
/// * contains no scheme separator (`://`) and does not start with a scheme.
///   A single leading `/` already excludes `http:`, `javascript:`, etc., but
///   the explicit check keeps the rule defensible.
///
/// In every other case `fallback` is returned. A `?`-only string does not
/// start with `/`, so it is rejected by the leading-slash rule.
pub fn safe_redirect_path(candidate: Option<&str>, fallback: &str) -> String {
    let Some(raw) = candidate else {
        return fallback.to_string();
    };

    // Reject empty / whitespace-only payloads (also catches "   ").
    if raw.trim().is_empty() {
        return fallback.to_string();
    }

    // Reject backslashes and control characters. Backslashes can be
    // normalised to '/' by some clients, and control chars enable header
    // splitting in the Location response header.
    if raw.chars().any(|c| c == '\\' || c.is_control()) {
        return fallback.to_string();
    }

    // Must be an absolute *path*: exactly one leading slash. This rules out
    // protocol-relative URLs ("//host") and any bare scheme ("https:...").
    if !raw.starts_with('/') || raw.starts_with("//") {
        return fallback.to_string();
    }

    // Defence-in-depth: never allow a scheme separator anywhere.
    if raw.contains("://") {
        return fallback.to_string();
    }

    raw.to_string()
}

/// Extract the raw `session_id` cookie value from request headers.
///
/// Cookies are a single `Cookie:` header with `; `-separated `name=value`
/// pairs. Returns `None` when the header is absent, malformed, or the value
/// is empty. The value is returned raw (never hashed here) so callers can
/// pass it to [`crate::utils::sha256_hex`] before any DB lookup.
pub fn session_cookie(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|c| {
            c.split(';')
                .map(str::trim)
                .find_map(|pair| pair.strip_prefix("session_id="))
        })
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// Resolve the real client IP from proxy headers, in trust order:
///
/// 1. `cf-connecting-ip` — a single value set (and overwritten) by
///    Cloudflare, so it is the most trustworthy source.
/// 2. leftmost entry of `x-forwarded-for` — the original client appended by
///    the first proxy in the chain. The socket peer, if reached, is the
///    nearest proxy, not the client, so it is not trusted when a header is
///    present.
/// 3. `fallback` — the TCP peer address, used only when no header is present.
///
/// Returns `None` when no valid IP is available. Every candidate is parsed as
/// an [`IpAddr`] before being trusted, so a bogus/placeholder header value can
/// never be bound to an `inet` column and abort the request.
pub fn client_ip(headers: &HeaderMap, fallback: Option<IpAddr>) -> Option<String> {
    if let Some(ip) = headers
        .get("cf-connecting-ip")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<IpAddr>().ok())
    {
        return Some(ip.to_string());
    }

    if let Some(ip) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<IpAddr>().ok())
    {
        return Some(ip.to_string());
    }

    fallback.map(|ip| ip.to_string())
}

/// Inserts the `HX-Trigger` response header.
///
/// Skips the header (and logs a warning) when the payload cannot be turned into
/// a header value, so a malformed trigger can never panic the handler.
pub fn set_hx_trigger(resp: &mut Response, payload: &str) {
    match HeaderValue::from_str(payload) {
        Ok(value) => {
            resp.headers_mut().insert("HX-Trigger", value);
        }
        Err(err) => {
            tracing::warn!(error = %err, "skipping invalid HX-Trigger header");
        }
    }
}

/// Inserts the `Set-Cookie` response header.
///
/// Skips the header (and logs an error) when the cookie cannot be turned into a
/// header value instead of panicking.
pub fn set_set_cookie(resp: &mut Response, cookie: &str) {
    match HeaderValue::from_str(cookie) {
        Ok(value) => {
            resp.headers_mut().insert(SET_COOKIE, value);
        }
        Err(err) => {
            tracing::error!(error = %err, "skipping invalid Set-Cookie header");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_mal_rewrite_suffix() {
        let input = Some(
            "Twelve years ago the Village Hidden in the Leaves...\n[Written by MAL Rewrite]"
                .to_string(),
        );
        assert_eq!(
            clean_description(input).as_deref(),
            Some("Twelve years ago the Village Hidden in the Leaves...")
        );
    }

    #[test]
    fn strips_trailing_whitespace() {
        let input = Some("  Hello world  ".to_string());
        assert_eq!(clean_description(input).as_deref(), Some("Hello world"));
    }

    #[test]
    fn returns_none_for_empty() {
        assert_eq!(clean_description(Some("   ".to_string())), None);
        assert_eq!(clean_description(None), None);
    }

    #[test]
    fn keeps_description_without_footer() {
        let input = Some("A simple description.".to_string());
        assert_eq!(
            clean_description(input).as_deref(),
            Some("A simple description.")
        );
    }

    #[test]
    fn strips_shikimori_footer() {
        let input = Some("Some synopsis text[Written by ShikimoriRewrite]".to_string());
        assert_eq!(
            clean_description(input).as_deref(),
            Some("Some synopsis text")
        );
    }

    #[test]
    fn strips_markdown_link_keeping_text() {
        let input = Some("From Viz: story text [Jump +](https://shonenjumpplus.com/episode/108335195563250218) more text".to_string());
        assert_eq!(
            clean_description(input).as_deref(),
            Some("From Viz: story text Jump + more text")
        );
    }

    #[test]
    fn strips_multiple_markdown_links() {
        let input = Some("**Original**: [Jump +](https://a.com/x) **English**: [Read for FREE on Manga Plus](https://b.com/y)".to_string());
        assert_eq!(
            clean_description(input).as_deref(),
            Some("**Original**: Jump + **English**: Read for FREE on Manga Plus")
        );
    }

    #[test]
    fn strips_entire_mangaupdates_description() {
        let input = Some(
            "From Viz: Twelve years ago the Village Hidden in the Leaves was attacked...\n\
             **Original**: [Jump +](https://shonenjumpplus.com/episode/108335195563250218)\n\
             **Translations**\n\
             **English**: [Read for FREE on Manga Plus](https://mangaplus.shueisha.co.jp/titles/100018)"
                .to_string(),
        );
        let result = clean_description(input).unwrap();
        assert!(!result.contains("https://"));
        assert!(!result.contains("]("));
        assert!(result.contains("Jump +"));
        assert!(result.contains("Read for FREE on Manga Plus"));
    }

    #[test]
    fn collapses_excess_whitespace() {
        let input = Some("Line 1.\n\n\n\nLine 2.".to_string());
        assert_eq!(
            clean_description(input).as_deref(),
            Some("Line 1.\n\nLine 2.")
        );
    }

    #[test]
    fn safe_redirect_path_table() {
        const FALLBACK: &str = "/tracking";
        let cases: &[(Option<&str>, &str)] = &[
            // accepted site-local paths
            (Some("/tracking"), "/tracking"),
            (Some("/search?q=x"), "/search?q=x"),
            (Some("/search?q=x#results"), "/search?q=x#results"),
            // rejected -> fallback
            (Some("//evil.com"), FALLBACK),
            (Some("/\\evil"), FALLBACK),
            (Some("https://evil.com"), FALLBACK),
            (Some("http://"), FALLBACK),
            (Some("javascript:alert(1)"), FALLBACK),
            (Some(""), FALLBACK),
            (Some("   "), FALLBACK),
            (Some("?"), FALLBACK),
            (Some("evil"), FALLBACK),
            (None, FALLBACK),
        ];

        for (candidate, expected) in cases {
            assert_eq!(
                safe_redirect_path(*candidate, FALLBACK),
                *expected,
                "candidate={:?}",
                candidate
            );
        }
    }

    #[test]
    fn safe_redirect_path_rejects_control_chars() {
        assert_eq!(
            safe_redirect_path(Some("/track\ning"), "/tracking"),
            "/tracking"
        );
        assert_eq!(
            safe_redirect_path(Some("/track\r\nSet-Cookie: x=1"), "/tracking"),
            "/tracking"
        );
    }

    #[test]
    fn session_cookie_extracts_value() {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::COOKIE,
            "theme=dark; session_id=abc123; other=1".parse().unwrap(),
        );
        assert_eq!(session_cookie(&h).as_deref(), Some("abc123"));

        let mut empty = HeaderMap::new();
        assert_eq!(session_cookie(&empty), None);
        empty.insert(axum::http::header::COOKIE, "session_id=".parse().unwrap());
        assert_eq!(session_cookie(&empty), None);
    }

    #[test]
    fn client_ip_precedence_and_fallback() {
        use std::net::{IpAddr, Ipv4Addr};

        let fallback = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));

        // No headers -> fallback, or None when absent.
        let empty = HeaderMap::new();
        assert_eq!(client_ip(&empty, fallback), Some("10.0.0.1".to_string()));
        assert_eq!(client_ip(&empty, None), None);

        // x-forwarded-for -> leftmost entry.
        let mut xff = HeaderMap::new();
        xff.insert("x-forwarded-for", "198.51.100.9, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&xff, fallback), Some("198.51.100.9".to_string()));

        // cf-connecting-ip wins over x-forwarded-for.
        let mut both = HeaderMap::new();
        both.insert("cf-connecting-ip", "203.0.113.7".parse().unwrap());
        both.insert("x-forwarded-for", "198.51.100.9, 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&both, fallback), Some("203.0.113.7".to_string()));
    }

    #[test]
    fn client_ip_ignores_blank_headers() {
        use std::net::{IpAddr, Ipv4Addr};

        let fallback = Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        let mut h = HeaderMap::new();
        h.insert("cf-connecting-ip", "   ".parse().unwrap());
        h.insert("x-forwarded-for", "  , 10.0.0.1".parse().unwrap());
        assert_eq!(client_ip(&h, fallback), Some("10.0.0.1".to_string()));
    }

    #[test]
    fn client_ip_rejects_unparseable_header() {
        // A bogus header must never be returned (it would break an `inet` bind).
        let mut h = HeaderMap::new();
        h.insert("cf-connecting-ip", "not-an-ip".parse().unwrap());
        assert_eq!(client_ip(&h, None), None);
    }
}
