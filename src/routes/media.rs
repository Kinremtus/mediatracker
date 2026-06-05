use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use uuid::Uuid;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;
use super::home::SidebarStats;

#[derive(Template)]
#[template(path = "media_drawer_content.html")]
struct MediaDrawerTemplate {
    item: CreateMediaItem,
    tracking_id: Option<Uuid>,
    current_status: Option<String>,
    progress: Option<i32>,
    rating: Option<f64>,
    total_count: Option<i32>,
    progress_unit: String,
    has_progress: bool,
    role: String,
    star_classes: Vec<&'static str>,
    progress_display: i32,
    total_display: String,
    rating_display: String,
    can_increment: bool,
    next_progress: i32,
    status_display: String,
}

impl MediaDrawerTemplate {
    fn compute_star_classes(rating: Option<f64>) -> Vec<&'static str> {
        (1..=10).map(|star| {
            match rating {
                Some(r) => {
                    if r >= star as f64 { "active" }
                    else if r >= (star as f64) - 0.5 { "half" }
                    else { "" }
                }
                None => "",
            }
        }).collect()
    }
}

#[derive(Template)]
#[template(path = "media_detail.html")]
struct MediaDetailTemplate {
    username: String,
    role: String,
    stats: SidebarStats,
    active_page: String,
    item: CreateMediaItem,
    current_status: String,
    flash_message: String,
}

#[derive(Deserialize)]
pub struct MediaDetailQuery {
    media_type: Option<String>,
    flash: Option<String>,
}

pub async fn get_media_detail(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
    Query(params): Query<MediaDetailQuery>,
) -> impl IntoResponse {
    let item = match provider.as_str() {
        "shikimori" => state.shikimori.get_details(&external_id).await,
        "mal" => state.mal.get_details(&external_id).await,
        "mangaupdates" => state.mangaupdates.get_details(&external_id).await,
        "tmdb" => {
            let media_type = params.media_type.as_deref().unwrap_or("movie");
            state.tmdb.get_details(&external_id, media_type).await
        }
        "rawg" => state.rawg.get_details(&external_id).await,
        "igdb" => state.igdb.get_details(&external_id).await,
        "google_books" => state.google_books.get_details(&external_id).await,
        "openlibrary" => state.openlibrary.get_details(&external_id).await,
        _ => Err(anyhow::anyhow!("Unknown provider")),
    };

    let stats = get_sidebar_stats(&state, &user).await;

    match item {
        Ok(mut item) => {
            if let Ok(Some(_)) = state.tracking.find_entry_by_media(
                user.id, &item.provider, &item.external_id,
            ).await {
                item.is_tracked = true;
            }
            let flash_message = params.flash.as_deref().map(|f| match f {
                "added" => "✓ Медиа добавлено в список".to_string(),
                "error" => "Ошибка при добавлении".to_string(),
                _ => String::new(),
            }).unwrap_or_default();

            Html(
                MediaDetailTemplate {
                    username: user.username,
                    role: user.role,
                    stats,
                    active_page: "search".to_string(),
                    item,
                    current_status: String::new(),
                    flash_message,
                }
                .render()
                .unwrap(),
            )
            .into_response()
        }
        Err(_) => Html("Not found".to_string()).into_response(),
    }
}

