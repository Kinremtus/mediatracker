//! Shared chapter-title classification for the meta-capable official sources.
//!
//! Sources report chapter numbers as free-form titles (`281`, `第533话`,
//! `479-2`, `第533话（下）`). The classifiers here turn one title into a
//! [`ShortTitle`]; [`rows_to_chapters`] then maps an oldest-first row list onto
//! [`ChapterMeta`] rows (the first sighting of a key is a main chapter, every
//! repeat a bonus duplicate) and [`main_count`] derives the chapter count from
//! the highest main base.
//!
//! Moved out of `official_bilibili` when kuaikan gained a fetcher: both sources
//! speak the same `第N话` + `（上）/（下）` + `N-M` dialect.

use std::collections::HashSet;

use chrono::NaiveDate;

use crate::services::external::official_meta::ChapterMeta;

/// How one source title maps onto the chapter sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortTitle {
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

/// One source row reduced to the fields the chapter mapper keeps.
pub struct EpisodeRow {
    pub title: Option<String>,
    pub release_date: Option<NaiveDate>,
    pub kind: ShortTitle,
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

/// `第N话` / `第N話` -> `Some(N)` for `N >= 1` when the whole string is the
/// number.
fn parse_cn_chapter(s: &str) -> Option<i32> {
    let rest = s.strip_prefix('第')?;
    let digits = rest
        .strip_suffix('话')
        .or_else(|| rest.strip_suffix('話'))?;
    let n: i32 = digits.trim().parse().ok()?;
    (n >= 1).then_some(n)
}

/// `第N话<tail>` -> `Some((N, tail))` for `N >= 1`: kuaikan titles carry the
/// chapter's own title text after the number (`第533话 圣帝之手`), bilibili's
/// tail is empty.
fn parse_cn_chapter_prefix(s: &str) -> Option<(i32, &str)> {
    let rest = s.strip_prefix('第')?;
    let digits_end = rest.find(|c: char| !c.is_ascii_digit())?;
    if digits_end == 0 {
        return None;
    }
    let n: i32 = rest[..digits_end].parse().ok()?;
    let after = &rest[digits_end..];
    let tail = after
        .strip_prefix('话')
        .or_else(|| after.strip_prefix('話'))?;
    (n >= 1).then_some((n, tail))
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

/// Strip every trailing `上/下` marker and return the remaining head plus the
/// part number (the marker nearest the number wins).
fn strip_part_markers(s: &str) -> (&str, Option<i32>) {
    let mut head = s.trim();
    let mut part: Option<i32> = None;
    while let Some((h, p)) = split_part_marker(head) {
        part = Some(p);
        head = h;
    }
    (head, part)
}

/// Classify one `short_title`. Bracketed `上/下` markers are stripped first
/// (iteratively: a hostile page could chain many, and the marker nearest the
/// number wins), then `第N话`, plain integers (incl. concatenated ranges),
/// `N-M` parts/ranges, and finally fractional specials.
pub fn classify_short_title(raw: &str) -> ShortTitle {
    let (head, part) = strip_part_markers(raw);
    // A marker only refines a plain number; if the number already carries its
    // own part (`479-1（下）`) that part wins.
    match (classify_number(head), part) {
        (ShortTitle::Base(n), Some(p)) => ShortTitle::Part(n, p),
        (other, _) => other,
    }
}

/// Kuaikan titles read `第N话 <own title>（上）/（下）`: the number comes first,
/// the part marker sits at the end of the chapter's own title text.
pub fn classify_kuaikan_title(raw: &str) -> ShortTitle {
    let (head, part) = strip_part_markers(raw);
    if let Some((n, _own_title)) = parse_cn_chapter_prefix(head) {
        return match part {
            Some(p) => ShortTitle::Part(n, p),
            None => ShortTitle::Base(n),
        };
    }
    // No `第N话` prefix: the generic classifier handles plain numbers, dash
    // parts and fractional specials (the markers are already stripped).
    match (classify_number(head), part) {
        (ShortTitle::Base(n), Some(p)) => ShortTitle::Part(n, p),
        (other, _) => other,
    }
}

/// Classify a title with every bracketed marker already stripped.
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

/// Oldest-first rows -> `ChapterMeta`. Numbering comes from the title, not the
/// row position. A split part is its own tickable row (`479-1` -> 47901,
/// `第533话（下）` -> 53302); the first sighting of a key is a main chapter,
/// every repeat a bonus duplicate. Duplicates are **kept** (as bonus rows),
/// never dropped.
pub fn rows_to_chapters(rows: Vec<EpisodeRow>) -> Vec<ChapterMeta> {
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
pub fn main_count(chapters: &[ChapterMeta]) -> usize {
    chapters
        .iter()
        .filter(|c| !c.is_bonus)
        .map(|c| c.number_x100 / 100)
        .max()
        .unwrap_or(0) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn classify_kuaikan_titles() {
        let c = classify_kuaikan_title;
        // `第N话 <own title>` — the own title text is ignored.
        assert_eq!(c("第1话 重生"), ShortTitle::Base(1));
        assert_eq!(c("第533话 圣帝之手"), ShortTitle::Base(533));
        // Leading whitespace (one fixture row carries it) is trimmed.
        assert_eq!(c(" 第386话 妖血祭（上）"), ShortTitle::Part(386, 1));
        // The part marker sits at the end of the own title text.
        assert_eq!(c("第533话 圣帝之手（下）"), ShortTitle::Part(533, 2));
        assert_eq!(c("第100话 大战(上)"), ShortTitle::Part(100, 1));
        // Specials are unnumbered.
        for raw in ["番外", "延更公告", "愚人节快乐~", "请 假 条"] {
            assert_eq!(c(raw), ShortTitle::Unnumbered, "{raw:?}");
        }
        // A plain number without the `第N话` prefix still classifies.
        assert_eq!(c("479-2"), ShortTitle::Part(479, 2));
    }

    #[test]
    fn rows_map_parts_and_duplicates() {
        let rows = vec![
            EpisodeRow {
                title: Some("第1话 A".into()),
                release_date: None,
                kind: ShortTitle::Base(1),
            },
            EpisodeRow {
                title: Some("第2话 B（上）".into()),
                release_date: None,
                kind: ShortTitle::Part(2, 1),
            },
            EpisodeRow {
                title: Some("第2话 B（下）".into()),
                release_date: None,
                kind: ShortTitle::Part(2, 2),
            },
            EpisodeRow {
                title: Some("番外".into()),
                release_date: None,
                kind: ShortTitle::Unnumbered,
            },
        ];
        let chapters = rows_to_chapters(rows);
        let keys: Vec<i32> = chapters.iter().map(|c| c.number_x100).collect();
        assert_eq!(keys, vec![100, 201, 202, 0]);
        let bonus: Vec<bool> = chapters.iter().map(|c| c.is_bonus).collect();
        assert_eq!(bonus, vec![false, false, false, true]);
        assert_eq!(chapters[1].label.as_deref(), Some("2-1"));
        assert_eq!(main_count(&chapters), 2);
    }

    #[test]
    fn main_count_zero_when_all_rows_are_bonus() {
        let rows = vec![
            EpisodeRow {
                title: None,
                release_date: None,
                kind: ShortTitle::Unnumbered,
            },
            EpisodeRow {
                title: None,
                release_date: None,
                kind: ShortTitle::Fractional(4950),
            },
        ];
        let chapters = rows_to_chapters(rows);
        assert_eq!(chapters.len(), 2);
        assert_eq!(main_count(&chapters), 0);
        assert!(chapters.iter().all(|c| c.is_bonus));
    }
}
