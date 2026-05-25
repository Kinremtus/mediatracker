use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;
use crate::services::search;
use super::home::SidebarStats;

#[derive(Template)]
#[template(path = "search.html")]
struct SearchTemplate {
    username: String,
    stats: SidebarStats,
    active_page: String,
    query: String,
    current_type: String,
    results: Vec<CreateMediaItem>,
    current_status: String,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    #[serde(rename = "type")]
    search_type: Option<String>,
}

pub async fn get_search(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Html<String> {
    let query = params.q.unwrap_or_default();
    let search_type = params.search_type.unwrap_or_default();

    let results = if query.is_empty() {
        Vec::new()
    } else {
        search::by_media_type(&state, &query, &search_type).await
    };

    let stats = get_sidebar_stats(&state, user.id).await;

    SearchTemplate {
        username: user.username,
        stats,
        active_page: "search".to_string(),
        query,
        current_type: search_type,
        results,
        current_status: String::new(),
    }
    .render()
    .unwrap()
    .into()
}

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user_id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp }
}
