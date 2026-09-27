use axum::response::IntoResponse;

/// Log an internal error and return a generic 500 response.
pub(crate) fn internal_error(
    context: &str,
    error: impl std::fmt::Display,
) -> axum::response::Response {
    tracing::error!(context = context, error = %error, "internal error");
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        "Internal Server Error",
    )
        .into_response()
}

/// Log a client error and return a generic 400 response.
pub(crate) fn bad_request(context: &str) -> axum::response::Response {
    tracing::warn!(context = context, "bad request");
    (axum::http::StatusCode::BAD_REQUEST, "Bad Request").into_response()
}

/// Liveness/readiness endpoint. `src/main.rs` wires this exact function at
/// `/health`, so tests exercise the same handler the server serves.
pub async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({"status": "ok"}))
}

pub mod admin;
pub mod auth;
pub mod calendar;
pub mod home;
pub mod media;
pub mod progress;
pub mod search;
pub mod settings;
pub mod stats;
pub mod tmdb_episodes;
pub mod tmdb_image;
pub mod tracking;
