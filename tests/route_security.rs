use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{
        HeaderName, Request, StatusCode,
        header::{
            CONTENT_SECURITY_POLICY, REFERRER_POLICY, RETRY_AFTER, X_CONTENT_TYPE_OPTIONS,
            X_FRAME_OPTIONS,
        },
    },
    middleware::{from_fn, from_fn_with_state},
    routing::{get, post},
};
use mediatracker::middleware::rate_limit::{RateLimiter, rate_limit_middleware};
use mediatracker::middleware::security_headers::{
    CONTENT_SECURITY_POLICY_VALUE, security_headers_middleware,
};
use tower::ServiceExt;

async fn ok() -> &'static str {
    "ok"
}

fn request_with_ip(method: &str, uri: &str, ip: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("cf-connecting-ip", ip)
        .body(Body::empty())
        .unwrap()
}

/// P1-9: the auth burst limit must reject the request after the burst with a
/// `429` and a `Retry-After` header, and must not penalise other clients.
#[tokio::test]
async fn rate_limit_returns_429_after_burst() {
    let limiter = RateLimiter::new(3, Duration::from_secs(60));
    let app = Router::new()
        .route("/login", post(ok))
        .route_layer(from_fn_with_state(limiter, rate_limit_middleware));

    for _ in 0..3 {
        let response = app
            .clone()
            .oneshot(request_with_ip("POST", "/login", "203.0.113.7"))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "requests within the burst must pass"
        );
    }

    let response = app
        .clone()
        .oneshot(request_with_ip("POST", "/login", "203.0.113.7"))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "request over the burst must be rejected"
    );
    assert!(
        response.headers().contains_key(RETRY_AFTER),
        "429 must carry Retry-After"
    );

    // A different client address has its own window.
    let response = app
        .oneshot(request_with_ip("POST", "/login", "198.51.100.9"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

/// P1-11: every response must carry the security headers that used to come
/// from `nginx.conf` (which is not used in k3s).
#[tokio::test]
async fn security_headers_are_present() {
    let app = Router::new()
        .route("/", get(ok))
        .layer(from_fn(security_headers_middleware));

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers.get(CONTENT_SECURITY_POLICY).unwrap(),
        CONTENT_SECURITY_POLICY_VALUE
    );
    assert_eq!(headers.get(X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
    assert_eq!(headers.get(X_FRAME_OPTIONS).unwrap(), "DENY");
    assert_eq!(
        headers.get(REFERRER_POLICY).unwrap(),
        "strict-origin-when-cross-origin"
    );
    assert!(
        headers.contains_key(HeaderName::from_static("permissions-policy")),
        "Permissions-Policy must be present"
    );
}
