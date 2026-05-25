use askama::Template;
use axum::{
    extract::State,
    response::Html,
};
use askama::filters::Safe;

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::stats::{StatsOverview, TitleProgress};
use crate::utils::activity_calendar::build_activity_calendar;
use super::home::SidebarStats;

fn translate_status(status: &str) -> String {
    match status {
        "in_progress" => "В процессе".to_string(),
        "completed" => "Завершено".to_string(),
        "planned" => "Запланировано".to_string(),
        "dropped" => "Брошено".to_string(),
        _ => status.to_string(),
    }
}

fn translate_media_type(media_type: &str) -> String {
    match media_type {
        "anime" => "Аниме".to_string(),
        "manga" => "Манга".to_string(),
        "manhwa" => "Манхва".to_string(),
        "manhua" => "Маньхуа".to_string(),
        "novel" => "Новеллы".to_string(),
        "other-comics" => "Другие комиксы".to_string(),
        "movie" => "Фильмы".to_string(),
        "series" => "Сериалы".to_string(),
        "dramas" => "Дорамы".to_string(),
        "cartoons" => "Мультсериалы".to_string(),
        "animated-movies" => "Мультфильмы".to_string(),
        "game" => "Игры".to_string(),
        "book" => "Книги".to_string(),
        _ => media_type.to_string(),
    }
}

#[derive(Template)]
#[template(path = "stats.html")]
struct StatsTemplate {
    username: String,
    stats: SidebarStats,
    active_page: String,
    overview: StatsOverview,
    status_labels: Vec<(String, String, i32, i32)>,
    top_category_label: String,
    activity_total: i32,
    activity_year: i32,
    progress: Vec<TitleProgress>,
    progress_count: usize,
    calendar_html: Safe<String>,
    current_status: String,
}

pub async fn get_stats(
    user: CurrentUser,
    State(state): State<AppState>,
) -> Html<String> {
    let mut overview = state.stats.get_overview(user.id).await.unwrap_or_default();
    let activity_by_day: HashMap<NaiveDate, i32> = state
        .stats
        .get_activity_by_day(user.id)
        .await
        .unwrap_or_default();
    let progress = state.stats.get_title_progress(user.id).await.unwrap_or_default();
    let sidebar_stats = get_sidebar_stats(&state, user.id).await;

    let calendar = build_activity_calendar(&activity_by_day);
    let progress_count = progress.len();

    // Calculate percentages for status counts
    for sc in &mut overview.status_counts {
        if overview.total_titles > 0 {
            sc.percentage = (sc.count as f64 / overview.total_titles as f64 * 100.0) as i32;
        }
    }

    // Create status labels (status_key, label, count, percentage)
    let status_labels: Vec<(String, String, i32, i32)> = overview.status_counts.iter().map(|sc| {
        (sc.status.clone(), translate_status(&sc.status), sc.count, sc.percentage)
    }).collect();

    // Translate top category
    let top_category_label = overview.top_category.as_ref().map(|t| translate_media_type(t)).unwrap_or_else(|| "—".to_string());

    StatsTemplate {
        username: user.username,
        stats: sidebar_stats,
        active_page: "stats".to_string(),
        overview,
        status_labels,
        top_category_label,
        activity_total: calendar.total_actions,
        activity_year: calendar.year,
        progress,
        progress_count,
        calendar_html: Safe(calendar.html),
        current_status: String::new(),
    }
    .render()
    .unwrap()
    .into()
}

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let mut stats = SidebarStats::default();
    if let Ok(entries) = state.tracking.get_user_entries(user_id, None, None).await {
        for e in entries {
            match e.entry.status.as_str() {
                "in_progress" => stats.in_progress += 1,
                "completed" => stats.completed += 1,
                "planned" => stats.planned += 1,
                "dropped" => stats.dropped += 1,
                _ => {}
            }
        }
    }
    stats
}
