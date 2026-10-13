//! Shared chapter-title classification for the meta-capable official sources.
//!
//! Sources report chapter numbers as free-form titles: Chinese `281`,
//! `第533话`, `479-2`, `第533话（下）`, Korean `132화`, Japanese `第1話`/
//! `第9話-1`/`第2話前編`, Manga UP! `Chapter 25.5` and MangaPlus `#143`. The
//! classifiers here turn one title into a [`ShortTitle`]; [`rows_to_chapters`]
//! then maps an oldest-first row list onto [`ChapterMeta`] rows (the first
//! sighting of a key is a main chapter, every repeat a bonus duplicate) and
//! [`main_count`] derives the chapter count from the highest main base.
//! Sources that report no number in their titles at all (narou, kakuyomu,
//! qidian, naver, tappytoon) go through [`positional_rows`] instead: marker
//! titles become bonus rows and the rest are numbered `1..N` in order.
//!
//! Moved out of `official_bilibili` when kuaikan gained a fetcher: bilibili and
//! kuaikan share the `第N话` + `（上）/（下）` + `N-M` dialect, naver/kakao the
//! Korean `N화` one, comicwalker/alphapolis the Japanese `第N話`/`第N回` one and
//! mangaup the `Chapter N.M` one. The KR search-title normalization helpers
//! ([`strip_catalog_markers`], [`strip_kr_episode_suffix`]) live here too:
//! naver and ridibooks use them to reduce a hit title to the bare series name
//! so [`crate::services::official_title::titles_match`] can compare it.
//! [`starts_with_word`] backs the Latin side-content markers of the English
//! sources (tappytoon, webtoons).

use std::collections::HashSet;

use chrono::NaiveDate;

use crate::services::external::official_meta::ChapterMeta;

/// How one source title maps onto the chapter sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortTitle {
    /// A plain chapter number (`"281"`, `"第533话"`, `"132화"`).
    Base(i32),
    /// One split release of a chapter (`"479-1"`, `"533（下）"`): `base` plus a
    /// `part` in `1..=99`. Parts become separate tickable rows
    /// (`base * 100 + part`) but count as the single chapter `base`.
    Part(i32, i32),
    /// Fractional special (`"49.5"` -> 4950) — always a bonus row.
    Fractional(i32),
    /// Prologues, notices and placeholders (`"00"`, `"0화"`, `"프롤로그"`) —
    /// bonus rows keyed 0.
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
    with_part(classify_number(head), part)
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
    with_part(classify_number(head), part)
}

/// MangaPlus app rows carry `#NNN` names for numbered chapters; anything else
/// (`ex`) is a bonus row, whose own label lives in the row's `subTitle`.
pub fn classify_mangaplus_name(name: Option<&str>) -> ShortTitle {
    let Some(raw) = name else {
        return ShortTitle::Unnumbered;
    };
    let Some(digits) = raw.trim().strip_prefix('#') else {
        return ShortTitle::Unnumbered;
    };
    match digits.trim().parse::<i32>() {
        Ok(n) if n >= 1 => ShortTitle::Base(n),
        _ => ShortTitle::Unnumbered,
    }
}

/// Japanese manga episode titles: `第N話` / `第N回`, split releases
/// (`第11話前編①`, `第16話後編`, `第10話①`, `第9話-1`) and bonus/extra content
/// (`第0話`, プロローグ, 番外編, 特別編, 外伝, 読切, おまけ, 特典, 限定).
///
/// Split parts are keyed `N*100 + p` (see [`rows_to_chapters`]): a half is
/// `10`/`20` (`前編`/`後編`) plus a circled sub-part number, so
/// `第11話前編①` -> `11*100 + 11`. Titles without a `第N話/回` marker fall back
/// to the generic classifier (plain numbers, dash parts).
pub fn classify_jp_title(raw: &str) -> ShortTitle {
    let s = raw.trim();
    // A numbered title wins over a marker (`第5話 幕間` is chapter 5).
    if !has_chapter_number_prefix(s) && is_jp_bonus_title(s) {
        return ShortTitle::Unnumbered;
    }
    if let Some((n, rest)) = parse_jp_number(s) {
        return match jp_part_index(rest) {
            Some(p) => ShortTitle::Part(n, p),
            None => ShortTitle::Base(n),
        };
    }
    classify_number(s)
}

