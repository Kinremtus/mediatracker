//! Security response headers applied to every response.
//!
//! All headers previously came from `nginx.conf`, but nginx is not used in
//! k3s (Cloudflare Tunnel -> Traefik -> app), so they are applied here. A
//! handler that already set a header wins: this middleware never overwrites
//! an existing value.

use axum::{
    extract::Request,
    http::{
        HeaderMap,
        header::{
            CONTENT_SECURITY_POLICY, HeaderName, HeaderValue, REFERRER_POLICY,
            STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
        },
    },
    middleware::Next,
    response::Response,
};

/// Content-Security-Policy matching the current HTMX + Alpine.js frontend.
///
/// `script-src`/`style-src` allow inline code because the templates embed
/// inline scripts and styles. Tighten these (nonce or externalise) once the
/// templates no longer rely on them; that is a follow-up, not a P1 blocker.
pub const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'none'; img-src 'self' data: https:; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; font-src 'self' data:; connect-src 'self'";

/// Security headers and their values.
fn security_header_pairs() -> [(HeaderName, &'static str); 6] {
    [
        (CONTENT_SECURITY_POLICY, CONTENT_SECURITY_POLICY_VALUE),
        (X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (X_FRAME_OPTIONS, "DENY"),
        (REFERRER_POLICY, "strict-origin-when-cross-origin"),
        (
            STRICT_TRANSPORT_SECURITY,
            "max-age=31536000; includeSubDomains",
        ),
        (
            // `PERMISSIONS_POLICY` is not exposed by `http::header`, so build it.
            HeaderName::from_static("permissions-policy"),
            "geolocation=(), microphone=(), camera=(), payment=()",
        ),
    ]
}

/// Insert the security headers unless a handler already provided them.
pub fn apply_security_headers(headers: &mut HeaderMap) {
    for (name, value) in security_header_pairs() {
        if !headers.contains_key(&name)
            && let Ok(value) = HeaderValue::from_str(value)
        {
            headers.insert(name, value);
        }
    }
}

/// Middleware wrapper: add security headers to every response.
pub async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    apply_security_headers(response.headers_mut());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn adds_all_headers_when_absent() {
        let mut headers = HeaderMap::new();
        apply_security_headers(&mut headers);

        assert_eq!(
            headers.get(&CONTENT_SECURITY_POLICY).unwrap(),
            CONTENT_SECURITY_POLICY_VALUE
        );
        assert_eq!(headers.get(&X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
        assert_eq!(headers.get(&X_FRAME_OPTIONS).unwrap(), "DENY");
        assert_eq!(
            headers.get(&REFERRER_POLICY).unwrap(),
            "strict-origin-when-cross-origin"
        );
        assert!(headers.contains_key(&STRICT_TRANSPORT_SECURITY));
        assert!(headers.contains_key(HeaderName::from_static("permissions-policy")));
        assert!(CONTENT_SECURITY_POLICY_VALUE.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn does_not_overwrite_existing_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'none'"),
        );
        apply_security_headers(&mut headers);

        assert_eq!(
            headers.get(&CONTENT_SECURITY_POLICY).unwrap(),
            "default-src 'none'"
        );
        // Other headers are still added when absent.
        assert_eq!(headers.get(&X_FRAME_OPTIONS).unwrap(), "DENY");
    }
}
