use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use chrono::{NaiveDate, Datelike, Duration, Utc};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::schedule::{CalendarDay, ReleaseEntry};
use super::home::SidebarStats;

const MONTHS_RU: &[&str] = &[
    "Январь", "Февраль", "Март", "Апрель", "Май", "Июнь",
    "Июль", "Август", "Сентябрь", "Окторябрь", "Ноябрь", "Декабрь",
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
) -> Html<String> {
    let now = Utc::now();
    let year = params.year.unwrap_or_else(|| now.year());
    let month = params.month.unwrap_or_else(|| now.month());

    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user.id).await.unwrap_or_default();
    let stats = SidebarStats {
        in_progress: ip,
        completed: cp,
        planned: pp,
        dropped: dp,
        role: user.role.clone(),
    };

    let _ = state.release_schedule.ensure_fresh(&state.shikimori).await;

    let month_name = MONTHS_RU
        .get((month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("")
        .to_string();

    // Calculate first/last day of month
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid date");
    let last = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("valid date")
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).expect("valid date")
    } - Duration::days(1);

    // Start from Monday of the week containing the 1st
    let start = first - Duration::days(first.weekday().num_days_from_monday() as i64);
    let end = last + Duration::days((6 - last.weekday().num_days_from_monday()) as i64);

    // Fetch releases for the range
    let from = chrono::NaiveDateTime::new(start, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()).and_utc();
    let to = chrono::NaiveDateTime::new(end + Duration::days(1), chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()).and_utc();

    let releases = state.release_schedule.get_by_date_range(user.id, from, to).await.unwrap_or_default();

    // Build release map: date -> Vec<ReleaseEntry>
    let mut release_map: std::collections::HashMap<NaiveDate, Vec<ReleaseEntry>> = std::collections::HashMap::new();
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

    CalendarTemplate {
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
    .render()
    .unwrap()
    .into()
}
