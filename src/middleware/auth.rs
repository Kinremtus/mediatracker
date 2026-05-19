use axum::{
    extract::{FromRequestParts, Request, State},
    http::request::Parts,
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use uuid::Uuid;

use crate::app_state::AppState;

// Extractor for current user
#[derive(Clone)]
pub struct CurrentUser {
    pub id: Uuid,
    pub username: String,
}

impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = (axum::http::StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CurrentUser>()
            .cloned()
            .ok_or((axum::http::StatusCode::UNAUTHORIZED, "Missing user session"))
    }
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    // Extract session cookie
    let cookie = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|c| c.split(';').find(|c| c.trim().starts_with("session_id=")))
        .map(|c| c.trim().trim_start_matches("session_id=").to_string());

    let token = match cookie {
        Some(t) => t,
        None => return Redirect::to("/login").into_response(),
    };

    // Validate session
    match state.auth.get_session(&token).await {
        Ok(session) => {
            // Get user details
            match state.auth.get_user_by_id(session.user_id).await {
                Ok(user) => {
                    let current_user = CurrentUser {
                        id: user.id,
                        username: user.username,
                    };
                    req.extensions_mut().insert(current_user);
                    next.run(req).await
                }
                Err(_) => Redirect::to("/login").into_response(),
            }
        }
        Err(_) => Redirect::to("/login").into_response(),
    }
}
