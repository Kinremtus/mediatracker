// Health endpoint test: exercises the REAL handler that src/main.rs serves.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_ok_json() {
    let app = Router::new().route("/health", get(mediatracker::routes::health_check));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(&body[..], &br#"{"status":"ok"}"#[..]);
}
