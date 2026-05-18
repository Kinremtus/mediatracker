use askama::Template;
use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::media_item::CreateMediaItem;

#[derive(Template)]
#[template(path = "media_detail.html")]
struct MediaDetailTemplate {
    username: String,
    item: CreateMediaItem,
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
        "mangaupdates" => state.mangaupdates.get_details(&external_id).await,
        "tmdb" => {
            let media_type = params.media_type.as_deref().unwrap_or("movie");
            state.tmdb.get_details(&external_id, media_type).await
        }
        "rawg" => state.rawg.get_details(&external_id).await,
        _ => Err(anyhow::anyhow!("Unknown provider")),
    };

    match item {
        Ok(item) => Html(
            MediaDetailTemplate {
                username: user.username,
                item,
            }
            .render()
            .unwrap(),
        )
        .into_response(),
        Err(_) => Html("Not found".to_string()).into_response(),
    }
}