/// Japanese side-content markers (substring match, same trade-off as the KR
/// list): a rare main episode that mentions one is folded into bonuses rather
/// than lost. Shared kanji markers (`序章`/`幕間`/`終章`/`間話`/`閑話`) also
/// cover Chinese titles.
pub fn is_jp_bonus_title(title: &str) -> bool {
    [
        "プロローグ",
        "エピローグ",
        "番外編",
        "番外",
        "特別編",
        "外伝",
        "読切",
        "おまけ",
        "特典",
        "限定",
        "幕間",
        "幕间",
        "序章",
        "終章",
        "终章",
        "間話",
        "间话",
        "閑話",
        "闲话",
    ]
    .iter()
    .any(|marker| title.contains(marker))
}

/// True when the title starts with a numbered `第…<unit>` chapter marker
/// (Chinese numerals included: `第三十九章`, `第35话`, `第１２章`). The numeral
/// must be non-zero — `第0話`/`第0章` are prologues, exactly like in
/// [`parse_jp_number`]/[`parse_cn_chapter`]. A numbered title is a real chapter
/// even when the rest mentions side content (`第三十九章 尾声`), so the
/// bonus-marker checks must not override it.
pub fn has_chapter_number_prefix(raw: &str) -> bool {
    let Some(rest) = raw.trim().strip_prefix('第') else {
        return false;
    };
    let token = rest.split_whitespace().next().unwrap_or("");
    let mut numeral = false;
    let mut nonzero = false;
    let mut unit = false;
    for c in token.chars() {
        match c {
            '0' | '０' | '○' | '〇' | '零' => numeral = true,
            '1'..='9' | '１'..='９' => {
                numeral = true;
                nonzero = true;
            }
            '一' | '二' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百' | '千'
            | '万' | '两' | '壹' | '贰' | '叁' | '肆' | '伍' | '陆' | '柒' | '捌' | '玖' =>
            {
                numeral = true;
                nonzero = true;
            }
            '章' | '话' | '話' | '节' | '節' | '回' => unit = true,
            _ => {}
        }
    }
    numeral && nonzero && unit
}

/// Chinese sources mark side content in the title: `番外`/`外传` (side story),
/// `序章`/`楔子` (prologue), `后记`/`尾声` (afterword) and
/// `公告`/`上架`/`感言`/`请假` (notices). Such rows are bonus rows, never
/// numbered chapters.
pub fn is_cn_bonus_title(title: &str) -> bool {
    [
        "番外", "外传", "外傳", "序章", "楔子", "后记", "后記", "尾声", "尾聲", "公告", "上架",
        "感言", "请假", "請假",
    ]
    .iter()
    .any(|marker| title.contains(marker))
}

/// Bonus-row rule for the positional sources: a title matching a Japanese
/// side-content marker is a bonus row unless it also carries a chapter number
/// (`第三十九章 尾声` is a real chapter).
pub fn is_jp_positional_bonus(title: &str) -> bool {
    !has_chapter_number_prefix(title) && is_jp_bonus_title(title)
}

/// [`is_jp_positional_bonus`] with the Chinese marker set.
pub fn is_cn_positional_bonus(title: &str) -> bool {
    !has_chapter_number_prefix(title) && is_cn_bonus_title(title)
}

/// `第N話` / `第N回` -> `Some((N, tail))` for `N >= 1`; ASCII and full-width
/// digits are accepted (`第１回`). `第0話` yields `None` (a bonus row).
fn parse_jp_number(s: &str) -> Option<(i32, &str)> {
    let rest = s.strip_prefix('第')?;
    let mut end = 0usize;
    let mut value = 0i32;
    for (i, c) in rest.char_indices() {
        let digit = match c {
            '0'..='9' => (c as u8 - b'0') as i32,
            '０'..='９' => (c as u32 - '０' as u32) as i32,
            _ => break,
        };
        value = value.saturating_mul(10).saturating_add(digit);
        end = i + c.len_utf8();
    }
    if end == 0 || value < 1 {
        return None;
    }
    let after = &rest[end..];
    let tail = after
        .strip_prefix('話')
        .or_else(|| after.strip_prefix('话'))
        .or_else(|| after.strip_prefix('回'))?;
    Some((value, tail))
}

