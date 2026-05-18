use askama::Template;
use axum::{
    extract::State,
    response::Html,
};
use askama::filters::Safe;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::stats::{StatsOverview, TitleProgress};

#[derive(Template)]
#[template(path = "stats.html")]
struct StatsTemplate {
    username: String,
    overview: StatsOverview,
    activity_count: usize,
    progress: Vec<TitleProgress>,
    progress_count: usize,
    calendar_html: Safe<String>,
}

pub async fn get_stats(
    user: CurrentUser,
    State(state): State<AppState>,
) -> Html<String> {
    let mut overview = state.stats.get_overview(user.id).await.unwrap_or_default();
    let activity = state.stats.get_activity(user.id).await.unwrap_or_default();
    let progress = state.stats.get_title_progress(user.id).await.unwrap_or_default();
    
    let activity_count = activity.len();
    let progress_count = progress.len();
    
    // Calculate percentages for status counts
    for sc in &mut overview.status_counts {
        if overview.total_titles > 0 {
            sc.percentage = (sc.count as f64 / overview.total_titles as f64 * 100.0) as i32;
        }
    }
    
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
        overview,
        activity_count,
        progress,
        progress_count,
        calendar_html: Safe(calendar_html),
    }
    .render()
    .unwrap()
    .into()
}
