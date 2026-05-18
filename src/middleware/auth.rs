use axum::{
    extract::{FromRequestParts, Request, State},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::Response,
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
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<CurrentUser>()
            .cloned()
            .ok_or((StatusCode::UNAUTHORIZED, "Missing user session"))
    }
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Extract session cookie
    let cookie = req
        .headers()
        .get(axum::http::header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|c| c.split(';').find(|c| c.trim().starts_with("session_id=")))
        .map(|c| c.trim().trim_start_matches("session_id=").to_string());

    let token = match cookie {
        Some(t) => t,
        None => return Err(StatusCode::UNAUTHORIZED),
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
                    Ok(next.run(req).await)
                }
                Err(_) => Err(StatusCode::UNAUTHORIZED),
            }
        }
        Err(_) => Err(StatusCode::UNAUTHORIZED),
    }
}
