use std::collections::BTreeMap;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseEntry {
    pub provider: String,
    pub external_id: String,
    pub title: String,
    pub poster_url: Option<String>,
    /// 0 means "no season information" (Shikimori rows, legacy rows).
    pub season_number: i32,
    pub episode_number: i32,
    pub air_date: DateTime<Utc>,
}

impl ReleaseEntry {
    pub fn day(&self) -> u32 {
        self.air_date.format("%d").to_string().parse().unwrap_or(0)
    }

    pub fn month(&self) -> String {
        self.air_date.format("%b").to_string()
    }

    pub fn poster_or_placeholder(&self) -> &str {
        self.poster_url
            .as_deref()
            .unwrap_or("/static/images/placeholders/poster.svg")
    }

    /// `"S2 E5"` when the season is known, otherwise `"E12"`.
    pub fn episode_label(&self) -> String {
        if self.season_number > 0 {
            format!("S{} E{}", self.season_number, self.episode_number)
        } else {
            format!("E{}", self.episode_number)
        }
    }
}

#[derive(Debug, Clone)]
pub struct CalendarDay {
    pub date: NaiveDate,
    pub day_num: u32,
    pub is_current_month: bool,
    pub is_today: bool,
    pub releases: Vec<ReleaseEntry>,
}

/// Short Russian genitive month names for week labels ("5 — 11 окт").
const MONTHS_RU_GEN: [&str; 12] = [
    "янв", "фев", "мар", "апр", "мая", "июн", "июл", "авг", "сен", "окт", "ноя", "дек",
];

fn month_ru_gen(month: u32) -> &'static str {
    MONTHS_RU_GEN
        .get((month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("")
}

/// What a calendar entry represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CalendarEventKind {
    /// An episode of a tracked series/anime.
    Episode {
        season_number: i32,
        episode_number: i32,
    },
    /// A non-episodic premiere (movie, game, book, ...) or a series with no
    /// episode row, sourced from `media_items.aired_from`.
    Premiere,
}

/// One card in the calendar feed.
#[derive(Debug, Clone, Serialize)]
pub struct CalendarEvent {
    pub provider: String,
    pub external_id: String,
    pub media_type: String,
    pub title: String,
    pub poster_url: Option<String>,
    /// `None` for year-section entries ("whenever in {year}").
    pub date: Option<NaiveDate>,
    pub kind: CalendarEventKind,
    /// Family key from `services::search_families`.
    pub family: &'static str,
    /// Cross-source identity used for dedup (never rendered).
    pub canonical_key: String,
    pub is_past: bool,
}

impl CalendarEvent {
    /// `"S2 E5"`, `"E12"` or `"Премьера"`.
    pub fn label(&self) -> String {
        match self.kind {
            CalendarEventKind::Episode {
                season_number,
                episode_number,
            } => {
                if season_number > 0 {
                    format!("S{} E{}", season_number, episode_number)
                } else {
                    format!("E{}", episode_number)
                }
            }
            CalendarEventKind::Premiere => "Премьера".to_string(),
        }
    }

    pub fn is_episode(&self) -> bool {
        matches!(self.kind, CalendarEventKind::Episode { .. })
    }

    pub fn is_past(&self) -> bool {
        self.is_past
    }

    pub fn poster_or_placeholder(&self) -> &str {
        self.poster_url
            .as_deref()
            .unwrap_or("/static/images/placeholders/poster.svg")
    }

    pub fn has_date(&self) -> bool {
        self.date.is_some()
    }

    pub fn day(&self) -> u32 {
        self.date.map(|d| d.day()).unwrap_or(0)
    }

    /// Short Russian date like `"5 окт"` (empty for year-section entries).
    pub fn date_ru(&self) -> String {
        match self.date {
            Some(d) => format!("{} {}", d.day(), month_ru_gen(d.month())),
            None => String::new(),
        }
    }
}

/// Canonical cross-source identity: `shikimori:{id}` when a Shikimori id is
/// known, otherwise `{provider}:{external_id}`.
pub fn canonical_key(shikimori_id: Option<i64>, provider: &str, external_id: &str) -> String {
    match shikimori_id {
        Some(id) => format!("shikimori:{id}"),
        None => format!("{provider}:{external_id}"),
    }
}

