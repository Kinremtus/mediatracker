//! Redirect-safe image fetching for the image proxies (`/src-image`,
//! `/tmdb-image`).
//!
//! The proxies accept user-controlled URLs, so reqwest must never follow
//! redirects on its own: its default policy walks up to ten hops without
//! re-checking anything, and a redirect from an allowlisted CDN to
//! `169.254.169.254` (or an in-cluster service) would be followed silently.
//! This module fetches with a no-redirect client and re-validates every hop
//! through the caller's allowlist before the request goes out.

use reqwest::header::HeaderName;

/// Hops followed before giving up.
pub const MAX_HOPS: usize = 4;

/// Outcome of a redirect-checked fetch.
pub enum ImageFetch {
    /// A final (non-redirect) response from an allowlisted host.
    Response(reqwest::Response),
    /// A hop pointed outside the allowlist — the SSRF guard tripped.
    Rejected,
    /// More redirects than [`MAX_HOPS`].
    TooManyHops,
    /// Transport-level failure.
    Request(reqwest::Error),
}

/// Fetch `url` with a no-redirect client, following redirects manually.
///
/// `headers_for` is the allowlist: it returns the headers the host requires
/// (e.g. the `Referer` a CDN expects) or `None` when the host must not be
/// fetched. It runs for the initial URL and for every redirect target, so a
/// hop leaving the allowlist is rejected before any request is sent.
pub async fn fetch_checked(
    client: &reqwest::Client,
    url: &str,
    headers_for: impl Fn(&str) -> Option<Vec<(HeaderName, String)>>,
) -> ImageFetch {
    let mut current = url.to_string();
    for _ in 0..=MAX_HOPS {
        let Some(host) = host_of(&current) else {
            return ImageFetch::Rejected;
        };
        let Some(headers) = headers_for(&host) else {
            return ImageFetch::Rejected;
        };
        let mut request = client.get(&current);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        let response = match request.send().await {
            Ok(response) => response,
            Err(e) => return ImageFetch::Request(e),
        };
        if !is_followed_redirect(response.status().as_u16()) {
            return ImageFetch::Response(response);
        }
        let Some(next) = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|location| resolve_redirect(&current, location))
        else {
            // A redirect without a usable target cannot be validated.
            return ImageFetch::Rejected;
        };
        current = next;
    }
    ImageFetch::TooManyHops
}

/// Redirect statuses the proxy follows (300/304/305/306 are returned as-is).
fn is_followed_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

/// Resolve a `Location` against the current URL; only http(s) targets pass.
fn resolve_redirect(current: &str, location: &str) -> Option<String> {
    let base = url::Url::parse(current).ok()?;
    let next = base.join(location).ok()?;
    matches!(next.scheme(), "http" | "https").then(|| next.to_string())
}

/// Lowercased host of a URL.
fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(|host| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::routing::get;

    /// Allow only the loopback test server.
    fn loopback_only(host: &str) -> Option<Vec<(HeaderName, String)>> {
        host.starts_with("127.").then(Vec::new)
    }

    async fn spawn_redirect_server() -> String {
        let app = Router::new()
            .route(
                "/a",
                get(|| async { axum::response::Redirect::temporary("/b") }),
            )
            .route("/b", get(|| async { "final" }))
            .route(
                "/away",
                get(|| async {
                    axum::response::Redirect::temporary("http://169.254.169.254/latest/meta-data")
                }),
            )
            .route(
                "/bad-scheme",
                get(|| async { axum::response::Redirect::temporary("file:///etc/passwd") }),
            )
            .route(
                "/loop",
                get(|| async { axum::response::Redirect::temporary("/loop") }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("test listener addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn follows_same_host_redirects() {
        let base = spawn_redirect_server().await;
        let client = crate::services::external::image_client().expect("image client");
        let outcome = fetch_checked(&client, &format!("{base}/a"), loopback_only).await;
        let response = match outcome {
            ImageFetch::Response(response) => response,
            _ => panic!("expected a final response"),
        };
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.text().await.expect("body"), "final");
    }

    #[tokio::test]
    async fn rejects_redirects_leaving_the_allowlist() {
        let base = spawn_redirect_server().await;
        let client = crate::services::external::image_client().expect("image client");
        // The metadata IP is rejected before any request is sent to it.
        let outcome = fetch_checked(&client, &format!("{base}/away"), loopback_only).await;
        assert!(matches!(outcome, ImageFetch::Rejected));
        let outcome = fetch_checked(&client, &format!("{base}/bad-scheme"), loopback_only).await;
        assert!(matches!(outcome, ImageFetch::Rejected));
    }

    #[tokio::test]
    async fn stops_after_the_hop_budget() {
        let base = spawn_redirect_server().await;
        let client = crate::services::external::image_client().expect("image client");
        let outcome = fetch_checked(&client, &format!("{base}/loop"), loopback_only).await;
        assert!(matches!(outcome, ImageFetch::TooManyHops));
    }

    #[test]
    fn resolves_relative_and_absolute_locations() {
        assert_eq!(
            resolve_redirect("https://cdn.example/a/b.jpg", "/c/d.jpg").as_deref(),
            Some("https://cdn.example/c/d.jpg")
        );
        assert_eq!(
            resolve_redirect("https://cdn.example/a/", "b.jpg").as_deref(),
            Some("https://cdn.example/a/b.jpg")
        );
        assert_eq!(
            resolve_redirect("https://cdn.example/a", "https://other.example/x").as_deref(),
            Some("https://other.example/x")
        );
        assert_eq!(
            resolve_redirect("https://cdn.example/a", "file:///etc/passwd"),
            None
        );
        assert_eq!(
            resolve_redirect("https://cdn.example/a", "javascript:alert(1)"),
            None
        );
    }

    #[test]
    fn host_is_lowercased() {
        assert_eq!(
            host_of("https://CDN.Example.COM/a").as_deref(),
            Some("cdn.example.com")
        );
        assert_eq!(host_of("not a url"), None);
    }
}
