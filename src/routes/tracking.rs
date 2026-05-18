use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::{header::SET_COOKIE, HeaderValue},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::tracking_entry::UpdateTracking;

#[derive(Template)]
#[template(path = "tracking_list.html")]
struct TrackingListTemplate {
    username: String,
    entries: Vec<crate::models::tracking_entry::TrackingEntry>,
    current_status: String,
}

#[derive(Deserialize)]
pub struct TrackingQuery {
    status: Option<String>,
}

pub async fn get_tracking_list(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<TrackingQuery>,
) -> Html<String> {
    let status = params.status.as_deref();
    let entries = state.tracking.get_user_entries(user.id, status).await.unwrap_or_default();

    TrackingListTemplate {
        username: user.username,
        entries,
        current_status: params.status.unwrap_or_default(),
    }
    .render()
    .unwrap()
    .into()
}

#[derive(Deserialize)]
pub struct AddToTrackingForm {
    pub provider: String,
    pub external_id: String,
    pub media_type: String,
    pub title: String,
    pub title_english: Option<String>,
    pub title_native: Option<String>,
    pub title_russian: Option<String>,
    pub poster_url: Option<String>,
    pub episodes: Option<i32>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub score: Option<f64>,
    pub tracking_status: String,
}

pub async fn post_add_to_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    Form(form): Form<AddToTrackingForm>,
) -> Response {
    let media = crate::models::media_item::CreateMediaItem {
        provider: form.provider,
        external_id: form.external_id,
        media_type: form.media_type,
        title: form.title,
        title_english: form.title_english,
        title_native: form.title_native,
        title_russian: form.title_russian,
        poster_url: form.poster_url,
        episodes: form.episodes,
        description: form.description,
        status: form.status,
        score: form.score,
    };

    let status = if form.tracking_status.is_empty() {
        "planned"
    } else {
        &form.tracking_status
    };

    match state.tracking.add_to_list(user.id, &media, status).await {
        Ok(_) => Redirect::to("/tracking").into_response(),
        Err(e) => {
            // In a real app, we'd show an error. For now, redirect back.
            eprintln!("Error adding to tracking: {}", e);
            Redirect::to("/tracking").into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct UpdateTrackingForm {
    pub status: Option<String>,
    pub rating: Option<f64>,
    pub progress: Option<i32>,
}

pub async fn post_update_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Form(form): Form<UpdateTrackingForm>,
) -> Response {
    let update = UpdateTracking {
        status: form.status,
        rating: form.rating,
        progress: form.progress,
    };

    match state.tracking.update_entry(id, user.id, &update).await {
        Ok(_) => Redirect::to("/tracking").into_response(),
        Err(e) => {
            eprintln!("Error updating tracking: {}", e);
            Redirect::to("/tracking").into_response()
        }
    }
}

pub async fn post_delete_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.tracking.delete_entry(id, user.id).await {
        Ok(_) => Redirect::to("/tracking").into_response(),
        Err(e) => {
            eprintln!("Error deleting tracking: {}", e);
            Redirect::to("/tracking").into_response()
        }
    }
}