pub async fn get_media_drawer_content(
    user: CurrentUser,
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
    Query(params): Query<MediaDetailQuery>,
) -> impl IntoResponse {
    let item = match provider.as_str() {
        "shikimori" => state.shikimori.get_details(&external_id).await,
        "mal" => state.mal.get_details(&external_id).await,
        "mangaupdates" => state.mangaupdates.get_details(&external_id).await,
        "tmdb" => {
            let media_type = params.media_type.as_deref().unwrap_or("movie");
            state.tmdb.get_details(&external_id, media_type).await
        }
        "rawg" => state.rawg.get_details(&external_id).await,
        "igdb" => state.igdb.get_details(&external_id).await,
        "google_books" => state.google_books.get_details(&external_id).await,
        "openlibrary" => state.openlibrary.get_details(&external_id).await,
        _ => Err(anyhow::anyhow!("Unknown provider")),
    };

    match item {
        Ok(item) => {
            let tracking = state.tracking.find_entry_by_media(user.id, &provider, &external_id).await.unwrap_or(None);
            let (tracking_id, current_status, progress, rating) = match tracking {
                Some((id, status, prog, rat)) => (Some(id), Some(status), Some(prog), rat),
                None => (None, None, None, None),
            };
            let total_count = item.total_count();
            let progress_unit = item.progress_unit_ru().to_string();
            let has_progress = matches!(item.media_type.as_str(),
                "anime" | "series" | "cartoons" | "animated-movies"
                | "manga" | "manhwa" | "manhua" | "novel" | "other-comics"
                | "book" | "game"
            );
            let star_classes = MediaDrawerTemplate::compute_star_classes(rating);
            let progress_display = progress.unwrap_or(0);
            let total_display = match total_count {
                Some(tc) => format!(" / {tc}"),
                None => String::new(),
            };
            let rating_display = match rating {
                Some(r) => format!("{:.1}", r),
                None => "—".to_string(),
            };
            let can_increment = has_progress
                && progress_display < total_count.unwrap_or(i32::MAX);
            let next_progress = progress_display + 1;
            let status_display = current_status.clone().unwrap_or_else(|| "in_progress".to_string());
            Html(
                MediaDrawerTemplate {
                    item,
                    tracking_id,
                    current_status,
                    progress,
                    rating,
                    total_count,
                    progress_unit,
                    has_progress,
                    role: user.role,
                    star_classes,
                    progress_display,
                    total_display,
                    rating_display,
                    can_increment,
                    next_progress,
                    status_display,
                }
                .render()
                .unwrap()
            )
            .into_response()
        }
        Err(_) => Html("Not found".to_string()).into_response(),
    }
}

async fn get_sidebar_stats(state: &AppState, user: &CurrentUser) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user.id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp, role: user.role.clone() }
}

#[derive(Template)]
#[template(path = "partials/_episode_list.html")]
struct EpisodeListPartial {
    episodes: Vec<crate::services::episodes::StoredEpisode>,
}

/// Lazy-loaded endpoint for the drawer's "Episodes" section.
/// If episodes aren't in the DB yet (e.g. background fetch from
/// post_add_to_tracking hasn't completed), trigger a synchronous
/// fetch+store so the drawer doesn't show "Эпизоды не загружены"
/// on first open.
///
/// Episode source is always Jikan v4. We store them under
/// `provider = "mal"`, `external_id = mal_id.to_string()`. For
/// Shikimori-sourced entries the URL still has the shikimori id
/// in `external_id`, so we look up `mal_id` from `media_items`
/// first and key the episode read/fetch on that.
pub async fn get_episodes(
    State(state): State<AppState>,
    Path((provider, external_id)): Path<(String, String)>,
) -> impl IntoResponse {
    // Resolve the MAL id (episode key) for this anime.
    let mal_id: Option<i64> = match provider.as_str() {
        "mal" => external_id.parse::<i64>().ok(),
        "shikimori" => {
            match crate::services::episodes::lookup_mal_id(
                &state.db, &provider, &external_id,
            )
            .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::warn!(provider, external_id, error = %e, "lookup_mal_id failed");
                    None
                }
            }
        }
        _ => None,
    };

    // Try DB first (episodes are stored under provider="mal" keyed by mal_id).
    let mut existing = Vec::new();
    if let Some(mal_id) = mal_id {
        existing = crate::services::episodes::get_episodes(
            &state.db,
            "mal",
            &mal_id.to_string(),
        )
        .await
        .unwrap_or_default();
    }

    // If empty, fetch on-demand via Jikan.
    if existing.is_empty() {
        if let Some(mal_id) = mal_id {
            if let Err(e) = crate::services::episodes::fetch_and_store_mal(
                state.db.clone(),
                &state.mal,
                mal_id,
            )
            .await
            {
                tracing::warn!(provider, external_id, mal_id, error = %e, "on-demand episode fetch failed");
            }
        } else {
            tracing::debug!(provider, external_id, "no mal_id available; cannot fetch episodes");
        }
    }

    let episodes = match mal_id {
        Some(id) => crate::services::episodes::get_episodes(
            &state.db,
            "mal",
            &id.to_string(),
        )
        .await
        .unwrap_or_default(),
        None => Vec::new(),
    };

    let html = EpisodeListPartial { episodes }.render().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "episode list render failed");
        String::new()
    });
    Html(html)
}
