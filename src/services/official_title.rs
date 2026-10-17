//! Title normalization and script detection for official-source matching.
//!
//! [`pick_candidate`] picks the title a provider should be searched with:
//! [`search_scripts`] lists the scripts that provider accepts, in preference
//! order, and the first candidate matching one of them wins. Most catalogs are
//! single-script (CN sites take Han, KR sites Hangul); the multi-script ones
//! need an order because the row title (usually Latin) finds nothing in their
//! native catalog: JP sites are searched with the Japanese title first
//! (alphapolis even answers 404 for Latin queries) and ridibooks with the
//! Korean title.

use unicode_normalization::UnicodeNormalization;

/// Dominant writing system of a title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Script {
    Hangul,
    /// Japanese: any kana makes a title Japanese (kanji-only titles stay Han).
    Japanese,
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
        if (0xAC00..=0xD7A3).contains(&cp)
            || (0x1100..=0x11FF).contains(&cp)
            || (0x3130..=0x318F).contains(&cp)
        {
            // Hangul syllables + Jamo + compatibility Jamo.
            hangul += 1;
        } else if (0x3040..=0x30FF).contains(&cp) || (0xFF66..=0xFF9D).contains(&cp) {
            // Hiragana + Katakana (+ halfwidth Katakana).
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

    // Kana implies Japanese: report Japanese so Han-only providers don't match
    // it and JP providers can prefer it over Latin romanizations.
    if hangul > 0 {
        Script::Hangul
    } else if kana > 0 {
        Script::Japanese
    } else if han > 0 {
        Script::Han
    } else if latin > 0 {
        Script::Latin
    } else {
        Script::Other
    }
}

/// Scripts a source accepts, in preference order: the first candidate whose
/// script appears in the list wins. Single-script catalogs get a one-entry
/// list; an empty list means "no candidate" (unknown/armless source).
fn search_scripts(source: &str) -> &'static [Script] {
    match source {
        // CN catalogs are Han-only.
        "kuaikan" | "bilibili" | "qidian" | "acqq" | "manmanapp" | "jinjiang" => &[Script::Han],
        // KR catalogs are Hangul-only; ridibooks also indexes Latin/JP rows.
        "kakao" | "naver" | "daum" | "naverwebtoon" | "bomtoon" => &[Script::Hangul],
        "ridibooks" => &[
            Script::Hangul,
            Script::Latin,
            Script::Japanese,
            Script::Other,
        ],
        // JP catalogs must be searched with the Japanese title first:
        // alphapolis answers 404 for Latin queries and the others index their
        // native names.
        "comicwalker" | "mangaup" | "alphapolis" | "kakuyomu" | "syosetu" | "comici"
        | "sundaywebry" | "yanmaga" => {
            &[Script::Japanese, Script::Han, Script::Latin, Script::Other]
        }
        // The global app indexes English names but also serves JP titles.
        "mangaplus" => &[Script::Latin, Script::Japanese, Script::Other],
        // Tappytoon's catalogue is English-only.
        "tappytoon" => &[Script::Latin],
        // WEBTOON's English site indexes English names.
        "webtoons" => &[Script::Latin],
        // Wuxiaworld's lite catalogue is English-only.
        "wuxiaworld" => &[Script::Latin],
        // Toomics and Manta index translated titles.
        "toomics" | "manta" => &[Script::Latin],
        _ => &[],
    }
}

/// First candidate whose script fits `source`; None when none fits.
pub fn pick_candidate<'a>(candidates: &'a [String], source: &str) -> Option<&'a str> {
    search_scripts(&source.to_ascii_lowercase())
        .iter()
        .find_map(|script| {
            candidates
                .iter()
                .find(|candidate| detect_script(candidate) == *script)
                .map(|candidate| strip_media_suffix(candidate))
        })
}

/// MU stores a media-type suffix on associated titles (`… (Novel)`,
/// `… (Manga)`); it is not part of the title, would narrow the search query and
/// would defeat [`titles_match`]'s normalized comparison.
fn strip_media_suffix(s: &str) -> &str {
    for suffix in [" (Novel)", " (Manga)", " (Manhwa)", " (Manhua)", " (Anime)"] {
        if let Some(head) = s.strip_suffix(suffix) {
            let head = head.trim_end();
            // A title that is *only* a suffix would strip to nothing and make
            // every such title compare equal - keep the original then.
            if !head.is_empty() {
                return head;
            }
        }
    }
    s
}

