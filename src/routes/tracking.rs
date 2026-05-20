use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use uuid::Uuid;
use std::collections::HashMap;

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
    categories: Vec<CategoryGroup>,
    type_buttons: Vec<TypeButton>,
    current_status: String,
    current_media_type: String,
}

struct CategoryGroup {
    media_type: String,
    icon: String,
    label: String,
    entries: Vec<TrackingEntryWithMedia>,
}

struct TypeButton {
    media_type: String,
    icon: String,
    label: String,
    url: String,
    is_active: bool,
}

fn get_type_buttons(current_status: &str, current_media_type: &str) -> Vec<TypeButton> {
    let all_types = get_all_media_types();
    let mut buttons = Vec::new();
    
    // "Все" button
    let all_url = if current_status.is_empty() {
        "/tracking".to_string()
    } else {
        format!("/tracking?status={}", current_status)
    };
    buttons.push(TypeButton {
        media_type: String::new(),
        icon: "📦".to_string(),
        label: "Все".to_string(),
        url: all_url,
        is_active: current_media_type.is_empty(),
    });
    
    // Type buttons
    for (mt, icon, label) in all_types {
        let url = if current_status.is_empty() {
            format!("/tracking?type={}", mt)
        } else {
            format!("/tracking?status={}&type={}", current_status, mt)
        };
        buttons.push(TypeButton {
            media_type: mt.to_string(),
            icon: icon.to_string(),
            label: label.to_string(),
            url,
            is_active: current_media_type == mt,
        });
    }
    
    buttons
}

fn get_all_media_types() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        ("anime", "▶", "Аниме"),
        ("manga", "📚", "Манга"),
        ("manhwa", "📚", "Манхва"),
        ("manhua", "📚", "Маньхуа"),
        ("novels", "", "Новеллы"),
        ("other-comics", "📚", "Другие комиксы"),
        ("movies", "", "Фильмы"),
        ("tv", "📺", "Сериалы"),
        ("dramas", "🎬", "Дорамы"),
        ("cartoons", "📺", "Мультсериалы"),
        ("animated-movies", "", "Мультфильмы"),
        ("games", "🎮", "Игры"),
        ("books", "📖", "Книги"),
    ]
}

fn group_entries_by_type(entries: Vec<TrackingEntryWithMedia>, filter_type: Option<&str>) -> Vec<CategoryGroup> {
    let all_types = get_all_media_types();
    
    if let Some(mt) = filter_type {
        // Filtered: only one category
        let (icon, label) = all_types.iter()
            .find(|(t, _, _)| *t == mt)
            .map(|(_, i, l)| (i.to_string(), l.to_string()))
            .unwrap_or_else(|| ("📦".to_string(), mt.to_string()));
        
        vec![CategoryGroup {
            media_type: mt.to_string(),
            icon,
            label,
            entries,
        }]
    } else {
        // All types: group entries
        let mut grouped: HashMap<String, Vec<TrackingEntryWithMedia>> = HashMap::new();
        for entry in entries {
            grouped.entry(entry.media.media_type.clone()).or_default().push(entry);
        }
        
        all_types.iter().map(|(mt, icon, label)| {
            CategoryGroup {
                media_type: mt.to_string(),
                icon: icon.to_string(),
                label: label.to_string(),
                entries: grouped.remove(*mt).unwrap_or_default(),
            }
        }).collect()
    }
}

#[derive(Deserialize)]
pub struct TrackingQuery {
    status: Option<String>,
    #[serde(rename = "type")]
    media_type: Option<String>,
}

pub async fn get_tracking_list(
    user: CurrentUser,
    State(state): State<AppState>,
    Query(params): Query<TrackingQuery>,
) -> Html<String> {
    let status = params.status.as_deref();
    let media_type = params.media_type.as_deref();
    let entries = state.tracking.get_user_entries(user.id, status, media_type).await.unwrap_or_default();
    let stats = get_sidebar_stats(&state, user.id).await;
    let categories = group_entries_by_type(entries, media_type);
    let current_status = params.status.unwrap_or_default();
    let current_media_type = params.media_type.unwrap_or_default();
    let type_buttons = get_type_buttons(&current_status, &current_media_type);

    TrackingListTemplate {
        username: user.username,
        stats,
        active_page: "tracking".to_string(),
        categories,
        type_buttons,
        current_status,
        current_media_type,
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

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let mut stats = SidebarStats::default();
    if let Ok(entries) = state.tracking.get_user_entries(user_id, None, None).await {
        for e in entries {
            match e.entry.status.as_str() {
                "watching" => stats.watching += 1,
                "reading" => stats.reading += 1,
                "completed" => stats.completed += 1,
                "planned" => stats.planned += 1,
                "dropped" => stats.dropped += 1,
                _ => {}
            }
        }
    }
    stats
}
