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

pub mod admin;
pub mod auth;
pub mod calendar;
pub mod home;
pub mod media;
pub mod search;
pub mod settings;
pub mod stats;
pub mod tmdb_episodes;
pub mod tmdb_image;
pub mod tracking;
