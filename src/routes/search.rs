use askama::Template;
use axum::{
    extract::{Query, State},
    response::{Html, Json},
};
use serde::Deserialize;

use super::home::{SidebarStats, get_sidebar_stats};
use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::{CreateMediaItem, SearchSuggestion};
use crate::services::search;
use crate::services::search_families;

const ITEMS_PER_PAGE: usize = 24;
const PANEL_PAGE: usize = 5;
const PANEL_LIMIT_CAP: usize = 50;

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
    media_types: Vec<super::tracking::MediaTypeItem>,
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
    }
    .render()
    .unwrap_or_else(|e| {
        tracing::error!(error = %e, "template render failed");
        String::from("Internal Server Error")
    })
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

    mark_tracked(&state, user.id, &mut results).await;

    let suggestions: Vec<SearchSuggestion> = results
        .into_iter()
        .take(5)
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

#[derive(Deserialize)]
pub struct QuickSearchQuery {
    q: Option<String>,
    family: Option<String>,
    #[serde(rename = "type")]
    section_type: Option<String>,
    limit: Option<usize>,
}

struct FamilyChip {
    key: String,
    icon: String,
    label: String,
}

struct QuickSearchItem {
    provider: String,
    external_id: String,
    media_type: String,
    media_type_label: String,
    title: String,
    poster_url: Option<String>,
    year: Option<i16>,
    score: Option<f64>,
    is_tracked: bool,
}

struct QuickSearchSection {
    media_type: String,
    icon: String,
    label: String,
    items: Vec<QuickSearchItem>,
    has_more: bool,
    remaining: usize,
    next_limit: usize,
}

#[derive(Template)]
#[template(path = "partials/_quick_search_results.html")]
struct QuickSearchView {
    query: String,
    active_family: String,
    tabs_visible: bool,
    families: Vec<FamilyChip>,
    sections: Vec<QuickSearchSection>,
    too_short: bool,
}

/// `GET /api/search/panel` — server-rendered quick-search fragment.
///
/// - `family=""` (the default "Все" tab) renders `all_types` results grouped
///   inline by raw media type.
/// - `family=<key>` fans out to the raw types of that family.
/// - `type=<raw>&limit=<n>` is the "Показать ещё" expansion mode: one section,
///   more items, no tabs.
pub async fn get_search_panel(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<QuickSearchQuery>,
) -> Html<String> {
    let query = params.q.unwrap_or_default().trim().to_string();
    let active_family = params.family.unwrap_or_default();
    let limit = params.limit.unwrap_or(PANEL_PAGE).clamp(1, PANEL_LIMIT_CAP);

    let families: Vec<FamilyChip> = search_families::families()
        .into_iter()
        .map(|f| FamilyChip {
            key: f.key.to_string(),
            icon: f.icon.to_string(),
            label: f.label.to_string(),
        })
        .collect();

    if query.chars().count() < 2 {
        return render_panel(QuickSearchView {
            query,
            active_family,
            tabs_visible: true,
            families,
            sections: Vec::new(),
            too_short: true,
        });
    }

    // Expansion mode ("Показать ещё" outerHTML swap onto one section).
    if let Some(section_type) = params.section_type.as_deref() {
        let mut items = search::by_media_type(&state, &query, section_type).await;
        mark_tracked(&state, user.id, &mut items).await;
        let section = build_section(section_type, items, limit);
        return render_panel(QuickSearchView {
            query,
            active_family,
            tabs_visible: false,
            families,
            sections: vec![section],
            too_short: false,
        });
    }

    let mut all_items: Vec<CreateMediaItem> = if active_family.is_empty() {
        // "Все" is a real tab: one call covering 7 providers, grouped inline.
        search::all_types(&state, &query).await
    } else {
        match search_families::family(&active_family) {
            Some(fam) if fam.types.len() == 1 => {
                search::by_media_type(&state, &query, fam.types[0]).await
            }
            Some(fam) => {
                let mut set = tokio::task::JoinSet::new();
                for raw in fam.types {
                    let state = state.clone();
                    let query = query.clone();
                    let raw = raw.to_string();
                    set.spawn(async move { search::by_media_type(&state, &query, &raw).await });
                }
                let mut merged = Vec::new();
                while let Some(joined) = set.join_next().await {
                    match joined {
                        Ok(items) => merged.extend(items),
                        Err(e) => tracing::warn!(error = %e, "quick-search fan-out task failed"),
                    }
                }
                merged
            }
            None => Vec::new(),
        }
    };

    mark_tracked(&state, user.id, &mut all_items).await;
    let sections = group_sections(all_items, limit);

    render_panel(QuickSearchView {
        query,
        active_family,
        tabs_visible: true,
        families,
        sections,
        too_short: false,
    })
}

fn render_panel(view: QuickSearchView) -> Html<String> {
    view.render().map(Html).unwrap_or_else(|e| {
        tracing::error!(error = %e, "template render failed");
        Html(String::from("Internal Server Error"))
    })
}

fn to_item(m: CreateMediaItem) -> QuickSearchItem {
    let (_, label) = search_families::type_meta(&m.media_type);
    QuickSearchItem {
        provider: m.provider,
        external_id: m.external_id,
        media_type: m.media_type,
        media_type_label: label,
        title: m.title,
        poster_url: m.poster_url,
        year: m.year,
        score: m.score,
        is_tracked: m.is_tracked,
    }
}

fn build_section(
    media_type: &str,
    items: Vec<CreateMediaItem>,
    limit: usize,
) -> QuickSearchSection {
    let (icon, label) = search_families::type_meta(media_type);
    let total = items.len();
    let shown = total.min(limit);
    let remaining = total.saturating_sub(shown);
    QuickSearchSection {
        media_type: media_type.to_string(),
        icon,
        label,
        items: items.into_iter().take(shown).map(to_item).collect(),
        has_more: remaining > 0,
        remaining,
        next_limit: shown + PANEL_PAGE,
    }
}

fn group_sections(items: Vec<CreateMediaItem>, limit: usize) -> Vec<QuickSearchSection> {
    use std::collections::HashMap;
    let mut by_type: HashMap<String, Vec<CreateMediaItem>> = HashMap::new();
    for item in items {
        by_type
            .entry(item.media_type.clone())
            .or_default()
            .push(item);
    }
    let mut sections = Vec::new();
    for (key, _, _) in super::tracking::get_all_media_types() {
        if let Some(items) = by_type.remove(key) {
            if items.is_empty() {
                continue;
            }
            sections.push(build_section(key, items, limit));
        }
    }
    sections
}
