use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;
use super::home::SidebarStats;

#[derive(Template)]
#[template(path = "media_drawer_content.html")]
struct MediaDrawerTemplate {
    item: CreateMediaItem,
}

#[derive(Template)]
#[template(path = "media_detail.html")]
struct MediaDetailTemplate {
    username: String,
    stats: SidebarStats,
    active_page: String,
    item: CreateMediaItem,
    current_status: String,
}

#[derive(Deserialize)]
pub struct MediaDetailQuery {
    media_type: Option<String>,
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
        "google_books" => state.google_books.get_details(&external_id).await,
        _ => Err(anyhow::anyhow!("Unknown provider")),
    };

    let stats = get_sidebar_stats(&state, user.id).await;

    match item {
        Ok(item) => Html(
            MediaDetailTemplate {
                username: user.username,
                stats,
                active_page: "search".to_string(),
                item,
                current_status: String::new(),
            }
            .render()
            .unwrap(),
        )
        .into_response(),
        Err(_) => Html("Not found".to_string()).into_response(),
    }
}

pub async fn get_media_drawer_content(
    _user: CurrentUser,
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
        "google_books" => state.google_books.get_details(&external_id).await,
        _ => Err(anyhow::anyhow!("Unknown provider")),
    };

    match item {
        Ok(item) => Html(
            MediaDrawerTemplate { item }.render().unwrap()
        )
        .into_response(),
        Err(_) => Html("Not found".to_string()).into_response(),
    }
}

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user_id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp }
}
