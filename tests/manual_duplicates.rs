//! Интеграционные тесты дедупликации при ручном добавлении (Batch 5 / T5.1).
//!
//! `post_manual_add` блокирует ручное добавление, если в `media_items`
//! уже есть строка с тем же `media_type` и case-insensitive/trimmed title,
//! и редиректит на существующую карточку с `?flash=duplicate`.

mod common;

use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode, header},
    routing::get,
};
use mediatracker::app_state::AppState;
use mediatracker::middleware::auth::CurrentUser;
use mediatracker::models::media_item::CreateMediaItem;
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
         VALUES ($1, $2, 'fakehash_manual_duplicates_test', 'user') RETURNING id",
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
        username: "manual-dup-test".to_string(),
        role: "user".to_string(),
    }
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

fn api_item(provider: &str, external_id: &str, title: &str, media_type: &str) -> CreateMediaItem {
    CreateMediaItem {
        provider: provider.to_string(),
        external_id: external_id.to_string(),
        media_type: media_type.to_string(),
        title: title.to_string(),
        ..Default::default()
    }
}

fn app(state: AppState, user_id: Uuid) -> Router {
    Router::new()
        .route(
            "/tracking/manual",
            get(tracking::get_manual_form).post(tracking::post_manual_add),
        )
        .layer(Extension(current_user(user_id)))
        .with_state(state)
}

fn post_form(uri: &str, body: &'static str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

fn location(resp: &axum::response::Response) -> String {
    resp.headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn manual_add_blocks_manual_duplicate() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "dup_manual_user").await;

    let item = manual_item("Dup Title", "movie");
    ctx.state
        .tracking
        .add_to_list(user_id, &item, "planned")
        .await
        .expect("seed manual row");

    let resp = app(ctx.state.clone(), user_id)
        .oneshot(post_form(
            "/tracking/manual",
            "title=Dup+Title&media_type=movie&tracking_status=planned",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let loc = location(&resp);
    assert!(
        loc.starts_with("/media/manual/"),
        "location must point at the existing card, got {loc}"
    );
    assert!(
        loc.contains("flash=duplicate"),
        "location must carry duplicate flash, got {loc}"
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM media_items \
         WHERE media_type = 'movie' AND lower(btrim(title)) = lower(btrim($1))",
    )
    .bind("Dup Title")
    .fetch_one(&ctx.pool)
    .await
    .expect("count");
    assert_eq!(count, 1, "no second manual row may be created");
}

#[tokio::test]
async fn manual_add_blocks_duplicate_of_api_row() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "dup_api_user").await;

    let item = api_item("comicvine", "777", "API Comic", "comic");
    ctx.state
        .tracking
        .add_to_list(user_id, &item, "planned")
        .await
        .expect("seed comicvine row");

    let resp = app(ctx.state.clone(), user_id)
        .oneshot(post_form(
            "/tracking/manual",
            "title=API+Comic&media_type=comic&tracking_status=planned",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let loc = location(&resp);
    assert!(
        loc.starts_with("/media/comicvine/777"),
        "location must point at the API card, got {loc}"
    );
    assert!(
        loc.contains("flash=duplicate"),
        "location must carry duplicate flash, got {loc}"
    );

    let manual_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM media_items WHERE provider = 'manual'")
            .fetch_one(&ctx.pool)
            .await
            .expect("count");
    assert_eq!(manual_count, 0, "no manual row may shadow the API row");
}

#[tokio::test]
async fn manual_add_allows_same_title_with_different_media_type() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "dup_type_user").await;

    let movie = manual_item("Same Name", "movie");
    ctx.state
        .tracking
        .add_to_list(user_id, &movie, "planned")
        .await
        .expect("seed manual movie row");

    let resp = app(ctx.state.clone(), user_id)
        .oneshot(post_form(
            "/tracking/manual",
            "title=Same+Name&media_type=anime&tracking_status=planned",
        ))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let loc = location(&resp);
    assert!(
        loc.starts_with("/media/manual/"),
        "a new manual row must be created, got {loc}"
    );
    assert!(
        !loc.contains("flash=duplicate"),
        "different media_type is not a duplicate, got {loc}"
    );
    assert_ne!(
        loc,
        format!("/media/manual/{}", movie.external_id),
        "must not redirect to the existing movie row"
    );

    let anime_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM media_items \
         WHERE media_type = 'anime' AND lower(btrim(title)) = lower(btrim($1))",
    )
    .bind("Same Name")
    .fetch_one(&ctx.pool)
    .await
    .expect("count anime");
    assert_eq!(anime_count, 1, "new anime row must be created");

    let movie_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM media_items \
         WHERE media_type = 'movie' AND lower(btrim(title)) = lower(btrim($1))",
    )
    .bind("Same Name")
    .fetch_one(&ctx.pool)
    .await
    .expect("count movie");
    assert_eq!(movie_count, 1, "original movie row must be untouched");
}
