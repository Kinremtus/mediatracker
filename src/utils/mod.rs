pub mod activity_calendar;

use regex::Regex;
use std::sync::OnceLock;

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
    markdown_link_regex()
        .replace_all(text, "$1")
        .into_owned()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_mal_rewrite_suffix() {
        let input = Some("Twelve years ago the Village Hidden in the Leaves...\n[Written by MAL Rewrite]".to_string());
        assert_eq!(
            clean_description(input).as_deref(),
            Some("Twelve years ago the Village Hidden in the Leaves...")
        );
    }

    #[test]
    fn strips_trailing_whitespace() {
        let input = Some("  Hello world  ".to_string());
        assert_eq!(
            clean_description(input).as_deref(),
            Some("Hello world")
        );
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
}
