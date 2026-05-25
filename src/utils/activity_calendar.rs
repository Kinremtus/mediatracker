use chrono::{Datelike, Duration, NaiveDate, Utc};
use std::collections::HashMap;

const MONTHS_SHORT: &[&str] = &[
    "Янв", "Фев", "Мар", "Апр", "Май", "Июн", "Июл", "Авг", "Сен", "Окт", "Ноя", "Дек",
];

const DAY_LABELS: [&str; 7] = ["Пн", "", "Ср", "", "Пт", "", "Вс"];

pub struct ActivityCalendar {
    pub html: String,
    pub total_actions: i32,
    pub year: i32,
}

fn monday_of_week(date: NaiveDate) -> NaiveDate {
    date - Duration::days(date.weekday().num_days_from_monday() as i64)
}

fn sunday_of_week(date: NaiveDate) -> NaiveDate {
    date + Duration::days((6 - date.weekday().num_days_from_monday()) as i64)
}

fn format_date_ru(date: NaiveDate) -> String {
    let month = MONTHS_SHORT
        .get((date.month() as usize).saturating_sub(1))
        .copied()
        .unwrap_or("");
    format!("{} {} {}", date.day(), month, date.year())
}

fn level_class(level: i32) -> &'static str {
    match level.clamp(0, 4) {
        0 => "",
        1 => " level-1",
        2 => " level-2",
        3 => " level-3",
        _ => " level-4",
    }
}

/// GitHub-style activity heatmap for the current calendar year (ported from legacy ActivityGrid).
pub fn build_activity_calendar(counts: &HashMap<NaiveDate, i32>) -> ActivityCalendar {
    let year = Utc::now().year();
    let start_of_year = NaiveDate::from_ymd_opt(year, 1, 1).expect("valid year start");
    let end_of_year = NaiveDate::from_ymd_opt(year, 12, 31).expect("valid year end");

    let grid_start = monday_of_week(start_of_year);
    let grid_end = sunday_of_week(end_of_year);

    let mut weeks: Vec<Vec<Option<(NaiveDate, i32)>>> = Vec::new();
    let mut current_week: Vec<Option<(NaiveDate, i32)>> = Vec::new();

    let mut d = grid_start;
    while d <= grid_end {
        if d.year() != year {
            current_week.push(None);
        } else {
            let count = counts.get(&d).copied().unwrap_or(0);
            let level = count.min(4);
            current_week.push(Some((d, level)));
        }

        if current_week.len() == 7 {
            weeks.push(current_week);
            current_week = Vec::new();
        }

        d += Duration::days(1);
    }

    let total_actions: i32 = counts.values().sum();

    let num_weeks = weeks.len();
    let mut html = String::with_capacity(num_weeks * 7 * 80 + 512);

    // All 12 months always visible (aligned to week columns where the 1st falls)
    html.push_str(&format!(
        r#"<div class="activity-months-row" style="grid-template-columns: auto repeat({num_weeks}, minmax(0, 1fr));"><div class="calendar-corner"></div><div class="activity-months-track" style="grid-column: 2 / -1; grid-template-columns: repeat({num_weeks}, minmax(0, 1fr));">"#
    ));
    for month in 1..=12 {
        let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
        let week_idx = weeks.iter().position(|week| {
            week.iter()
                .any(|day| matches!(day, Some((date, _)) if *date == first))
        });
        if let Some(idx) = week_idx {
            let label = MONTHS_SHORT[(month - 1) as usize];
            html.push_str(&format!(
                r#"<span class="calendar-month-label" style="grid-column-start:{}">{label}</span>"#,
                idx + 1
            ));
        }
    }
    html.push_str("</div></div>");

    html.push_str(&format!(
        "<div class=\"activity-calendar-grid\" style=\"grid-template-columns: auto repeat({}, minmax(0, 1fr));\">",
        num_weeks
    ));

    // Day rows (Mon–Sun)
    for day_idx in 0..7 {
        html.push_str(&format!(
            "<div class=\"calendar-day-label\">{}</div>",
            DAY_LABELS[day_idx]
        ));

        for (week_idx, week) in weeks.iter().enumerate() {
            match &week[day_idx] {
                None => {
                    html.push_str(&format!(
                        "<div class=\"calendar-day-empty\" data-week=\"{week_idx}\" data-day=\"{day_idx}\"></div>"
                    ));
                }
                Some((date, level)) => {
                    let count = counts.get(date).copied().unwrap_or(0);
                    let title = format!(
                        "{} действий — {}",
                        count,
                        format_date_ru(*date)
                    );
                    html.push_str(&format!(
                        "<div class=\"calendar-day{}\" title=\"{}\" data-week=\"{week_idx}\" data-day=\"{day_idx}\"></div>",
                        level_class(*level),
                        html_escape(&title)
                    ));
                }
            }
        }
    }

    html.push_str("</div>");

    // Legend
    html.push_str(
        r#"<div class="activity-calendar-legend"><span>Меньше</span><div class="legend-cells"><div class="calendar-day"></div><div class="calendar-day level-1"></div><div class="calendar-day level-2"></div><div class="calendar-day level-3"></div><div class="calendar-day level-4"></div></div><span>Больше</span></div>"#,
    );

    ActivityCalendar {
        html,
        total_actions,
        year,
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_year_grid_with_labels() {
        let mut counts = HashMap::new();
        let day = NaiveDate::from_ymd_opt(2026, 5, 20).unwrap();
        counts.insert(day, 3);

        let cal = build_activity_calendar(&counts);
        for month in MONTHS_SHORT {
            assert!(
                cal.html.contains(month),
                "missing month label: {month}"
            );
        }
        assert!(cal.html.contains("level-3"));
        assert_eq!(cal.total_actions, 3);
    }
}
