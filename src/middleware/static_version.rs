use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderValue},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

/// Short git hash baked into the binary at build time. Used as a
/// cache-bust query string for `/static/*` assets so the browser
/// fetches the new JS/HTML after every deploy.
pub const STATIC_VERSION: &str = include_str!(concat!(env!("OUT_DIR"), "/static_version.txt"));

/// Appends `?v=<git_hash>` to every `/static/js/*.js` reference in
/// HTML responses. Bypasses the nginx/browser `public, immutable`
/// 30-day cache on the static directory.
pub async fn static_version_middleware(
    State(_state): State<Arc<()>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path().to_string();
    let is_static_js = path.starts_with("/static/js/") && path.ends_with(".js");
    let is_static_css = path.starts_with("/static/css/") && path.ends_with(".css");

    let mut response = next.run(req).await;

    if is_static_js || is_static_css {
        // For the static assets themselves: tell the browser to revalidate.
        // Drop the long `immutable` cache headers and let the browser ask.
        response.headers_mut().remove(header::CACHE_CONTROL);
        response.headers_mut().remove(header::EXPIRES);
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache, must-revalidate"));
    } else {
        // For HTML responses: append cache-bust query string to /static/* references.
        let ct = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !ct.starts_with("text/html") {
            return response;
        }

        let body = std::mem::replace(response.body_mut(), Body::empty());
        let bytes = match axum::body::to_bytes(body, 4 * 1024 * 1024).await {
            Ok(b) => b,
            Err(_) => return response,
        };
        let html = match std::str::from_utf8(&bytes) {
            Ok(s) => s.to_string(),
            Err(_) => return response,
        };

        let v = STATIC_VERSION.trim();
        let patched = html
            .replace("\"/static/js/", &format!("\"/static/js/"))
            // We want to keep simple "…/app.js" → "…/app.js?v=hash".
            .replace("app.js\"", &format!("app.js?v={v}\""))
            .replace("htmx.min.js\"", &format!("htmx.min.js?v={v}\""))
            .replace("alpine.min.js\"", &format!("alpine.min.js?v={v}\""))
            .replace("main.css\"", &format!("main.css?v={v}\""))
            .replace("components.css\"", &format!("components.css?v={v}\""))
            .replace("animations.css\"", &format!("animations.css?v={v}\""))
            .replace("utilities.css\"", &format!("utilities.css?v={v}\""));

        let new_len = patched.len();
        *response.body_mut() = Body::from(patched);
        response.headers_mut().insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&new_len.to_string()).unwrap(),
        );
        // HTML itself should not be cached aggressively either.
        response.headers_mut().remove(header::CACHE_CONTROL);
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache, must-revalidate"));
    }

    response
}
