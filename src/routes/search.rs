use askama::Template;
use axum::{
    extract::{Query, State},
    response::{Html, Json},
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::{CreateMediaItem, SearchSuggestion};
use crate::services::search;
use super::home::SidebarStats;

const ITEMS_PER_PAGE: usize = 24;

#[derive(Debug)]
struct PageItem {
    num: u32,
    current: bool,
}

#[derive(Template)]
#[template(path = "search.html")]
#[expect(dead_code)]
struct SearchTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    query: String,
    current_type: String,
    results: Vec<CreateMediaItem>,
    current_status: String,
    flash_message: String,
    page: u32,
    total_pages: u32,
    pages: Vec<PageItem>,
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
    #[serde(rename = "type")]
    search_type: Option<String>,
    flash: Option<String>,
    page: Option<u32>,
}

pub async fn get_search(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Html<String> {
    let query = params.q.unwrap_or_default();
    let search_type = params.search_type.unwrap_or_default();

    let mut all_results = if query.is_empty() {
        Vec::new()
    } else {
        search::by_media_type(&state, &query, &search_type).await
    };

    for item in &mut all_results {
        if let Ok(Some(_)) = state.tracking.find_entry_by_media(
            user.id, &item.provider, &item.external_id,
        ).await {
            item.is_tracked = true;
        }
    }

    let total_pages = if all_results.is_empty() {
        1
    } else {
        (all_results.len() as f64 / ITEMS_PER_PAGE as f64).ceil() as u32
    };

    let page = params
        .page
        .unwrap_or(1)
        .clamp(1, total_pages);

    let page_idx = (page - 1) as usize;
    let results: Vec<CreateMediaItem> = all_results
        .chunks(ITEMS_PER_PAGE)
        .nth(page_idx)
        .unwrap_or_default()
        .to_vec();

    let stats = get_sidebar_stats(&state, &user).await;

    let flash_message = params.flash.as_deref().map(|f| match f {
        "added" => "✓ Медиа добавлено в список".to_string(),
        "error" => "Ошибка при добавлении".to_string(),
        _ => String::new(),
    }).unwrap_or_default();

    SearchTemplate {
        username: user.username,
        role: user.role.clone(),
        stats,
        active_page: "search".to_string(),
        query,
        current_type: search_type,
        results,
        current_status: String::new(),
        flash_message,
        page,
        total_pages,
        pages: (1..=total_pages).map(|p| PageItem { num: p, current: p == page }).collect(),
    }
    .render()
    .unwrap()
    .into()
}

#[derive(Deserialize)]
pub struct SuggestionsQuery {
    q: Option<String>,
}

pub async fn get_search_suggestions(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<SuggestionsQuery>,
) -> Json<Vec<SearchSuggestion>> {
    let query = match params.q {
        Some(q) if !q.trim().is_empty() => q.trim().to_string(),
        _ => return Json(Vec::new()),
    };

    if query.len() < 2 {
        return Json(Vec::new());
    }

    let mut results = search::by_media_type(&state, &query, "").await;

    for item in &mut results {
        if let Ok(Some(_)) = state.tracking.find_entry_by_media(
            user.id, &item.provider, &item.external_id,
        ).await {
            item.is_tracked = true;
        }
    }

    let suggestions: Vec<SearchSuggestion> = results
        .into_iter()
        .take(10)
        .map(|item| SearchSuggestion {
            provider: item.provider,
            external_id: item.external_id,
            media_type: item.media_type,
            title: item.title,
            title_english: item.title_english,
            poster_url: item.poster_url,
            year: item.year,
            score: item.score,
            is_tracked: item.is_tracked,
        })
        .collect();

    Json(suggestions)
}

async fn get_sidebar_stats(state: &AppState, user: &CurrentUser) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user.id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp, role: user.role.clone() }
}
