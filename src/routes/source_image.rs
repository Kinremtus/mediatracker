//! `/src-image/{host}/{path}` — server-side fetch of an official-source cover.
//!
//! The browser cannot load some publisher CDNs directly (hotlink protection,
//! `application/octet-stream`), so official hits store their cover through
//! this proxy (see [`crate::services::source_images`]). The allowlist there is
//! also the SSRF guard: only the listed hosts are fetched, only over https,
//! with the referer the host expects. The body is capped so a hostile answer
//! cannot stream forever.

use axum::{
    extract::{OriginalUri, Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

use crate::app_state::AppState;
use crate::services::source_images;

/// Hard cap on one proxied image (covers are a few hundred KiB).
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// One week, like `/tmdb-image/`.
const CACHE_CONTROL: &str = "public, max-age=604800";

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                  (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

pub async fn get_source_image(
    State(state): State<AppState>,
    Path((host, path)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
) -> Response {
    let Some(referer) = source_images::referer_for(&host) else {
        return (StatusCode::BAD_REQUEST, "image host not allowed").into_response();
    };
    // Rebuild the upstream URL; the query string rides along untouched.
    let mut url = format!("https://{host}/{path}");
    if let Some(query) = uri.query() {
        url.push('?');
        url.push_str(query);
    }
    let fallback_content_type = guess_content_type(&path);

    let response = state
        .http_client
        .get(&url)
        .header(reqwest::header::REFERER, referer)
        .header(reqwest::header::USER_AGENT, UA)
        .send()
        .await;

    let response = match response {
        Ok(response) => response,
        Err(e) => {
            tracing::debug!(host, error = %e, "source image fetch failed");
            return (StatusCode::BAD_GATEWAY, "image fetch failed").into_response();
        }
    };
    let status = response.status();
    if !status.is_success() {
        return (status, "image fetch failed").into_response();
    }

    // Some CDNs answer `application/octet-stream` for webp/jpg; normalize by
    // extension so the browser renders instead of downloading.
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .filter(|ct| ct.starts_with("image/"))
        .unwrap_or_else(|| fallback_content_type.to_string());

    let body = match response.bytes().await {
        Ok(body) if body.len() <= MAX_IMAGE_BYTES => body,
        Ok(_) => {
            tracing::warn!(host, "source image too large, dropped");
            return (StatusCode::BAD_GATEWAY, "image too large").into_response();
        }
        Err(e) => {
            tracing::debug!(host, error = %e, "source image body failed");
            return (StatusCode::BAD_GATEWAY, "image fetch failed").into_response();
        }
    };

    let mut headers = HeaderMap::new();
    if let Ok(value) = HeaderValue::from_str(&content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(CACHE_CONTROL),
    );
    (StatusCode::OK, headers, body).into_response()
}

/// Content type by file extension (the CDN's answer is octet-stream or absent).
fn guess_content_type(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else {
        "image/jpeg"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_by_extension() {
        assert_eq!(guess_content_type("/a/b/c.webp"), "image/webp");
        assert_eq!(guess_content_type("/a/b/c.PNG"), "image/png");
        assert_eq!(guess_content_type("/a/b/c.jpg"), "image/jpeg");
        assert_eq!(guess_content_type("/download/resource"), "image/jpeg");
    }
}
