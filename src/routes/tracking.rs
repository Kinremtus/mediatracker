use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;
use crate::models::tracking_entry::{TrackingEntryWithMedia, UpdateTracking};
use super::home::SidebarStats;

#[derive(Template)]
#[template(path = "tracking_list.html")]
struct TrackingListTemplate {
    username: String,
    stats: SidebarStats,
    active_page: String,
    entries: Vec<TrackingEntryWithMedia>,
    status_label: String,
    current_status: String,
    current_media_type: String,
    search_query: String,
    media_types: Vec<MediaTypeItem>,
    statuses: Vec<StatusItem>,
    current_type_label: String,
}

struct MediaTypeItem {
    key: String,
    icon: String,
    label: String,
}

struct StatusItem {
    key: String,
    label: String,
}

fn get_all_media_types() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("anime", "▶", "Аниме"),
        ("manga", "📚", "Манга"),
        ("manhwa", "📚", "Манхва"),
        ("manhua", "📚", "Маньхуа"),
        ("novel", "", "Новеллы"),
        ("other-comics", "📚", "Другие комиксы"),
        ("movie", "", "Фильмы"),
        ("series", "", "Сериалы"),
        ("dramas", "🎬", "Дорамы"),
        ("cartoons", "📺", "Мультсериалы"),
        ("animated-movies", "", "Мультфильмы"),
        ("game", "", "Игры"),
        ("book", "📖", "Книги"),
    ]
}

fn get_status_label(status: &str) -> String {
    match status {
        "in_progress" => "В процессе",
        "completed" => "Завершено",
        "planned" => "Запланировано",
        "dropped" => "Брошено",
        _ => "Все списки",
    }.to_string()
}

#[derive(Deserialize)]
pub struct TrackingQuery {
    status: Option<String>,
    #[serde(rename = "type")]
    media_type: Option<String>,
    q: Option<String>,
}

pub async fn get_tracking_list(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<TrackingQuery>,
) -> Html<String> {
    let status = params.status.as_deref();
    let media_type = params.media_type.as_deref();
    let search_query = params.q.as_deref();
    let entries = state.tracking.get_user_entries(user.id, status, media_type, search_query).await.unwrap_or_default();
    let stats = get_sidebar_stats(&state, user.id).await;
    let current_status = params.status.unwrap_or_default();
    let current_media_type = params.media_type.unwrap_or_default();
    let search_query = params.q.unwrap_or_default();
    let status_label = get_status_label(&current_status);

    let all_types = get_all_media_types();
    let current_type_label = if current_media_type.is_empty() {
        "Все".to_string()
    } else {
        all_types.iter()
            .find(|(k, _, _)| *k == current_media_type)
            .map(|(_, _, l)| l.to_string())
            .unwrap_or_default()
    };
    let media_types: Vec<MediaTypeItem> = all_types.into_iter().map(|(k, i, l)| MediaTypeItem {
        key: k.to_string(),
        icon: i.to_string(),
        label: l.to_string(),
    }).collect();
    let statuses: Vec<StatusItem> = vec![
        ("", "Все списки"),
        ("in_progress", "В процессе"),
        ("completed", "Завершено"),
        ("planned", "Запланировано"),
        ("dropped", "Брошено"),
    ].into_iter().map(|(k, l)| StatusItem {
        key: k.to_string(),
        label: l.to_string(),
    }).collect();

    TrackingListTemplate {
        username: user.username,
        stats,
        active_page: "tracking".to_string(),
        entries,
        status_label,
        current_status,
        current_media_type,
        search_query,
        media_types,
        statuses,
        current_type_label,
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
    pub redirect_to: Option<String>,
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
        is_tracked: false,
        mal_id: None,
        comparison_key: None,
    };

    let status = if form.tracking_status.is_empty() {
        "planned"
    } else {
        &form.tracking_status
    };

    let redirect_url = form.redirect_to
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/tracking".to_string());

    match state.tracking.add_to_list(user.id, &media, status).await {
        Ok(_) => {
            let url = add_flash_param(&redirect_url, "added");
            Redirect::to(&url).into_response()
        }
        Err(e) => {
            eprintln!("Error adding to tracking: {}", e);
            let url = add_flash_param(&redirect_url, "error");
            Redirect::to(&url).into_response()
        }
    }
}

fn add_flash_param(url: &str, flash: &str) -> String {
    if url.contains('?') {
        format!("{}&flash={}", url, flash)
    } else {
        format!("{}?flash={}", url, flash)
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

// ========== HTMX Endpoints ==========

#[derive(Template)]
#[template(path = "partials/tracking_card.html")]
struct TrackingCardPartial {
    entry_with_media: TrackingEntryWithMedia,
}

#[derive(Template)]
#[template(path = "partials/tracking_grid.html")]
struct TrackingGridPartial {
    entries: Vec<TrackingEntryWithMedia>,
    current_status: String,
    current_media_type: String,
    search_query: String,
}

#[derive(Template)]
#[template(path = "partials/message.html")]
struct MessagePartial {
    message: Option<String>,
    error: Option<String>,
}

pub async fn htmx_update_tracking(
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
        Ok(entry) => {
            let entries = state.tracking.get_user_entries(user.id, None, None, None).await.unwrap_or_default();
            let entry_with_media = entries.iter().find(|e| e.entry.id == id).cloned();
            match entry_with_media {
                Some(ewm) => {
                    let html = TrackingCardPartial { entry_with_media: ewm }.render().unwrap();
                    (
                        [("HX-Trigger", "trackingUpdated")],
                        Html(html),
                    ).into_response()
                }
                None => Redirect::to("/tracking").into_response(),
            }
        }
        Err(e) => {
            eprintln!("Error updating tracking: {}", e);
            Redirect::to("/tracking").into_response()
        }
    }
}

pub async fn htmx_delete_tracking(
    user: CurrentUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Response {
    match state.tracking.delete_entry(id, user.id).await {
        Ok(_) => {
            (
                [("HX-Trigger", "trackingUpdated")],
                "",
            ).into_response()
        }
        Err(e) => {
            eprintln!("Error deleting tracking: {}", e);
            Redirect::to("/tracking").into_response()
        }
    }
}

pub async fn htmx_tracking_partial(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<TrackingQuery>,
) -> Html<String> {
    let status = params.status.as_deref();
    let media_type = params.media_type.as_deref();
    let search_query = params.q.as_deref();
    let entries = state.tracking.get_user_entries(user.id, status, media_type, search_query).await.unwrap_or_default();

    TrackingGridPartial {
        entries,
        current_status: params.status.unwrap_or_default(),
        current_media_type: params.media_type.unwrap_or_default(),
        search_query: params.q.unwrap_or_default(),
    }
    .render()
    .unwrap()
    .into()
}

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let (ip, cp, pp, dp) = state.tracking.get_status_counts(user_id).await.unwrap_or_default();
    SidebarStats { in_progress: ip, completed: cp, planned: pp, dropped: dp }
}
