//! P2-S: route degradation tests.
//!
//! These guard behaviour that used to be fatal or silent:
//! - the calendar used to break with a Postgres `42P10` error (P1-3) whenever
//!   `release_schedule` was read; rendering an empty month must still be a 200.
//! - the search page must render an empty result set without external
//!   providers (all API keys are empty in tests), i.e. no network is required.
//!
//! The calendar test seeds one fresh `release_schedule` row so
//! `ensure_fresh()` sees fresh data and never calls Shikimori. Without the
//! seed the request would try the network, making the test slow and flaky.

mod common;

use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use mediatracker::middleware::auth::CurrentUser;
use mediatracker::routes::{calendar, search};
use tower::ServiceExt;
use uuid::Uuid;

fn current_user() -> CurrentUser {
    CurrentUser {
        id: Uuid::new_v4(),
        username: "degradation".to_string(),
        role: "user".to_string(),
    }
}

async fn get_page(app: Router, uri: &str) -> StatusCode {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    response.status()
}

#[tokio::test]
async fn search_page_renders_without_query_or_providers() {
    let ctx = common::TestContext::new().await;
    let app = Router::new()
        .route("/search", get(search::get_search))
        .layer(Extension(current_user()))
        .with_state(ctx.state.clone());

    assert_eq!(get_page(app, "/search").await, StatusCode::OK);
}

#[tokio::test]
async fn search_suggestions_render_without_query_or_providers() {
    let ctx = common::TestContext::new().await;
    let app = Router::new()
        .route("/search/suggestions", get(search::get_search_suggestions))
        .layer(Extension(current_user()))
        .with_state(ctx.state.clone());

    assert_eq!(get_page(app, "/search/suggestions").await, StatusCode::OK);
}

#[tokio::test]
async fn calendar_page_renders_empty_month_without_network() {
    let ctx = common::TestContext::new().await;

    // A fresh row makes `release_schedule::ensure_fresh()` skip the Shikimori
    // refresh entirely, so this test stays offline and deterministic.
    sqlx::query(
        "INSERT INTO release_schedule (provider, external_id, episode_number, air_date, title)
         VALUES ('shikimori', 'degradation-test', 1, NOW(), 'Test Anime')",
    )
    .execute(&ctx.pool)
    .await
    .expect("seed release_schedule");

    let app = Router::new()
        .route("/calendar", get(calendar::get_calendar))
        .layer(Extension(current_user()))
        .with_state(ctx.state.clone());

    assert_eq!(
        get_page(app, "/calendar?year=2026&month=9").await,
        StatusCode::OK
    );
}
