use askama::Template;
use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, header::SET_COOKIE},
    response::{Html, IntoResponse, Redirect, Response},
};
use chrono::{Datelike, Utc};
use serde::Serialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::schedule::ReleaseEntry;
use crate::models::tracking_entry::TrackingEntryWithMedia;

#[derive(Template)]
#[template(path = "home.html")]
#[expect(dead_code)]
struct HomeTemplate {
    username: String,
    role: String,
    greeting: String,
    today: String,
    active_page: String,
    current_status: String,
    stats: SidebarStats,
    in_progress: Vec<TrackingEntryWithMedia>,
    upcoming_releases: Vec<ReleaseEntry>,
}

#[derive(Serialize, Clone, Default)]
pub struct SidebarStats {
    pub in_progress: i32,
    pub completed: i32,
    pub planned: i32,
    pub dropped: i32,
    pub role: String,
}

/// Single source of truth for the sidebar counters used by every
/// page. Building `SidebarStats` inline in each route used to mean
/// seven byte-identical copies that could drift apart.
pub async fn get_sidebar_stats(state: &AppState, user: &CurrentUser) -> SidebarStats {
    let (ip, cp, pp, dp) = state
        .tracking
        .get_status_counts(user.id)
        .await
        .unwrap_or_default();
    SidebarStats {
        in_progress: ip,
        completed: cp,
        planned: pp,
        dropped: dp,
        role: user.role.clone(),
    }
}

fn greeting() -> String {
    let hour = Utc::now()
        .format("%H")
        .to_string()
        .parse::<u32>()
        .unwrap_or(12);
    match hour {
        5..=11 => "Доброе утро".to_string(),
        12..=16 => "Добрый день".to_string(),
        17..=23 => "Добрый вечер".to_string(),
        _ => "Доброй ночи".to_string(),
    }
}

fn today_ru() -> String {
    let now = Utc::now();
    let months = [
        "января",
        "февраля",
        "марта",
        "апреля",
        "мая",
        "июня",
        "июля",
        "августа",
        "сентября",
        "октября",
        "ноября",
        "декабря",
    ];
    let month = months[(now.month() as usize).saturating_sub(1)];
    format!("Сегодня {} {}", now.day(), month)
}

pub async fn get_home(user: CurrentUser, State(state): State<AppState>) -> Response {
    let stats = get_sidebar_stats(&state, &user).await;

    // In-progress entries with progress
    let entries = match state
        .tracking
        .get_user_entries(user.id, Some("in_progress"), None, None)
        .await
    {
        Ok(entries) => entries,
        Err(e) => return super::internal_error("home: load in-progress entries", e),
    };
    let in_progress: Vec<TrackingEntryWithMedia> = entries.into_iter().take(6).collect();

    // Ensure fresh schedule data
    let _ = state.release_schedule.ensure_fresh(&state.shikimori).await;

    let upcoming = match state
        .release_schedule
        .get_upcoming_for_user(user.id, 7)
        .await
    {
        Ok(releases) => releases,
        Err(e) => {
            tracing::error!("Failed to load upcoming releases: {}", e);
            Vec::new()
        }
    };

    HomeTemplate {
        username: user.username,
        role: user.role.clone(),
        greeting: greeting(),
        today: today_ru(),
        active_page: "home".to_string(),
        current_status: String::new(),
        stats,
        in_progress,
        upcoming_releases: upcoming,
    }
    .render()
    .map(Html)
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "template render failed");
        Html(String::from("Internal Server Error"))
    })
    .into_response()
}

fn logout_redirect() -> Response {
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_static("session_id=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0"),
    );
    response
}

pub async fn post_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    _user: CurrentUser,
) -> Response {
    // Invalidate the session server-side so a captured cookie cannot be
    // replayed after logout. Clearing the cookie alone is not enough.
    let Some(token) = crate::utils::session_cookie(&headers) else {
        return logout_redirect();
    };
    if let Err(e) = state.auth.logout(&token).await {
        tracing::warn!("Failed to delete session during logout: {}", e);
    }
    logout_redirect()
}
