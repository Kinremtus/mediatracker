use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};

use crate::app_state::AppState;

pub async fn get_tmdb_image(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Response {
    let url = format!("https://image.tmdb.org/t/p/w500/{}", path);

    match state.http_client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();

            let mut headers = HeaderMap::new();
            if let Some(content_type) = resp.headers().get(header::CONTENT_TYPE) {
                headers.insert(header::CONTENT_TYPE, content_type.clone());
            }

            headers.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, immutable, max-age=604800"),
            );

            let body = resp.bytes().await.unwrap_or_default();

            (status, headers, body).into_response()
        }
        Err(_) => {
            (StatusCode::BAD_GATEWAY, "Failed to fetch image from TMDB").into_response()
        }
    }
}
