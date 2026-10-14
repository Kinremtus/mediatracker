use askama::Template;
use axum::{
    extract::{Query, State},
    response::Html,
};
use serde::Deserialize;

use super::home::{SidebarStats, get_sidebar_stats};
use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;
use crate::services::search;
use crate::services::search_catalog;

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
    default_search_type: String,
    query: String,
    current_type: String,
    media_types: Vec<super::tracking::MediaTypeItem>,
    results: Vec<CreateMediaItem>,
    current_status: String,
    flash_message: String,
    page: u32,
    total_pages: u32,
    pages: Vec<PageItem>,
    /// Query without a selected type: the page asks to pick a category
    /// (search is type-scoped since the catalog rework).
    needs_type: bool,
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
    // An unknown `?type=` is treated as "none selected": the search is
    // type-scoped, and a garbage type must not reach the catalog.
    let search_type = if search_type.is_empty()
        || super::tracking::get_all_media_types()
            .iter()
            .any(|(k, _, _)| *k == search_type)
    {
        search_type
    } else {
        String::new()
    };
    let needs_type = !query.trim().is_empty() && search_type.is_empty();

    let mut all_results = if query.is_empty() || search_type.is_empty() {
        Vec::new()
    } else {
        load_results(&state, &search_type, &query).await
    };

    mark_tracked(&state, user.id, &mut all_results).await;
    let total_pages = if all_results.is_empty() {
        1
    } else {
        (all_results.len() as f64 / ITEMS_PER_PAGE as f64).ceil() as u32
    };

    let page = params.page.unwrap_or(1).clamp(1, total_pages);

    let page_idx = (page - 1) as usize;
    let results: Vec<CreateMediaItem> = all_results
        .chunks(ITEMS_PER_PAGE)
        .nth(page_idx)
        .unwrap_or_default()
        .to_vec();

    let stats = get_sidebar_stats(&state, &user).await;
    let media_types = super::tracking::all_media_type_items();

    let flash_message = params
        .flash
        .as_deref()
        .map(|f| match f {
            "added" => "✓ Медиа добавлено в список".to_string(),
            "error" => "Ошибка при добавлении".to_string(),
            _ => String::new(),
        })
        .unwrap_or_default();

    SearchTemplate {
        username: user.username,
        role: user.role.clone(),
        stats,
        active_page: "search".to_string(),
        default_search_type: user.default_search_type.clone().unwrap_or_default(),
        query,
        current_type: search_type,
        media_types,
        results,
        current_status: String::new(),
        flash_message,
        page,
        total_pages,
        pages: (1..=total_pages)
            .map(|p| PageItem {
                num: p,
                current: p == page,
            })
            .collect(),
        needs_type,
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "template render failed");
        String::from("Internal Server Error")
    })
    .into()
}

/// The catalog first: a fresh recorded query answers from the DB; a miss runs
/// the live search (official sources, then the provider chains) and stores the
/// answer so the next search of the same query is instant. A failed lookup or
/// store only costs speed — the live answer is served regardless.
async fn load_results(state: &AppState, media_type: &str, query: &str) -> Vec<CreateMediaItem> {
    match search_catalog::lookup(&state.db, media_type, query).await {
        Ok(Some(items)) => return items,
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "search catalog lookup failed, going live"),
    }
    let items = search::live_search(state, media_type, query).await;
    if let Err(e) = search_catalog::store(&state.db, media_type, query, &items).await {
        tracing::warn!(error = %e, "search catalog store failed");
    }
    items
}

/// Fill `is_tracked` for every result with a single batched query (P2-F),
/// instead of one `find_entry_by_media` round-trip per item.
async fn mark_tracked(state: &AppState, user_id: uuid::Uuid, items: &mut [CreateMediaItem]) {
    let pairs: Vec<(String, String)> = items
        .iter()
        .map(|i| (i.provider.clone(), i.external_id.clone()))
        .collect();
    let tracked = match state.tracking.find_tracked_media(user_id, &pairs).await {
        Ok(set) => set,
        Err(e) => {
            tracing::warn!(error = %e, "search: failed to load tracked media");
            return;
        }
    };
    for item in items {
        if tracked.contains(&(item.provider.clone(), item.external_id.clone())) {
            item.is_tracked = true;
        }
    }
}
