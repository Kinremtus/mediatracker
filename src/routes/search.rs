use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;

#[derive(Template)]
#[template(path = "search.html")]
struct SearchTemplate {
    username: String,
    query: String,
    results: Vec<CreateMediaItem>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
}

pub async fn get_search(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Html<String> {
    let query = params.q.unwrap_or_default();
    let mut results = Vec::new();

    if !query.is_empty() {
        // Search in Shikimori
        if let Ok(shikimori_results) = state.shikimori.search(&query).await {
            results.extend(shikimori_results);
        }
        // Search in MangaUpdates
        if let Ok(mangaupdates_results) = state.mangaupdates.search(&query).await {
            results.extend(mangaupdates_results);
        }
        // Search in TMDB
        if !state.tmdb.api_key.is_empty() {
            if let Ok(tmdb_results) = state.tmdb.search(&query).await {
                results.extend(tmdb_results);
            }
        }
        // Search in RAWG
        if !state.rawg.api_key.is_empty() {
            if let Ok(rawg_results) = state.rawg.search(&query).await {
                results.extend(rawg_results);
            }
        }
    }

    SearchTemplate {
        username: user.username,
        query,
        results,
    }
    .render()
    .unwrap()
    .into()
}