/// Title equality after normalization and media-suffix stripping.
pub fn titles_match(a: &str, b: &str) -> bool {
    normalize_title(strip_media_suffix(a)) == normalize_title(strip_media_suffix(b))
}

/// Keep the hits whose title matches the query: the exact normalized matches
/// when there are any, otherwise the normalized-substring matches. Sources
/// whose search page ignores the query or falls back to a generic listing
/// (mangaup's static catalogue, qidian's "no match" recommendations,
/// kakuyomu's fuzzy matches) use this so the search catalog never stores
/// unrelated filler. The exact step preserves the resolve walk's semantics:
/// an exact hit is never hidden behind substring noise.
pub fn filter_hits_by_query(
    hits: Vec<crate::services::official_types::OfficialHit>,
    query: &str,
) -> Vec<crate::services::official_types::OfficialHit> {
    let exact: Vec<_> = hits
        .iter()
        .filter(|h| titles_match(&h.title, query))
        .cloned()
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    let q = normalize_title(query);
    if q.is_empty() {
        return Vec::new();
    }
    hits.into_iter()
        .filter(|h| normalize_title(&h.title).contains(&q))
        .collect()
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
    fn media_type_suffix_is_ignored() {
        // MU associated titles carry a `(Novel)` suffix; the search hit does not.
        assert!(titles_match("巫界术士", "巫界术士 (Novel)"));
        assert!(titles_match(
            "Warlock of the Magus World",
            "Warlock of the Magus World (Novel)"
        ));
        // The query drops the suffix too.
        let candidates = vec!["巫界术士 (Novel)".to_string()];
        assert_eq!(pick_candidate(&candidates, "qidian"), Some("巫界术士"));
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
    fn detect_kana_is_japanese() {
        assert_eq!(detect_script("鬼滅の刃"), Script::Japanese);
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
    fn pick_naver_requires_hangul() {
        let candidates = ["One Piece".to_string(), "원피스".to_string()];
        assert_eq!(pick_candidate(&candidates, "naver"), Some("원피스"));
        assert_eq!(pick_candidate(&["One Piece".to_string()], "naver"), None);
        // The KR webtoon platform shares the Hangul-only rule.
        assert_eq!(pick_candidate(&candidates, "naverwebtoon"), Some("원피스"));
        assert_eq!(
            pick_candidate(&["One Piece".to_string()], "naverwebtoon"),
            None
        );
    }

    #[test]
    fn pick_kuaikan_none_without_han() {
        let candidates = ["One Piece".to_string(), "원피스".to_string()];
        assert_eq!(pick_candidate(&candidates, "kuaikan"), None);
    }

    #[test]
    fn pick_kuaikan_skips_kana_and_takes_han() {
        // A JP title with kana must not satisfy a Han-only catalog; the Han
        // translation wins when present, and a kana-only list yields nothing.
        let candidates = ["進撃の巨人".to_string(), "進擊的巨人".to_string()];
        assert_eq!(pick_candidate(&candidates, "kuaikan"), Some("進擊的巨人"));
        assert_eq!(pick_candidate(&["進撃の巨人".to_string()], "kuaikan"), None);
    }

    #[test]
    fn pick_mangaplus_returns_latin() {
        let candidates = ["One Piece".to_string(), "원피스".to_string()];
        assert_eq!(pick_candidate(&candidates, "mangaplus"), Some("One Piece"));
    }

    #[test]
    fn pick_tappytoon_requires_latin() {
        let candidates = ["Solo Leveling".to_string(), "나 혼자만 레벨업".to_string()];
        assert_eq!(
            pick_candidate(&candidates, "tappytoon"),
            Some("Solo Leveling")
        );
        assert_eq!(
            pick_candidate(&["나 혼자만 레벨업".to_string()], "tappytoon"),
            None
        );
    }

    #[test]
    fn pick_webtoons_requires_latin() {
        let candidates = [
            "Omniscient Reader".to_string(),
            "전지적 독자 시점".to_string(),
        ];
        assert_eq!(
            pick_candidate(&candidates, "webtoons"),
            Some("Omniscient Reader")
        );
        assert_eq!(
            pick_candidate(&["전지적 독자 시점".to_string()], "webtoons"),
            None
        );
    }

    #[test]
    fn pick_wuxiaworld_requires_latin() {
        let candidates = ["Coiling Dragon".to_string(), "盘龙".to_string()];
        assert_eq!(
            pick_candidate(&candidates, "wuxiaworld"),
            Some("Coiling Dragon")
        );
        assert_eq!(pick_candidate(&["盘龙".to_string()], "wuxiaworld"), None);
    }

    #[test]
    fn pick_acqq_requires_han() {
        let candidates = ["One Piece".to_string(), "斗破苍穹".to_string()];
        assert_eq!(pick_candidate(&candidates, "acqq"), Some("斗破苍穹"));
        assert_eq!(pick_candidate(&["One Piece".to_string()], "acqq"), None);
    }

    #[test]
    fn pick_bilibili_requires_han() {
        let candidates = [
            "One Piece".to_string(),
            "我的英雄学院".to_string(),
            "원피스".to_string(),
        ];
        assert_eq!(
            pick_candidate(&candidates, "bilibili"),
            Some("我的英雄学院")
        );
        assert_eq!(pick_candidate(&["One Piece".to_string()], "bilibili"), None);
    }

    #[test]
    fn pick_daum_requires_hangul() {
        let candidates = vec!["One Piece".to_string(), "원피스".to_string()];
        assert_eq!(pick_candidate(&candidates, "daum"), Some("원피스"));
        assert_eq!(pick_candidate(&["One Piece".to_string()], "daum"), None);
    }

    #[test]
    fn pick_ridibooks_prefers_hangul() {
        let candidates = vec![
            "The Greatest Blunder".to_string(),
            "인류 최대의 실수".to_string(),
        ];
        assert_eq!(
            pick_candidate(&candidates, "ridibooks"),
            Some("인류 최대의 실수")
        );
        // Latin-only rows still search with what they have; a JP title and a
        // Cyrillic-only (Other) row are the last resorts.
        assert_eq!(
            pick_candidate(&["One Piece".to_string()], "ridibooks"),
            Some("One Piece")
        );
        assert_eq!(
            pick_candidate(&["鬼滅の刃".to_string()], "ridibooks"),
            Some("鬼滅の刃")
        );
        assert_eq!(
            pick_candidate(&["Лунный бог".to_string()], "ridibooks"),
            Some("Лунный бог")
        );
    }

    #[test]
    fn pick_jp_sources_prefer_the_japanese_title() {
        // Realistic MangaUpdates associated titles for a JP series: the row
        // title and translations are Latin/Cyrillic, the native name is JP.
        let candidates = vec![
            "Tsuki ga Michibiku Isekai Douchuu".to_string(),
            "Tsukimichi: Moonlit Fantasy".to_string(),
            "Лунный бог, приключение и другой мир".to_string(),
            "月が導く異世界道中".to_string(),
            "月光下的异世界之旅".to_string(),
        ];
        for source in [
            "alphapolis",
            "comicwalker",
            "mangaup",
            "kakuyomu",
            "syosetu",
            "comici",
        ] {
            assert_eq!(
                pick_candidate(&candidates, source),
                Some("月が導く異世界道中"),
                "{source} must search with the Japanese title"
            );
            // Latin-only and Other-only rows still search with what they have.
            assert_eq!(
                pick_candidate(&["One Piece".to_string()], source),
                Some("One Piece"),
                "{source} falls back to Latin"
            );
            assert_eq!(
                pick_candidate(&["Лунный бог".to_string()], source),
                Some("Лунный бог"),
                "{source} falls back to Other"
            );
        }
    }

    #[test]
    fn titles_match_ignores_case_and_punct() {
        assert!(titles_match("One Piece", "one-piece"));
    }

    #[test]
    fn filter_hits_keeps_exact_then_contains() {
        use crate::services::official_types::OfficialHit;
        let hits = vec![
            OfficialHit::new("1", "Solo Leveling"),
            OfficialHit::new("2", "Solo Leveling: Ragnarok"),
            OfficialHit::new("3", "Unrelated Title"),
        ];
        // An exact match exists: only it survives (no substring noise).
        let filtered = filter_hits_by_query(hits.clone(), "Solo Leveling");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "1");
        // No exact match: substring matches survive.
        let filtered = filter_hits_by_query(hits.clone(), "solo");
        assert_eq!(filtered.len(), 2);
        // Nothing matches (the generic-listing case) or the query is empty.
        assert!(filter_hits_by_query(hits.clone(), "zzz").is_empty());
        assert!(filter_hits_by_query(hits, "!!!").is_empty());
    }

    #[test]
    fn titles_differ() {
        assert!(!titles_match("One Piece", "One Piece Party"));
    }
}
