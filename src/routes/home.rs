use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
    http::{header::SET_COOKIE, HeaderValue},
};
use chrono::{Datelike, Utc};
use serde::Serialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::schedule::ReleaseEntry;

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    username: String,
    greeting: String,
    today: String,
    active_page: String,
    current_status: String,
    stats: SidebarStats,
    in_progress: Vec<HomeMediaCard>,
    upcoming_releases: Vec<ReleaseEntry>,
}

#[derive(Serialize, Clone, Default)]
pub struct SidebarStats {
    pub in_progress: i32,
    pub completed: i32,
    pub planned: i32,
    pub dropped: i32,
}

pub struct HomeMediaCard {
    pub provider: String,
    pub external_id: String,
    pub title: String,
    pub poster_url: String,
    pub progress_current: i32,
    pub progress_total: Option<i32>,
    pub progress_percent: u8,
}

fn greeting() -> String {
    let hour = Utc::now().format("%H").to_string().parse::<u32>().unwrap_or(12);
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
        "января", "февраля", "марта", "апреля", "мая", "июня",
        "июля", "августа", "сентября", "октября", "ноября", "декабря",
    ];
    let month = months[(now.month() as usize).saturating_sub(1)];
    format!("Сегодня {} {}", now.day(), month)
}

pub async fn get_home(
    user: CurrentUser,
    State(state): State<AppState>,
) -> Html<String> {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user.id).await.unwrap_or_default();
    let stats = SidebarStats {
        in_progress: ip,
        completed: cp,
        planned: pp,
        dropped: dp,
    };

    // In-progress entries with progress
    let entries = state.tracking.get_user_entries(user.id, Some("in_progress"), None).await.unwrap_or_default();
    let in_progress: Vec<HomeMediaCard> = entries
        .into_iter()
        .take(6)
        .map(|e| {
            let total = e.media.episodes;
            let current = e.entry.progress;
            let percent = total.map(|t| if t > 0 { (current * 100 / t).min(100) as u8 } else { 0 }).unwrap_or(0);
            HomeMediaCard {
                provider: e.media.provider,
                external_id: e.media.external_id,
                title: e.media.title_russian.as_deref().unwrap_or(&e.media.title).to_string(),
                poster_url: e.media.poster_url.unwrap_or_default(),
                progress_current: current,
                progress_total: total,
                progress_percent: percent,
            }
        })
        .collect();

    // Ensure fresh schedule data
    let _ = state.release_schedule.ensure_fresh(&state.shikimori).await;

    let upcoming = state.release_schedule.get_upcoming_for_user(user.id, 7).await.unwrap_or_default();

    HomeTemplate {
        username: user.username,
        greeting: greeting(),
        today: today_ru(),
        active_page: "home".to_string(),
        current_status: String::new(),
        stats,
        in_progress,
        upcoming_releases: upcoming,
    }
    .render()
    .unwrap()
    .into()
}

pub async fn post_logout(
    State(_state): State<AppState>,
    _user: CurrentUser,
) -> Response {
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str("session_id=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0").unwrap(),
    );
    response
}
