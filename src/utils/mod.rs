pub mod activity_calendar;

pub fn clean_description(text: Option<String>) -> Option<String> {
    let raw = text?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let cleaned = strip_footer(trimmed);
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
}
