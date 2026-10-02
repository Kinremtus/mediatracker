//! Title normalization and script detection for official-source matching.
//!
//! Used to decide whether a search hit from an official provider (kakao,
//! kuaikan, mangaplus) is written in the script that provider actually serves.

use unicode_normalization::UnicodeNormalization;

/// Dominant writing system of a title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Hangul,
    Han,
    Latin,
    Other,
}

/// NFC, fullwidth -> halfwidth fold, lower-case, drop whitespace + punctuation.
pub fn normalize_title(s: &str) -> String {
    s.nfc()
        .map(|c| match c {
            // Fullwidth ASCII forms U+FF01..=U+FF5E map to their halfwidth form.
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(c as u32 - 0xFEE0).unwrap_or(c),
            // Ideographic space becomes a regular space (later dropped).
            '\u{3000}' => ' ',
            other => other,
        })
        // `to_lowercase` may expand one char into several (e.g. 'İ').
        .flat_map(|c| c.to_lowercase())
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// Dominant script of a candidate title.
pub fn detect_script(s: &str) -> Script {
    let mut hangul = 0usize;
    let mut kana = 0usize;
    let mut han = 0usize;
    let mut latin = 0usize;

    for c in s.chars() {
        let cp = c as u32;
        if (0xAC00..=0xD7A3).contains(&cp) || (0x1100..=0x11FF).contains(&cp) {
            // Hangul syllables + Jamo.
            hangul += 1;
        } else if (0x3040..=0x30FF).contains(&cp) {
            // Hiragana + Katakana.
            kana += 1;
        } else if (0x4E00..=0x9FFF).contains(&cp) || (0x3400..=0x4DBF).contains(&cp) {
            // CJK Unified Ideographs + Extension A.
            han += 1;
        } else if (0x0041..=0x005A).contains(&cp)
            || (0x0061..=0x007A).contains(&cp)
            || (0x00C0..=0x024F).contains(&cp)
        {
            // ASCII letters + Latin-1 Supplement / Latin Extended-A, B.
            latin += 1;
        }
    }

    // Kana implies Japanese: report Other so Han-only providers don't match it.
    if hangul > 0 {
        Script::Hangul
    } else if kana > 0 {
        Script::Other
    } else if han > 0 {
        Script::Han
    } else if latin > 0 {
        Script::Latin
    } else {
        Script::Other
    }
}

/// First candidate whose script fits `source`; None when none fits.
pub fn pick_candidate<'a>(candidates: &'a [String], source: &str) -> Option<&'a str> {
    let source = source.to_ascii_lowercase();
    candidates.iter().find_map(|candidate| {
        let script = detect_script(candidate);
        let fits = match source.as_str() {
            "kakao" => script == Script::Hangul,
            "kuaikan" => script == Script::Han,
            "mangaplus" => matches!(script, Script::Latin | Script::Other),
            _ => false,
        };
        fits.then_some(candidate.as_str())
    })
}

/// Title equality after normalization.
pub fn titles_match(a: &str, b: &str) -> bool {
    normalize_title(a) == normalize_title(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_cyrillic_case_punct() {
        assert_eq!(normalize_title("  Ван-Пис!! "), normalize_title("ван пис"));
    }

    #[test]
    fn normalize_fullwidth_ascii() {
        assert_eq!(normalize_title("ＴＯＫＹＯ"), "tokyo");
    }

    #[test]
    fn normalize_empty() {
        assert_eq!(normalize_title("   !!! "), "");
    }

    #[test]
    fn normalize_idempotent() {
        let once = normalize_title("One Piece");
        assert_eq!(normalize_title(&once), once);
    }

    #[test]
    fn detect_hangul() {
        assert_eq!(detect_script("용사파티"), Script::Hangul);
    }

    #[test]
    fn detect_han() {
        assert_eq!(detect_script("我的英雄学院"), Script::Han);
    }

    #[test]
    fn detect_latin() {
        assert_eq!(detect_script("One Piece"), Script::Latin);
    }

    #[test]
    fn detect_kana_is_other() {
        assert_eq!(detect_script("鬼滅の刃"), Script::Other);
    }

    #[test]
    fn detect_empty_is_other() {
        assert_eq!(detect_script(""), Script::Other);
    }

    #[test]
    fn pick_kakao_prefers_hangul() {
        let candidates = ["One Piece".to_string(), "원피스".to_string()];
        assert_eq!(pick_candidate(&candidates, "kakao"), Some("원피스"));
    }

    #[test]
    fn pick_kuaikan_none_without_han() {
        let candidates = ["One Piece".to_string(), "원피스".to_string()];
        assert_eq!(pick_candidate(&candidates, "kuaikan"), None);
    }

    #[test]
    fn pick_mangaplus_returns_latin() {
        let candidates = ["One Piece".to_string(), "원피스".to_string()];
        assert_eq!(pick_candidate(&candidates, "mangaplus"), Some("One Piece"));
    }

    #[test]
    fn titles_match_ignores_case_and_punct() {
        assert!(titles_match("One Piece", "one-piece"));
    }

    #[test]
    fn titles_differ() {
        assert!(!titles_match("One Piece", "One Piece Party"));
    }
}