/// Merge episode and premiere events. A premiere on the same canonical key and
/// date as any episode is dropped ("episode wins"), so "Премьера 5 окт" does
/// not duplicate "S1 E1 · 5 окт". Episodes are deduped against each other too
/// (the widened SELECT can return the same episode once per tracking source).
pub fn merge_and_dedup(
    episodes: Vec<CalendarEvent>,
    premieres: Vec<CalendarEvent>,
) -> Vec<CalendarEvent> {
    let mut seen: std::collections::HashSet<(String, Option<NaiveDate>)> =
        std::collections::HashSet::new();
    let mut merged: Vec<CalendarEvent> = Vec::new();

    for event in episodes {
        if seen.insert((event.canonical_key.clone(), event.date)) {
            merged.push(event);
        }
    }
    for event in premieres {
        if seen.insert((event.canonical_key.clone(), event.date)) {
            merged.push(event);
        }
    }

    merged.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.label().cmp(&b.label()))
    });
    merged
}

/// Keep only events of `family` (`""` == all).
pub fn filter_by_family(events: &[CalendarEvent], family: &str) -> Vec<CalendarEvent> {
    events
        .iter()
        .filter(|e| family.is_empty() || e.family == family)
        .cloned()
        .collect()
}

/// One ISO week (Monday start) of the month.
#[derive(Debug, Clone)]
pub struct WeekGroup {
    pub label: String,
    pub events: Vec<CalendarEvent>,
}

/// Group dated events into ISO weeks. Only days of the requested month are ever
/// passed in (the SQL is bounded), so weeks are naturally clamped to the month.
#[allow(dead_code)]
pub fn group_into_weeks(events: Vec<CalendarEvent>) -> Vec<WeekGroup> {
    let mut buckets: BTreeMap<NaiveDate, Vec<CalendarEvent>> = BTreeMap::new();
    for event in events {
        let Some(date) = event.date else { continue };
        let monday = date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64);
        buckets.entry(monday).or_default().push(event);
    }

    buckets
        .into_iter()
        .map(|(monday, mut bucket)| {
            bucket.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.title.cmp(&b.title)));
            let end = bucket.iter().filter_map(|e| e.date).max().unwrap_or(monday);
            WeekGroup {
                label: week_label(monday, end),
                events: bucket,
            }
        })
        .collect()
}

fn week_label(start: NaiveDate, end: NaiveDate) -> String {
    if start == end {
        format!("{} {}", start.day(), month_ru_gen(start.month()))
    } else if start.month() == end.month() {
        format!(
            "{} — {} {}",
            start.day(),
            end.day(),
            month_ru_gen(end.month())
        )
    } else {
        format!(
            "{} {} — {} {}",
            start.day(),
            month_ru_gen(start.month()),
            end.day(),
            month_ru_gen(end.month())
        )
    }
}

/// A family filter tab with its precomputed link and whole-month count.
#[derive(Debug, Clone)]
pub struct FamilyTab {
    pub key: &'static str,
    pub label: String,
    pub count: usize,
    pub href: String,
    pub active: bool,
}

/// Everything the calendar page/partial needs.
#[derive(Debug, Clone, Default)]
pub struct CalendarMonthView {
    pub weeks: Vec<WeekGroup>,
    pub year_section: Vec<CalendarEvent>,
    pub year_section_title: String,
    pub family_tabs: Vec<FamilyTab>,
    pub is_empty: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(season: i32, episode: i32) -> ReleaseEntry {
        ReleaseEntry {
            provider: "tmdb".to_string(),
            external_id: "1".to_string(),
            title: "Show".to_string(),
            poster_url: None,
            season_number: season,
            episode_number: episode,
            air_date: Utc::now(),
        }
    }

    #[test]
    fn episode_label_uses_season_when_positive() {
        assert_eq!(entry(2, 5).episode_label(), "S2 E5");
    }

    #[test]
    fn episode_label_falls_back_to_episode_only() {
        assert_eq!(entry(0, 12).episode_label(), "E12");
    }

    fn event(
        kind: CalendarEventKind,
        title: &str,
        date: Option<NaiveDate>,
        family: &'static str,
        key: &str,
    ) -> CalendarEvent {
        CalendarEvent {
            provider: "tmdb".to_string(),
            external_id: "1".to_string(),
            media_type: "series".to_string(),
            title: title.to_string(),
            poster_url: None,
            date,
            kind,
            family,
            canonical_key: key.to_string(),
            is_past: false,
        }
    }

