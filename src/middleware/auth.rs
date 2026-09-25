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
    pub role: String,
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
    // Extract session cookie (raw token; the DB stores only its sha256 hash).
    let token = match crate::utils::session_cookie(req.headers()) {
        Some(t) => t,
        None => {
            tracing::warn!("No session_id cookie found");
            return Redirect::to("/login").into_response();
        }
    };

    // Validate session
    match state.auth.get_session(&token).await {
        Ok(session) => {
            tracing::debug!(user_id = %session.user_id, "session validated");
            // Get user details
            match state.auth.get_user_by_id(session.user_id).await {
                Ok(user) => {
                    let current_user = CurrentUser {
                        id: user.id,
                        username: user.username,
                        role: user.role,
                    };
                    req.extensions_mut().insert(current_user);
                    next.run(req).await
                }
                Err(e) => {
                    tracing::warn!("User lookup failed: {}", e);
                    Redirect::to("/login").into_response()
                }
            }
        }
        Err(e) => {
            tracing::warn!("Session validation failed: {}", e);
            Redirect::to("/login").into_response()
        }
    }
}
