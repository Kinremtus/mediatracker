use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;
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
    let mut results = Vec::new();

    if !query.is_empty() {
        results = match search_type.as_str() {
            "anime" => state.shikimori.search(&query).await.unwrap_or_default(),
            "manga" => state.mangaupdates.search_by_type(&query, &["Manga"]).await.unwrap_or_default(),
            "manhwa" => state.mangaupdates.search_by_type(&query, &["Manhwa"]).await.unwrap_or_default(),
            "manhua" => state.mangaupdates.search_by_type(&query, &["Manhua"]).await.unwrap_or_default(),
            "novels" => state.mangaupdates.search_by_type(&query, &["novel"]).await.unwrap_or_default(),
            "other-comics" => state.mangaupdates.search_by_type(&query, &["OEL", "Doujinshi", "Filipino", "Indonesian", "Thai", "Vietnamese", "Malaysian"]).await.unwrap_or_default(),
            "movies" => state.tmdb.search_movies(&query, None).await.unwrap_or_default(),
            "tv" => state.tmdb.search_tv(&query, None).await.unwrap_or_default(),
            "dramas" => state.tmdb.search_tv(&query, Some(18)).await.unwrap_or_default(),
            "cartoons" => state.tmdb.search_tv(&query, Some(16)).await.unwrap_or_default(),
            "animated-movies" => state.tmdb.search_movies(&query, Some(16)).await.unwrap_or_default(),
            "games" => state.rawg.search(&query).await.unwrap_or_default(),
            "books" => state.google_books.search(&query).await.unwrap_or_default(),
            _ => {
                // Default: search all providers
                let mut all = Vec::new();
                if let Ok(r) = state.shikimori.search(&query).await { all.extend(r); }
                if let Ok(r) = state.mangaupdates.search(&query).await { all.extend(r); }
                if !state.tmdb.api_key.is_empty() {
                    if let Ok(r) = state.tmdb.search(&query).await { all.extend(r); }
                }
                if !state.rawg.api_key.is_empty() {
                    if let Ok(r) = state.rawg.search(&query).await { all.extend(r); }
                }
                all
            }
        };
    }

    let stats = get_sidebar_stats(&state, user.id).await;

    SearchTemplate {
        username: user.username,
        stats,
        active_page: "search".to_string(),
        query,
        current_type: search_type,
        results,
    }
    .render()
    .unwrap()
    .into()
}

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let mut stats = SidebarStats::default();
    if let Ok(entries) = state.tracking.get_user_entries(user_id, None).await {
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
