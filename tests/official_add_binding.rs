//! Adding a title straight from an official source binds it to itself: the
//! drawer's enrich looks up `media_source_ids`, so without the self-binding
//! row an official item would never get its chapter list.

mod common;

use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::post,
};
use mediatracker::app_state::AppState;
use mediatracker::middleware::auth::CurrentUser;
use mediatracker::routes::tracking;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

async fn seed_user(pool: &PgPool, username: &str) -> Uuid {
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(username)
        .execute(pool)
        .await
        .expect("delete fixture user");
    sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash_official_add_test', 'user') RETURNING id",
    )
    .bind(username)
    .bind(format!("{username}@example.com"))
    .fetch_one(pool)
    .await
    .expect("create fixture user")
}

fn app(state: AppState, user_id: Uuid) -> Router {
    Router::new()
        .route("/tracking", post(tracking::post_add_to_tracking))
        .layer(Extension(CurrentUser {
            id: user_id,
            username: "official-add-test".to_string(),
            role: "user".to_string(),
            default_search_type: None,
        }))
        .with_state(state)
}

async fn post_tracking(app: Router, body: &'static str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/tracking")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .expect("request"),
    )
    .await
    .expect("response")
}

#[tokio::test]
async fn official_add_creates_the_self_binding() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "official-add-test").await;

    // "other-comics" has no chapter-count providers, so the inline enrich is a
    // no-op and the test stays offline.
    let resp = post_tracking(
        app(ctx.state.clone(), user_id),
        "provider=webtoons&external_id=1320&media_type=other-comics&title=Lore+Olympus\
         &tracking_status=planned&redirect_to=%2Ftracking",
    )
    .await;
    assert!(
        resp.status().is_redirection(),
        "the add must redirect, got {}",
        resp.status()
    );

    let binding: Option<String> = sqlx::query_scalar(
        "SELECT source_id FROM media_source_ids \
         WHERE provider = 'webtoons' AND external_id = '1320' AND source = 'webtoons'",
    )
    .fetch_optional(&ctx.pool)
    .await
    .expect("read binding");
    assert_eq!(
        binding.as_deref(),
        Some("1320"),
        "the official item binds to its own source"
    );

    let title: Option<String> = sqlx::query_scalar(
        "SELECT title FROM media_items WHERE provider = 'webtoons' AND external_id = '1320'",
    )
    .fetch_optional(&ctx.pool)
    .await
    .expect("read item");
    assert_eq!(title.as_deref(), Some("Lore Olympus"));
}

#[tokio::test]
async fn non_official_add_creates_no_binding() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "official-add-test").await;

    // A `manual` provider is not an official source (and has no detail
    // provider to fetch), so no binding row may appear.
    let resp = post_tracking(
        app(ctx.state.clone(), user_id),
        "provider=manual&external_id=fixture-1&media_type=manga&title=Some+Manga\
         &tracking_status=planned&redirect_to=%2Ftracking",
    )
    .await;
    assert!(resp.status().is_redirection(), "got {}", resp.status());

    let bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM media_source_ids \
         WHERE provider = 'manual' AND external_id = 'fixture-1'",
    )
    .fetch_one(&ctx.pool)
    .await
    .expect("count bindings");
    assert_eq!(bindings, 0);
}

#[tokio::test]
async fn official_add_ignores_a_foreign_chain() {
    // `webtoons` is not in the `manga` chain: no binding is created there.
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "official-add-test").await;

    let resp = post_tracking(
        app(ctx.state.clone(), user_id),
        "provider=webtoons&external_id=9999&media_type=manga&title=Odd+Chain\
         &tracking_status=planned&redirect_to=%2Ftracking",
    )
    .await;
    assert!(resp.status().is_redirection(), "got {}", resp.status());

    let bindings: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM media_source_ids \
         WHERE provider = 'webtoons' AND external_id = '9999'",
    )
    .fetch_one(&ctx.pool)
    .await
    .expect("count bindings");
    assert_eq!(bindings, 0);
}

#[test]
fn official_chain_membership_is_the_gate() {
    // Sanity: webtoons serves other-comics and manhwa, not manga.
    use mediatracker::services::official_resolve::official_source_for_media_type;
    assert!(official_source_for_media_type("other-comics").contains(&"webtoons"));
    assert!(official_source_for_media_type("manhwa").contains(&"webtoons"));
    assert!(!official_source_for_media_type("manga").contains(&"webtoons"));
    assert_eq!(StatusCode::SEE_OTHER.as_u16(), 303);
}
