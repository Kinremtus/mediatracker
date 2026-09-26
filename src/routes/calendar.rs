use askama::Template;
use axum::{
    extract::{Query, State},
    response::{Html, IntoResponse},
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::Deserialize;

use super::home::{SidebarStats, get_sidebar_stats};
use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::schedule::{CalendarDay, ReleaseEntry};

const MONTHS_RU: &[&str] = &[
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];

#[derive(Template)]
#[template(path = "calendar.html")]
#[expect(dead_code)]
struct CalendarTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    current_status: String,
    year: i32,
    month: u32,
    month_name: String,
    weeks: Vec<Vec<CalendarDay>>,
    prev_month_url: String,
    next_month_url: String,
}

#[derive(Deserialize)]
pub struct CalendarQuery {
    year: Option<i32>,
    month: Option<u32>,
}

pub async fn get_calendar(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<CalendarQuery>,
) -> axum::response::Response {
    let now = Utc::now();
    let year = params.year.unwrap_or_else(|| now.year());
    let month = params.month.unwrap_or_else(|| now.month());

    // Reject nonsense month/year before any date math: `?month=13` used to panic
    // inside `NaiveDate::from_ymd_opt(..).expect(..)` (500 / DoS).
    let Some((first, last)) = month_bounds(year, month) else {
        return super::bad_request("calendar: year/month out of range");
    };

    let stats = get_sidebar_stats(&state, &user).await;

    let _ = state.release_schedule.ensure_fresh(&state.shikimori).await;

    let month_name = MONTHS_RU
        .get((month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("")
        .to_string();

    // Start from Monday of the week containing the 1st
    let start = first - Duration::days(first.weekday().num_days_from_monday() as i64);
    let end = last + Duration::days((6 - last.weekday().num_days_from_monday()) as i64);

    // Fetch releases for the range
    let from = chrono::NaiveDateTime::new(start, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap())
        .and_utc();
    let to = chrono::NaiveDateTime::new(
        end + Duration::days(1),
        chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
    )
    .and_utc();

    let releases = match state
        .release_schedule
        .get_by_date_range(user.id, from, to)
        .await
    {
        Ok(releases) => releases,
        Err(e) => {
            tracing::error!("Failed to load calendar releases: {}", e);
            Vec::new()
        }
    };

    // Build release map: date -> Vec<ReleaseEntry>
    let mut release_map: std::collections::HashMap<NaiveDate, Vec<ReleaseEntry>> =
        std::collections::HashMap::new();
    for r in releases {
        let date = r.air_date.date_naive();
        release_map.entry(date).or_default().push(r);
    }

    let today = Utc::now().date_naive();

    let mut weeks: Vec<Vec<CalendarDay>> = Vec::new();
    let mut current_week: Vec<CalendarDay> = Vec::new();
    let mut d = start;
    while d <= end {
        let day = CalendarDay {
            date: d,
            day_num: d.day(),
            is_current_month: d.month() == month,
            is_today: d == today,
            releases: release_map.remove(&d).unwrap_or_default(),
        };
        current_week.push(day);

        if current_week.len() == 7 {
            weeks.push(current_week);
            current_week = Vec::new();
        }

        d += Duration::days(1);
    }
    if !current_week.is_empty() {
        weeks.push(current_week);
    }

    // Prev/next month URLs
    let (prev_year, prev_month) = if month == 1 {
        (year - 1, 12)
    } else {
        (year, month - 1)
    };
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };

    let prev_month_url = format!("/calendar?year={}&month={}", prev_year, prev_month);
    let next_month_url = format!("/calendar?year={}&month={}", next_year, next_month);

    let rendered = CalendarTemplate {
        username: user.username,
        role: user.role,
        stats,
        active_page: "calendar".to_string(),
        current_status: String::new(),
        year,
        month,
        month_name,
        weeks,
        prev_month_url,
        next_month_url,
    }
    .render();
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(e) => super::internal_error("calendar: template render", e),
    }
}

/// Inclusive first/last day of `year`/`month`, or `None` when the request is
/// out of range so the handler can answer 400 instead of panicking.
fn month_bounds(year: i32, month: u32) -> Option<(NaiveDate, NaiveDate)> {
    if !(1..=12).contains(&month) || !(1900..=2999).contains(&year) {
        return None;
    }
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let last = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    } - Duration::days(1);
    Some((first, last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_bounds_accepts_december() {
        let (first, last) = month_bounds(2026, 12).expect("december is valid");
        assert_eq!(first, NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
        assert_eq!(last, NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
    }

    #[test]
    fn month_bounds_handles_leap_february() {
        let (_, last) = month_bounds(2028, 2).expect("february 2028 is valid");
        assert_eq!(last, NaiveDate::from_ymd_opt(2028, 2, 29).unwrap());
    }

    #[test]
    fn month_bounds_rejects_out_of_range_input() {
        assert!(month_bounds(2026, 13).is_none());
        assert!(month_bounds(2026, 0).is_none());
        assert!(month_bounds(1800, 1).is_none());
        assert!(month_bounds(i32::MAX, 1).is_none());
    }
}
