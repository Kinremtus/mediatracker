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
}

#[derive(Template)]
#[template(path = "media_detail.html")]
struct MediaDetailTemplate {
    username: String,
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

    let stats = get_sidebar_stats(&state, user.id).await;

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
            let (tracking_id, current_status) = match tracking {
                Some((id, status)) => (Some(id), Some(status)),
                None => (None, None),
            };
            Html(
                MediaDrawerTemplate { item, tracking_id, current_status }.render().unwrap()
            )
            .into_response()
        }
        Err(_) => Html("Not found".to_string()).into_response(),
    }
}

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user_id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp }
}
