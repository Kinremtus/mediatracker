use axum::{routing::get, Json, Router};
use serde_json::json;
use tower::ServiceExt;
use axum::http::{Request, StatusCode};
use axum::body::Body;

async fn health_check() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

#[tokio::test]
async fn health_returns_ok() {
    let app = Router::new()
        .route("/health", get(health_check));

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
}
