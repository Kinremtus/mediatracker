use askama::Template;
use axum::{
    extract::State,
    response::Html,
};
use askama::filters::Safe;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::stats::{StatsOverview, StatusCountDisplay, TitleProgress};
use super::home::SidebarStats;

fn translate_status(status: &str) -> String {
    match status {
        "watching" => "Смотрю".to_string(),
        "reading" => "Читаю".to_string(),
        "completed" => "Просмотрено".to_string(),
        "planned" => "Запланировано".to_string(),
        "dropped" => "Брошено".to_string(),
        _ => status.to_string(),
    }
}

#[derive(Template)]
#[template(path = "stats.html")]
struct StatsTemplate {
    username: String,
    stats: SidebarStats,
    active_page: String,
    overview: StatsOverview,
    status_display: Vec<StatusCountDisplay>,
    activity_count: usize,
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
    let activity = state.stats.get_activity(user.id).await.unwrap_or_default();
    let progress = state.stats.get_title_progress(user.id).await.unwrap_or_default();
    let sidebar_stats = get_sidebar_stats(&state, user.id).await;

    let activity_count = activity.len();
    let progress_count = progress.len();

    // Calculate percentages for status counts
    for sc in &mut overview.status_counts {
        if overview.total_titles > 0 {
            sc.percentage = (sc.count as f64 / overview.total_titles as f64 * 100.0) as i32;
        }
    }

    // Translate status labels
    let status_display: Vec<StatusCountDisplay> = overview.status_counts.iter().map(|sc| StatusCountDisplay {
        status: sc.status.clone(),
        label: translate_status(&sc.status),
        count: sc.count,
        percentage: sc.percentage,
    }).collect();

    // Generate calendar HTML
    let mut calendar_html = String::from("<div class=\"calendar-grid\">");
    for i in 0..53 {
        calendar_html.push_str("<div class=\"calendar-week\">");
        for j in 0..7 {
            let idx = i * 7 + j;
            if idx < activity_count {
                let level = (idx % 4) + 1;
                calendar_html.push_str(&format!("<div class=\"calendar-day level-{}\"></div>", level));
            } else {
                calendar_html.push_str("<div class=\"calendar-day\"></div>");
            }
        }
        calendar_html.push_str("</div>");
    }
    calendar_html.push_str("</div>");

    StatsTemplate {
        username: user.username,
        stats: sidebar_stats,
        active_page: "stats".to_string(),
        overview,
        status_display,
        activity_count,
        progress,
        progress_count,
        calendar_html: Safe(calendar_html),
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
                "watching" => stats.watching += 1,
                "reading" => stats.reading += 1,
                "completed" => stats.completed += 1,
                "planned" => stats.planned += 1,
                "dropped" => stats.dropped += 1,
                _ => {}
            }
        }
    }
    stats
}