/// Split-part index for the text after `第N話`: `-M` dash (`第9話-1`), `前編`
/// (10) / `後編` (20), plus a circled sub-part number (`①` -> 1). `None` for a
/// plain chapter.
fn jp_part_index(rest: &str) -> Option<i32> {
    let t = rest.trim_start();
    if let Some(after) = t.strip_prefix('-') {
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(p) = digits.parse::<i32>()
            && (1..=99).contains(&p)
        {
            return Some(p);
        }
    }
    let mut part = 0i32;
    if t.contains("前編") {
        part += 10;
    } else if t.contains("後編") {
        part += 20;
    }
    part += circled_number(t).unwrap_or(0);
    (part > 0).then_some(part)
}

/// Circled digit `①`..`⑳` (U+2460..U+2473) -> `1..=20`.
fn circled_number(s: &str) -> Option<i32> {
    s.chars().find_map(|c| {
        let v = c as u32;
        (0x2460..=0x2473)
            .contains(&v)
            .then_some((v - 0x2460 + 1) as i32)
    })
}

/// Manga UP! episode titles: `Chapter N` and `Chapter N.M` split parts;
/// anything else (`Final`, `Special`, `Prologue`, `Bonus`, `Side Story`, …)
/// is unnumbered bonus content. A subtitle after the number is ignored.
pub fn classify_mangaup_title(raw: &str) -> ShortTitle {
    let Some(rest) = raw.trim().strip_prefix("Chapter ") else {
        return ShortTitle::Unnumbered;
    };
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let Ok(n) = digits.parse::<i32>() else {
        return ShortTitle::Unnumbered;
    };
    if n < 1 {
        return ShortTitle::Unnumbered;
    }
    let after = &rest[digits.len()..];
    if let Some(frac) = after.strip_prefix('.') {
        let part_digits: String = frac.chars().take_while(|c| c.is_ascii_digit()).collect();
        return match part_digits.parse::<i32>() {
            Ok(p) if (1..=99).contains(&p) => ShortTitle::Part(n, p),
            _ => ShortTitle::Unnumbered,
        };
    }
    ShortTitle::Base(n)
}

