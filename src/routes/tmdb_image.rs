use axum::{
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::app_state::AppState;
use crate::services::image_proxy;

pub async fn get_tmdb_image(
    State(state): State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Response {
    let url = format!("https://image.tmdb.org/t/p/w500/{}", path);

    // The no-redirect client never follows a redirect by itself; only
    // image.tmdb.org is accepted on every hop.
    let response = match image_proxy::fetch_checked(&state.image_client, &url, |host| {
        (host == "image.tmdb.org").then(Vec::new)
    })
    .await
    {
        image_proxy::ImageFetch::Response(response) => response,
        _ => {
            return (StatusCode::BAD_GATEWAY, "Failed to fetch image from TMDB").into_response();
        }
    };

    let status = response.status();

    let mut headers = HeaderMap::new();
    if let Some(content_type) = response.headers().get(header::CONTENT_TYPE) {
        headers.insert(header::CONTENT_TYPE, content_type.clone());
    }

    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, immutable, max-age=604800"),
    );

    let body = response.bytes().await.unwrap_or_default();

    (status, headers, body).into_response()
}
