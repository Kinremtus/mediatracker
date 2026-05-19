use askama::Template;
use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect, Response},
    http::{header::SET_COOKIE, HeaderValue},
};
use serde::Serialize;

use crate::app_state::AppState;
use crate::middleware::CurrentUser;

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    username: String,
    stats: SidebarStats,
    active_page: String,
}

#[derive(Serialize, Clone, Default)]
pub struct SidebarStats {
    pub watching: i32,
    pub reading: i32,
    pub completed: i32,
    pub planned: i32,
    pub dropped: i32,
}

pub async fn get_home(
    user: CurrentUser,
    State(state): State<AppState>,
) -> Html<String> {
    let stats = get_sidebar_stats(&state, user.id).await;

    HomeTemplate {
        username: user.username,
        stats,
        active_page: "home".to_string(),
    }
    .render()
    .unwrap()
    .into()
}

pub async fn post_logout(
    State(_state): State<AppState>,
    _user: CurrentUser,
) -> Response {
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str("session_id=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0").unwrap(),
    );
    response
}

async fn get_sidebar_stats(state: &AppState, user_id: uuid::Uuid) -> SidebarStats {
    let mut stats = SidebarStats::default();
    if let Ok(entries) = state.tracking.get_user_entries(user_id, None).await {
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