/// Apply a stripped part marker to a plain base number: `(Base(n), Some(p))`
/// becomes `Part(n, p)`; every other classification is returned unchanged.
fn with_part(kind: ShortTitle, part: Option<i32>) -> ShortTitle {
    match (kind, part) {
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

/// Korean sources mark side content in the episode title: `예고편` (preview),
/// `외전` (side story), `번외` (extra), `프롤로그` (prologue), `공지` (notice),
/// `휴재` (hiatus), `후기` (afterword). Such rows are bonus rows, never
/// numbered chapters.
///
/// Substring matching is deliberate (Naver mixes markers into longer titles,
/// e.g. `101화 (시즌2 마지막화)` is a main row while `1화 예고편` is a bonus); a
/// rare main episode whose own title happens to mention one of the markers is
/// folded into the bonus section rather than being lost.
pub fn is_kr_bonus_title(title: &str) -> bool {
    ["예고편", "외전", "번외", "프롤로그", "공지", "휴재", "후기"]
        .iter()
        .any(|marker| title.contains(marker))
}

/// `lower` starts with `marker` and the next character does not continue the
/// word: `extra 3` matches, `extraordinary` does not. A single trailing plural
/// `s` is accepted (`extras 3`), but `extraspecial` still fails. Used by sources
/// whose markers are Latin words (`official_tappytoon`, `official_webtoons`).
pub fn starts_with_word(lower: &str, marker: &str) -> bool {
    let Some(rest) = lower.strip_prefix(marker) else {
        return false;
    };
    let rest = rest.strip_prefix('s').unwrap_or(rest);
    !rest.chars().next().is_some_and(char::is_alphabetic)
}

/// The number in a trailing `<n>화` (episode) or `<n>권` (volume) suffix:
/// `132화` -> `Some(132)`, `3권` -> `Some(3)`, `0화` -> `Some(0)`; `None`
/// when the title does not end in a numeric marker.
pub fn parse_kr_chapter_number(raw: &str) -> Option<i32> {
    let trimmed = raw.trim();
    let stem = trimmed
        .strip_suffix('화')
        .or_else(|| trimmed.strip_suffix('권'))?;
    let digits: String = stem
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Kakao episode titles end with `<n>화` (`… 132화`) or `<n>권` (`… 3권`).
/// `0화` prologues and unnumbered notices (`… 프롤로그`) are bonus rows; a
/// numbered title wins even when it mentions side content, matching the
/// duplicate-number rule elsewhere.
pub fn classify_kakao_title(raw: &str) -> ShortTitle {
    match parse_kr_chapter_number(raw) {
        Some(n) if n >= 1 => ShortTitle::Base(n),
        _ => ShortTitle::Unnumbered,
    }
}

/// Catalog markers Korean stores append to a series title (`… [독점]`,
/// `… (완결)`). They are not part of the title and must not defeat matching.
const KR_CATALOG_MARKERS: &[&str] = &[
    "독점",
    "완결",
    "무료",
    "성인",
    "개정판",
    "단행본",
    "독점연재",
    "선연재",
    "무료웹툰",
    "최신화",
];

/// Strip trailing `[...]` / `(...)` segments whose inner text is a known
/// catalog marker: `인류 최대의 실수 19화 (완결)` -> `… 19화`,
/// `역대급 영지 설계사 [독점]` -> `역대급 영지 설계사`. Unknown bracketed text
/// (`나루토 [2부]`) is kept — it may be part of the real title.
pub fn strip_catalog_markers(raw: &str) -> &str {
    let mut s = raw.trim_end();
    loop {
        let (inner, rest) = if let Some(stripped) = s.strip_suffix(']') {
            match stripped.rfind('[') {
                Some(i) => (&stripped[i + 1..], stripped[..i].trim_end()),
                None => break,
            }
        } else if let Some(stripped) = s.strip_suffix(')') {
            match stripped.rfind('(') {
                Some(i) => (&stripped[i + 1..], stripped[..i].trim_end()),
                None => break,
            }
        } else {
            break;
        };
        if !KR_CATALOG_MARKERS.contains(&inner.trim()) {
            break;
        }
        s = rest;
    }
    s
}

/// Strip a trailing episode/volume suffix (`… 1화`, `… 18권`) from a search
/// hit so it can be compared with the series title. The digits must follow a
/// space (or start the string) — `제1화` is a chapter title, not a series
/// suffix.
pub fn strip_kr_episode_suffix(raw: &str) -> &str {
    let s = raw.trim_end();
    for unit in ['화', '권'] {
        let Some(stem) = s.strip_suffix(unit) else {
            continue;
        };
        let stem = stem.trim_end();
        let digits_start = stem
            .char_indices()
            .rev()
            .take_while(|(_, c)| c.is_ascii_digit())
            .last()
            .map(|(i, _)| i)
            .unwrap_or(stem.len());
        let digits = &stem[digits_start..];
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let before = &stem[..digits_start];
        if !before.is_empty() && !before.ends_with(' ') {
            continue;
        }
        return before.trim_end();
    }
    s
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

/// One raw row from a source that reports no chapter number in its titles:
/// title, date and whether the title marks it as side content.
pub struct RawRow {
    pub title: Option<String>,
    pub release_date: Option<NaiveDate>,
    pub is_bonus: bool,
}

/// Rows from a source that reports no chapter number in its titles (narou,
/// kakuyomu, qidian, naver): rows flagged `is_bonus` become unnumbered bonus
/// rows and every other row is numbered `1..N` in the given order, so a bonus
/// never consumes a chapter number. Numbering stops at the `i32` key ceiling
/// instead of saturating into duplicate keys.
pub fn positional_rows(rows: Vec<RawRow>) -> Vec<ChapterMeta> {
    let mut out = Vec::with_capacity(rows.len());
    let mut main = 0i32;
    for row in rows {
        let (number_x100, is_bonus) = if row.is_bonus {
            (0, true)
        } else {
            let Some(next) = main.checked_add(1) else {
                break;
            };
            let Some(key) = next.checked_mul(100) else {
                break;
            };
            main = next;
            (key, false)
        };
        out.push(ChapterMeta {
            number_x100,
            title: row.title,
            release_date: row.release_date,
            is_bonus,
            label: None,
        });
    }
    out
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
    fn positional_rows_number_only_mains() {
        let rows = vec![
            RawRow {
                title: Some("プロローグ".into()),
                release_date: None,
                is_bonus: true,
            },
            RawRow {
                title: Some("第一話".into()),
                release_date: None,
                is_bonus: false,
            },
            RawRow {
                title: Some("第二話".into()),
                release_date: None,
                is_bonus: false,
            },
        ];
        let chapters = positional_rows(rows);
        let keys: Vec<i32> = chapters.iter().map(|c| c.number_x100).collect();
        assert_eq!(keys, vec![0, 100, 200]);
        assert!(chapters[0].is_bonus);
        assert_eq!(main_count(&chapters), 2);
    }

    #[test]
    fn chapter_number_prefix_needs_a_nonzero_numeral() {
        for yes in [
            "第一章 绯红",
            "第35话",
            "第１２章",
            "第三十九章 尾声",
            "第5回",
            "第1話「タイトル」",
        ] {
            assert!(has_chapter_number_prefix(yes), "{yes}");
        }
        for no in [
            "第0話",
            "第0章",
            "第00話",
            "第０話",
            "第X章",
            "序章",
            "第 1 話",
            "プロローグ",
        ] {
            assert!(!has_chapter_number_prefix(no), "{no}");
        }
    }

    #[test]
    fn cn_bonus_markers_and_kr_volume_suffix() {
        for yes in [
            "番外 后篇",
            "上架感言",
            "请假条",
            "序章",
            "楔子",
            "完本感言",
        ] {
            assert!(is_cn_bonus_title(yes), "{yes}");
        }
        assert!(!is_cn_bonus_title("第一章 绯红"));
        assert_eq!(parse_kr_chapter_number("3권"), Some(3));
        assert_eq!(parse_kr_chapter_number("12화"), Some(12));
        assert_eq!(parse_kr_chapter_number("프롤로그"), None);
    }

    #[test]
    fn positional_bonus_respects_numbered_titles() {
        assert!(is_jp_positional_bonus("プロローグ"));
        assert!(is_jp_positional_bonus("幕間 閑話"));
        assert!(!is_jp_positional_bonus("第5話 幕間"));
        assert!(is_cn_positional_bonus("上架感言"));
        assert!(!is_cn_positional_bonus("第三十九章 尾声"));
    }

    #[test]
    fn numbered_titles_win_over_bonus_markers() {
        assert!(has_chapter_number_prefix("第三十九章 尾声"));
        assert!(has_chapter_number_prefix("第35话"));
        assert!(has_chapter_number_prefix("第１２章 番外"));
        assert!(!has_chapter_number_prefix("上架感言"));
        assert!(!has_chapter_number_prefix("12月1日《诡秘之主》新番外发布"));
        assert!(!has_chapter_number_prefix("プロローグ"));
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
    fn classify_kakao_titles() {
        let c = classify_kakao_title;
        assert_eq!(c("용사파티 대마법사의 환생 132화"), ShortTitle::Base(132));
        assert_eq!(c("튜토리얼이 너무 어렵다 1화"), ShortTitle::Base(1));
        // A numbered title wins even when it mentions side content.
        assert_eq!(c("외전 5화"), ShortTitle::Base(5));
        // `0화` prologues and unnumbered specials are bonus rows.
        assert_eq!(c("용사파티 대마법사의 환생 0화"), ShortTitle::Unnumbered);
        assert_eq!(c("튜토리얼이 너무 어렵다 프롤로그"), ShortTitle::Unnumbered);
        assert_eq!(c("외전"), ShortTitle::Unnumbered);
        assert_eq!(c(""), ShortTitle::Unnumbered);
        // The shared suffix helper also exposes the `0화` number for display.
        assert_eq!(parse_kr_chapter_number("132화"), Some(132));
        assert_eq!(parse_kr_chapter_number("0화"), Some(0));
        assert_eq!(parse_kr_chapter_number("프롤로그"), None);
    }

    #[test]
    fn kr_bonus_markers() {
        for title in [
            "예고편",
            "1화 예고편",
            "외전",
            "번외편",
            "프롤로그",
            "연재 공지",
            "휴재 안내",
            "후기",
            "작가 후기",
        ] {
            assert!(is_kr_bonus_title(title), "{title}");
        }
        for title in ["1화", "1부-1화", "101화 (시즌2 마지막화)", "132화"] {
            assert!(!is_kr_bonus_title(title), "{title}");
        }
    }

    #[test]
    fn strips_kr_catalog_markers_and_episode_suffix() {
        assert_eq!(
            strip_catalog_markers("인류 최대의 실수 19화 (완결)"),
            "인류 최대의 실수 19화"
        );
        assert_eq!(
            strip_catalog_markers("역대급 영지 설계사 [독점]"),
            "역대급 영지 설계사"
        );
        assert_eq!(
            strip_catalog_markers("역대급 영지 설계사 [독점][무료]"),
            "역대급 영지 설계사"
        );
        // Unknown brackets are kept — they may be part of the real title.
        assert_eq!(strip_catalog_markers("나루토 [2부]"), "나루토 [2부]");
        assert_eq!(
            strip_kr_episode_suffix("인류 최대의 실수 1화"),
            "인류 최대의 실수"
        );
        assert_eq!(
            strip_kr_episode_suffix("나 혼자만 레벨업 18권"),
            "나 혼자만 레벨업"
        );
        // No space before the number -> a chapter title, not a series suffix.
        assert_eq!(strip_kr_episode_suffix("제1화"), "제1화");
        assert_eq!(
            strip_kr_episode_suffix("인류 최대의 실수"),
            "인류 최대의 실수"
        );
    }

    #[test]
    fn classify_mangaplus_names() {
        let c = classify_mangaplus_name;
        assert_eq!(c(Some("#001")), ShortTitle::Base(1));
        assert_eq!(c(Some("#143")), ShortTitle::Base(143));
        assert_eq!(c(Some(" #7 ")), ShortTitle::Base(7));
        // Bonus rows and anything unnumbered.
        for raw in [Some("ex"), Some("#0"), Some("Chapter 1"), None] {
            assert_eq!(c(raw), ShortTitle::Unnumbered, "{raw:?}");
        }
    }

    #[test]
    fn classify_jp_titles() {
        let c = classify_jp_title;
        // Plain and full-width numbers.
        assert_eq!(c("第1話"), ShortTitle::Base(1));
        assert_eq!(c("第１２回"), ShortTitle::Base(12));
        assert_eq!(c("第16話前編"), ShortTitle::Part(16, 10));
        assert_eq!(c("第16話後編"), ShortTitle::Part(16, 20));
        // Circled sub-parts and the combined `前編①` form.
        assert_eq!(c("第10話①"), ShortTitle::Part(10, 1));
        assert_eq!(c("第10話④"), ShortTitle::Part(10, 4));
        assert_eq!(c("第11話前編①"), ShortTitle::Part(11, 11));
        assert_eq!(c("第11話後編③"), ShortTitle::Part(11, 23));
        // Dash split release.
        assert_eq!(c("第9話-1"), ShortTitle::Part(9, 1));
        // An own-title tail does not disturb the number.
        assert_eq!(c("第1話 オープニング・アゲイン"), ShortTitle::Base(1));
        // Prologue / extras are unnumbered bonuses.
        for raw in [
            "第0話",
            "第0回",
            "プロローグ",
            "エピローグ",
            "番外編",
            "特別編",
            "外伝",
        ] {
            assert_eq!(c(raw), ShortTitle::Unnumbered, "{raw:?}");
        }
        // No `第N話` marker: the generic fallback still handles plain numbers.
        assert_eq!(c("5"), ShortTitle::Base(5));
    }

    #[test]
    fn classify_mangaup_titles() {
        let c = classify_mangaup_title;
        assert_eq!(c("Chapter 1"), ShortTitle::Base(1));
        assert_eq!(c("Chapter 25"), ShortTitle::Base(25));
        // Split parts keep their fractional number.
        assert_eq!(c("Chapter 10.1"), ShortTitle::Part(10, 1));
        assert_eq!(c("Chapter 25.5"), ShortTitle::Part(25, 5));
        // A subtitle after the number is ignored.
        assert_eq!(c("Chapter 3: The Beginning"), ShortTitle::Base(3));
        // Bonus markers and unnumbered rows.
        for raw in [
            "Final",
            "Special",
            "Prologue",
            "Bonus",
            "Side Story",
            "Omake",
            "",
        ] {
            assert_eq!(c(raw), ShortTitle::Unnumbered, "{raw:?}");
        }
        assert_eq!(c("Chapter 0"), ShortTitle::Unnumbered);
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
