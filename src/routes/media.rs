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
