//! P2-S: page handlers must fail loudly.
//!
//! Guards the P2-J / P2-M behaviour: when a database call fails, a page must
//! answer 500 instead of silently rendering an empty page as if the user had
//! no data. The pool is closed before the request so every query returns
//! `PoolClosed`, which is exactly what a broken database looks like to the
//! handler.

mod common;

use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use mediatracker::middleware::auth::CurrentUser;
use mediatracker::routes::{stats, tracking};
use tower::ServiceExt;
use uuid::Uuid;

fn current_user() -> CurrentUser {
    CurrentUser {
        id: Uuid::new_v4(),
        username: "db-failure".to_string(),
        role: "user".to_string(),
    }
}

async fn get_page(app: Router, uri: &str) -> StatusCode {
    let response = app
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    response.status()
}

#[tokio::test]
async fn tracking_page_returns_500_when_db_is_down() {
    let ctx = common::TestContext::new().await;
    let app = Router::new()
        .route("/tracking", get(tracking::get_tracking_list))
        .layer(Extension(current_user()))
        .with_state(ctx.state.clone());

    ctx.state.db.close().await;

    assert_eq!(
        get_page(app, "/tracking").await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[tokio::test]
async fn stats_page_returns_500_when_db_is_down() {
    let ctx = common::TestContext::new().await;
    let app = Router::new()
        .route("/stats", get(stats::get_stats))
        .layer(Extension(current_user()))
        .with_state(ctx.state.clone());

    ctx.state.db.close().await;

    assert_eq!(
        get_page(app, "/stats").await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
}