    #[test]
    fn canonical_key_prefers_shikimori_id() {
        assert_eq!(canonical_key(Some(21), "mal", "mal-1"), "shikimori:21");
        assert_eq!(canonical_key(None, "tmdb", "99"), "tmdb:99");
    }

    #[test]
    fn merge_dedup_prefers_episode_over_premiere_same_day() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let ep = event(
            CalendarEventKind::Episode {
                season_number: 1,
                episode_number: 1,
            },
            "Anime",
            Some(day),
            "anime",
            "shikimori:21",
        );
        let prem = event(
            CalendarEventKind::Premiere,
            "Anime",
            Some(day),
            "anime",
            "shikimori:21",
        );
        let merged = merge_and_dedup(vec![ep], vec![prem]);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].is_episode());
    }

    #[test]
    fn merge_dedup_keeps_premiere_without_episode() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let prem = event(
            CalendarEventKind::Premiere,
            "Game",
            Some(day),
            "games",
            "rawg:g1",
        );
        let merged = merge_and_dedup(Vec::new(), vec![prem]);
        assert_eq!(merged.len(), 1);
        assert!(!merged[0].is_episode());
    }

    #[test]
    fn filter_by_family_selects_matching_and_all() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        let a = event(
            CalendarEventKind::Premiere,
            "A",
            Some(day),
            "games",
            "rawg:a",
        );
        let b = event(
            CalendarEventKind::Premiere,
            "B",
            Some(day),
            "movies",
            "tmdb:b",
        );
        let all = vec![a.clone(), b.clone()];
        assert_eq!(filter_by_family(&all, "").len(), 2);
        assert_eq!(filter_by_family(&all, "games").len(), 1);
        assert_eq!(filter_by_family(&all, "games")[0].title, "A");
        assert_eq!(filter_by_family(&all, "books").len(), 0);
    }

    #[test]
    fn group_into_weeks_orders_and_buckets_by_iso_week() {
        // 2026-10-05 and 2026-10-07 are in the same ISO week (Mon..Sun).
        let a = event(
            CalendarEventKind::Premiere,
            "A",
            Some(NaiveDate::from_ymd_opt(2026, 10, 7).unwrap()),
            "games",
            "rawg:a",
        );
        let b = event(
            CalendarEventKind::Premiere,
            "B",
            Some(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()),
            "games",
            "rawg:b",
        );
        let weeks = group_into_weeks(vec![a, b]);
        assert_eq!(weeks.len(), 1);
        assert_eq!(weeks[0].events.len(), 2);
        assert_eq!(weeks[0].events[0].title, "B"); // earliest first
    }

    #[test]
    fn group_into_weeks_skips_undated_and_splits_weeks() {
        let dated = event(
            CalendarEventKind::Premiere,
            "A",
            Some(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()),
            "games",
            "rawg:a",
        );
        let undated = event(CalendarEventKind::Premiere, "X", None, "games", "rawg:x");
        let later = event(
            CalendarEventKind::Premiere,
            "B",
            Some(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()),
            "games",
            "rawg:b",
        );
        let weeks = group_into_weeks(vec![dated, undated, later]);
        assert_eq!(weeks.len(), 2);
        assert_eq!(weeks.iter().map(|w| w.events.len()).sum::<usize>(), 2);
    }

    #[test]
    fn week_label_same_day_and_range() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
        assert_eq!(week_label(d, d), "5 окт");
        let end = NaiveDate::from_ymd_opt(2026, 10, 11).unwrap();
        assert_eq!(week_label(d, end), "5 — 11 окт");
        let sep = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let oct = NaiveDate::from_ymd_opt(2026, 10, 4).unwrap();
        assert_eq!(week_label(sep, oct), "29 сен — 4 окт");
    }

    #[test]
    fn event_label_matches_kind() {
        let ep = event(
            CalendarEventKind::Episode {
                season_number: 2,
                episode_number: 5,
            },
            "S",
            None,
            "series",
            "tmdb:1",
        );
        assert_eq!(ep.label(), "S2 E5");
        let shiki = event(
            CalendarEventKind::Episode {
                season_number: 0,
                episode_number: 12,
            },
            "S",
            None,
            "anime",
            "shikimori:1",
        );
        assert_eq!(shiki.label(), "E12");
        let prem = event(CalendarEventKind::Premiere, "P", None, "games", "rawg:1");
        assert_eq!(prem.label(), "Премьера");
    }
}
