//! HTTP-level tests for the admin chapter-management endpoints.
mod common;

use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::{get, post},
};
use mediatracker::app_state::AppState;
use mediatracker::middleware::auth::CurrentUser;
use mediatracker::routes::admin;
use tower::ServiceExt;
use uuid::Uuid;

const PROVIDER: &str = "mangaupdates";
const EXTERNAL_ID: &str = "999999994";
const TITLE: &str = "Admin Chapter Fixture";

async fn seed_user(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query("DELETE FROM users WHERE username = 'admin_chapters_test'")
        .execute(pool)
        .await
        .expect("delete user");
    sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ('admin_chapters_test', 'admin_chapters_test@example.com', 'fakehash', 'admin') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("seed user")
}

fn current_user(id: Uuid) -> CurrentUser {
    CurrentUser {
        id,
        username: "admin_chapters_test".to_string(),
        role: "admin".to_string(),
    }
}

fn app(state: AppState, user: CurrentUser) -> Router {
    Router::new()
        .route("/admin", get(admin::get_admin_panel))
        .route("/admin/chapters/manual", post(admin::post_chapter_manual))
        .route("/admin/chapters/reset", post(admin::post_chapter_reset))
        .route("/admin/chapters/bind", post(admin::post_chapter_bind))
        .route("/admin/chapters/unbind", post(admin::post_chapter_unbind))
        .route("/admin/chapters/refresh", post(admin::post_chapter_refresh))
        .layer(Extension(user))
        .with_state(state)
}

async fn seed_item(ctx: &common::TestContext) {
    sqlx::query("DELETE FROM media_source_ids WHERE provider = $1 AND external_id = $2")
        .bind(PROVIDER)
        .bind(EXTERNAL_ID)
        .execute(&ctx.pool)
        .await
        .expect("delete stale bindings");
    sqlx::query("DELETE FROM media_items WHERE provider = $1 AND external_id = $2")
        .bind(PROVIDER)
        .bind(EXTERNAL_ID)
        .execute(&ctx.pool)
        .await
        .expect("delete stale item");
    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title, chapters) \
         VALUES ($1, $2, 'manhwa', $3, 145)",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .bind(TITLE)
    .execute(&ctx.pool)
    .await
    .expect("insert item");
}

fn post_form(uri: &str, body: String) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    String::from_utf8_lossy(&bytes).to_string()
}

async fn state_of(ctx: &common::TestContext) -> (Option<i32>, bool) {
    sqlx::query_as(
        "SELECT chapters, chapters_manual FROM media_items \
                    WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("read state")
}

#[tokio::test]
async fn search_renders_matching_item() {
    let ctx = common::TestContext::new().await;
    let uid = seed_user(&ctx.pool).await;
    seed_item(&ctx).await;

    let resp = app(ctx.state.clone(), current_user(uid))
        .oneshot(
            Request::builder()
                .uri("/admin?q=Admin+Chapter+Fixture")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    assert!(html.contains(TITLE), "result row must render the title");
    assert!(html.contains("145"), "result row must render the count");
}

#[tokio::test]
async fn manual_set_persists_flashes_and_keeps_query() {
    let ctx = common::TestContext::new().await;
    let uid = seed_user(&ctx.pool).await;
    seed_item(&ctx).await;

    let body = format!(
        "provider={PROVIDER}&external_id={EXTERNAL_ID}&chapters=157&q=Admin+Chapter+Fixture"
    );
    let resp = app(ctx.state.clone(), current_user(uid))
        .oneshot(post_form("/admin/chapters/manual", body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    assert!(html.contains("157"), "flash must show the new count");
    assert!(
        html.contains("value=\"Admin Chapter Fixture\""),
        "search query must be preserved in the re-rendered form"
    );

    let (ch, manual) = state_of(&ctx).await;
    assert_eq!(ch, Some(157));
    assert!(manual, "manual pin must set chapters_manual");
}

#[tokio::test]
async fn reset_clears_manual_flag() {
    let ctx = common::TestContext::new().await;
    let uid = seed_user(&ctx.pool).await;
    seed_item(&ctx).await;
    sqlx::query(
        "UPDATE media_items SET chapters = 157, chapters_manual = TRUE, chapters_source = 'manual' \
                WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed manual");

    let body = format!("provider={PROVIDER}&external_id={EXTERNAL_ID}&q=");
    let resp = app(ctx.state.clone(), current_user(uid))
        .oneshot(post_form("/admin/chapters/reset", body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let (_, manual) = state_of(&ctx).await;
    assert!(!manual, "reset must clear chapters_manual");
}

#[tokio::test]
async fn unbind_removes_binding() {
    let ctx = common::TestContext::new().await;
    let uid = seed_user(&ctx.pool).await;
    seed_item(&ctx).await;
    sqlx::query(
        "INSERT INTO media_source_ids (provider, external_id, source, source_id) \
                VALUES ($1, $2, 'kakao', '64096846')",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed binding");

    let body = format!("provider={PROVIDER}&external_id={EXTERNAL_ID}&source=kakao&q=");
    let resp = app(ctx.state.clone(), current_user(uid))
        .oneshot(post_form("/admin/chapters/unbind", body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM media_source_ids WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("count bindings");
    assert_eq!(count, 0, "binding must be removed");
}

#[tokio::test]
async fn refresh_without_binding_is_noop() {
    let ctx = common::TestContext::new().await;
    let uid = seed_user(&ctx.pool).await;
    seed_item(&ctx).await;

    let body = format!("provider={PROVIDER}&external_id={EXTERNAL_ID}&q=");
    let resp = app(ctx.state.clone(), current_user(uid))
        .oneshot(post_form("/admin/chapters/refresh", body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    assert!(html.contains("без изменений"));

    let (ch, _) = state_of(&ctx).await;
    assert_eq!(
        ch,
        Some(145),
        "no binding => provider skipped, count untouched"
    );
}

#[tokio::test]
async fn bind_rejects_unknown_source() {
    let ctx = common::TestContext::new().await;
    let uid = seed_user(&ctx.pool).await;
    seed_item(&ctx).await;

    let body = format!("provider={PROVIDER}&external_id={EXTERNAL_ID}&source=bogus&source_id=x&q=");
    let resp = app(ctx.state.clone(), current_user(uid))
        .oneshot(post_form("/admin/chapters/bind", body))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    assert!(html.contains("Неизвестный источник"));

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM media_source_ids WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("count bindings");
    assert_eq!(count, 0, "unknown source must not be persisted");
}
