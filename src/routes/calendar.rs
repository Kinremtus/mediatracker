use askama::Template;
use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::{Html, IntoResponse, Response},
};
use chrono::{Datelike, Duration, NaiveDate, Utc};
use serde::Deserialize;

use super::home::{SidebarStats, get_sidebar_stats};
use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::schedule::{CalendarEvent, CalendarMonthView, FamilyTab, WeekGroup};

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
    month_name: String,
    family_tabs: Vec<FamilyTab>,
    weeks: Vec<WeekGroup>,
    year_section: Vec<CalendarEvent>,
    year_section_title: String,
    is_empty: bool,
    prev_month_url: String,
    next_month_url: String,
}

/// Rendered alone for `HX-Request` swaps into `#calendar-content`.
#[derive(Template)]
#[template(path = "partials/calendar_weeks.html")]
struct CalendarWeeksPartial {
    family_tabs: Vec<FamilyTab>,
    weeks: Vec<WeekGroup>,
    year_section: Vec<CalendarEvent>,
    year_section_title: String,
    is_empty: bool,
}

#[derive(Deserialize)]
pub struct CalendarQuery {
    year: Option<i32>,
    month: Option<u32>,
    family: Option<String>,
}

pub async fn get_calendar(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<CalendarQuery>,
    headers: HeaderMap,
) -> Response {
    let now = Utc::now();
    let year = params.year.unwrap_or_else(|| now.year());
    let month = params.month.unwrap_or_else(|| now.month());

    // Reject nonsense month/year before any date math: `?month=13` used to panic
    // inside `NaiveDate::from_ymd_opt(..).expect(..)` (500 / DoS).
    if month_bounds(year, month).is_none() {
        return super::bad_request("calendar: year/month out of range");
    }

    // Unknown family is ignored -> "Все".
    let requested_family = params.family.unwrap_or_default();
    let family = if requested_family.is_empty()
        || crate::services::search_families::family(&requested_family).is_some()
    {
        requested_family
    } else {
        String::new()
    };

    let is_htmx = headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == "true")
        .unwrap_or(false);

    let _ = state.release_schedule.ensure_fresh(&state.shikimori).await;

    let view = match state
        .release_schedule
        .get_calendar_month(user.id, year, month, &family)
        .await
    {
        Ok(view) => view,
        Err(e) => {
            // Degrade gracefully: an empty feed rather than a 500.
            tracing::error!("Failed to load calendar: {}", e);
            CalendarMonthView::default()
        }
    };

    if is_htmx {
        let rendered = CalendarWeeksPartial {
            family_tabs: view.family_tabs,
            weeks: view.weeks,
            year_section: view.year_section,
            year_section_title: view.year_section_title,
            is_empty: view.is_empty,
        }
        .render();
        return match rendered {
            Ok(html) => Html(html).into_response(),
            Err(e) => super::internal_error("calendar: partial render", e),
        };
    }

    let stats = get_sidebar_stats(&state, &user).await;

    let month_name = MONTHS_RU
        .get((month as usize).saturating_sub(1))
        .copied()
        .unwrap_or("")
        .to_string();

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
    let family_qs = if family.is_empty() {
        String::new()
    } else {
        format!("&family={}", family)
    };
    let prev_month_url = format!(
        "/calendar?year={}&month={}{}",
        prev_year, prev_month, family_qs
    );
    let next_month_url = format!(
        "/calendar?year={}&month={}{}",
        next_year, next_month, family_qs
    );

    let rendered = CalendarTemplate {
        username: user.username,
        role: user.role,
        stats,
        active_page: "calendar".to_string(),
        current_status: String::new(),
        year,
        month_name,
        family_tabs: view.family_tabs,
        weeks: view.weeks,
        year_section: view.year_section,
        year_section_title: view.year_section_title,
        is_empty: view.is_empty,
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
