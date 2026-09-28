//! Интеграционные тесты ручного добавления (этап 3) и устойчивости
//! карточек (этап 4).
//!
//! Сеть не используется: fallback-тест опирается на ComicVine, чей
//! `get_details` делает `bail!` при пустом API-ключе (в тестах ключ пуст).

mod common;

use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
    routing::get,
};
use mediatracker::middleware::auth::CurrentUser;
use mediatracker::models::media_item::CreateMediaItem;
use mediatracker::routes::{media, tracking};
use mediatracker::services::external::dispatch::{Provider, ProviderClients};
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
         VALUES ($1, $2, 'fakehash_manual_test', 'user') RETURNING id",
    )
    .bind(username)
    .bind(format!("{username}@example.com"))
    .fetch_one(pool)
    .await
    .expect("create fixture user")
}

fn current_user(id: Uuid) -> CurrentUser {
    CurrentUser {
        id,
        username: "manual-test".to_string(),
        role: "user".to_string(),
    }
}

async fn body_string(resp: axum::response::Response) -> String {
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf8")
}

fn manual_item(title: &str, media_type: &str) -> CreateMediaItem {
    CreateMediaItem {
        provider: "manual".to_string(),
        external_id: Uuid::new_v4().to_string(),
        media_type: media_type.to_string(),
        title: title.to_string(),
        ..Default::default()
    }
}

#[tokio::test]
async fn manual_card_renders_from_db_without_network() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "manual_card_user").await;
    let item = manual_item("Handmade Movie", "movie");
    ctx.state
        .tracking
        .add_to_list(user_id, &item, "planned")
        .await
        .expect("seed manual row");

    let app = Router::new()
        .route(
            "/media/{provider}/{external_id}",
            get(media::get_media_detail),
        )
        .layer(Extension(current_user(user_id)))
        .with_state(ctx.state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .uri(format!("/media/manual/{}", item.external_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    assert!(html.contains("Handmade Movie"), "title must render from DB");
    // manual не показывает бейдж «провайдер недоступен».
    assert!(!html.contains("сохранённые данные"));
}

#[tokio::test]
async fn provider_failure_falls_back_to_stored_row() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "manual_fallback_user").await;

    let mut item = manual_item("Cached Comic", "comic");
    item.provider = "comicvine".to_string();
    item.external_id = "12345".to_string();
    ctx.state
        .tracking
        .add_to_list(user_id, &item, "planned")
        .await
        .expect("seed comicvine row");

    let app = Router::new()
        .route(
            "/media/{provider}/{external_id}",
            get(media::get_media_detail),
        )
        .layer(Extension(current_user(user_id)))
        .with_state(ctx.state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/media/comicvine/12345")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_string(resp).await;
    assert!(html.contains("Cached Comic"), "stored row must render");
    assert!(html.contains("источник"), "cache badge text");
}

#[tokio::test]
async fn missing_provider_and_no_row_is_not_found() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "manual_404_user").await;

    let app = Router::new()
        .route(
            "/media/{provider}/{external_id}",
            get(media::get_media_detail),
        )
        .layer(Extension(current_user(user_id)))
        .with_state(ctx.state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/media/comicvine/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_string(resp).await.contains("Not found"));
}

#[tokio::test]
async fn post_manual_add_persists_manual_row() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "manual_post_user").await;

    let app = Router::new()
        .route(
            "/tracking/manual",
            get(tracking::get_manual_form).post(tracking::post_manual_add),
        )
        .layer(Extension(current_user(user_id)))
        .with_state(ctx.state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tracking/manual")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "title=My+Manual+Title&media_type=movie&tracking_status=in_progress",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let (provider, external_id, title, media_type): (String, String, String, String) =
        sqlx::query_as(
            "SELECT provider, external_id, title, media_type FROM media_items WHERE title = $1",
        )
        .bind("My Manual Title")
        .fetch_one(&ctx.pool)
        .await
        .expect("manual row must exist");

    assert_eq!(provider, "manual");
    assert_eq!(media_type, "movie");
    assert_eq!(title, "My Manual Title");
    assert!(
        Uuid::parse_str(&external_id).is_ok(),
        "external_id must be a UUID"
    );
}

#[tokio::test]
async fn post_manual_add_rejects_invalid_media_type() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "manual_invalid_user").await;

    let app = Router::new()
        .route(
            "/tracking/manual",
            get(tracking::get_manual_form).post(tracking::post_manual_add),
        )
        .layer(Extension(current_user(user_id)))
        .with_state(ctx.state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tracking/manual")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "title=Bad+Type&media_type=bogus&tracking_status=planned",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        resp.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/tracking/manual?flash=error")
    );

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM media_items WHERE title = $1")
            .bind("Bad Type")
            .fetch_one(&ctx.pool)
            .await
            .expect("count");
    assert_eq!(count, 0, "invalid media_type must not be persisted");
}

#[tokio::test]
async fn manual_provider_is_unknown_to_dispatch() {
    // Контракт, на котором держится вся фоновая безопасность manual:
    // refresh_counts и admin refresh скипают его через `from_name -> None`.
    let ctx = common::TestContext::new().await;
    let clients = ProviderClients::from_state(&ctx.state);
    assert!(
        Provider::from_name(&clients, "manual").is_none(),
        "manual must stay unknown to Provider::from_name"
    );
}

#[tokio::test]
async fn post_manual_add_clears_irrelevant_metrics() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "manual_clear_metrics_user").await;

    let app = Router::new()
        .route(
            "/tracking/manual",
            get(tracking::get_manual_form).post(tracking::post_manual_add),
        )
        .layer(Extension(current_user(user_id)))
        .with_state(ctx.state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tracking/manual")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "title=Movie+With+Stray+Chapters&media_type=movie&tracking_status=planned&chapters=5&runtime_minutes=120",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let (chapters, runtime_minutes): (Option<i32>, Option<i32>) =
        sqlx::query_as("SELECT chapters, runtime_minutes FROM media_items WHERE title = $1")
            .bind("Movie With Stray Chapters")
            .fetch_one(&ctx.pool)
            .await
            .expect("manual movie row must exist");

    assert_eq!(chapters, None, "chapters must be cleared for a movie");
    assert_eq!(runtime_minutes, Some(120), "runtime_minutes must persist");
}

#[tokio::test]
async fn post_manual_add_keeps_relevant_metrics() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "manual_keep_metrics_user").await;

    let app = Router::new()
        .route(
            "/tracking/manual",
            get(tracking::get_manual_form).post(tracking::post_manual_add),
        )
        .layer(Extension(current_user(user_id)))
        .with_state(ctx.state.clone());

    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/tracking/manual")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(
                    "title=Kept+Manga+Metrics&media_type=manga&tracking_status=planned&chapters=12&volumes=3",
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let (chapters, volumes): (Option<i32>, Option<i32>) =
        sqlx::query_as("SELECT chapters, volumes FROM media_items WHERE title = $1")
            .bind("Kept Manga Metrics")
            .fetch_one(&ctx.pool)
            .await
            .expect("manual manga row must exist");

    assert_eq!(chapters, Some(12), "chapters must persist for manga");
    assert_eq!(volumes, Some(3), "volumes must persist for manga");
}
