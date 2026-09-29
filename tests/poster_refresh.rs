//! Интеграционные тесты обновления постера через `refresh_counts::update_item`
//! (Batch 5 / T5.2).
//!
//! `update_item` обновляет постер только когда новый URL непустой
//! (`COALESCE(NULLIF($10, ''), poster_url)`) и возвращает `Ok(true)` лишь
//! если хотя бы одно поле реально изменилось.

mod common;

use mediatracker::models::media_item::CreateMediaItem;
use mediatracker::services::refresh_counts::{MediaItemRow, update_item};
use sqlx::PgPool;
use uuid::Uuid;

async fn seed_user(pool: &PgPool, username: &str) -> Uuid {
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(username)
        .execute(pool)
        .await
        .expect("delete fixture user");
    sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash_poster_refresh_test', 'user') RETURNING id",
    )
    .bind(username)
    .bind(format!("{username}@example.com"))
    .fetch_one(pool)
    .await
    .expect("create fixture user")
}

async fn load_row(pool: &PgPool, external_id: &str) -> MediaItemRow {
    sqlx::query_as(
        "SELECT id, provider, external_id, media_type, episodes, chapters, volumes, pages, \
         runtime_minutes, playtime_hours, status, score \
         FROM media_items WHERE provider = 'manual' AND external_id = $1",
    )
    .bind(external_id)
    .fetch_one(pool)
    .await
    .expect("load MediaItemRow")
}

async fn poster_of(pool: &PgPool, id: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT poster_url FROM media_items WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("poster")
}

#[tokio::test]
async fn update_item_handles_poster_refresh_cases() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "poster_refresh_user").await;

    let item = CreateMediaItem {
        provider: "manual".to_string(),
        external_id: Uuid::new_v4().to_string(),
        media_type: "movie".to_string(),
        title: "Poster Refresh Movie".to_string(),
        poster_url: Some("https://old/old.jpg".to_string()),
        episodes: Some(5),
        ..Default::default()
    };
    ctx.state
        .tracking
        .add_to_list(user_id, &item, "planned")
        .await
        .expect("seed manual row");

    let row = load_row(&ctx.pool, &item.external_id).await;

    // (a) непустой постер -> обновление, остальные поля не тронуты.
    let updated = update_item(
        &ctx.pool,
        &row,
        &CreateMediaItem {
            poster_url: Some("https://new/img.jpg".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("update_item (a)");
    assert!(updated, "non-empty poster must update the row");
    assert_eq!(
        poster_of(&ctx.pool, row.id).await.as_deref(),
        Some("https://new/img.jpg"),
        "poster must be refreshed"
    );
    let (episodes, title): (Option<i32>, String) =
        sqlx::query_as("SELECT episodes, title FROM media_items WHERE id = $1")
            .bind(row.id)
            .fetch_one(&ctx.pool)
            .await
            .expect("reload");
    assert_eq!(episodes, Some(5), "other fields must stay unchanged");
    assert_eq!(title, "Poster Refresh Movie");

    // (b) пустой постер -> NULLIF-гвард, без изменений.
    let updated = update_item(
        &ctx.pool,
        &row,
        &CreateMediaItem {
            poster_url: Some(String::new()),
            ..Default::default()
        },
    )
    .await
    .expect("update_item (b)");
    assert!(!updated, "empty poster must not update the row");
    assert_eq!(
        poster_of(&ctx.pool, row.id).await.as_deref(),
        Some("https://new/img.jpg"),
        "empty poster must leave the old value intact"
    );

    // (c) все поля None -> ничего не меняется.
    let updated = update_item(&ctx.pool, &row, &CreateMediaItem::default())
        .await
        .expect("update_item (c)");
    assert!(!updated, "all-None update must be a no-op");
    assert_eq!(
        poster_of(&ctx.pool, row.id).await.as_deref(),
        Some("https://new/img.jpg"),
        "all-None update must not touch the poster"
    );
}

async fn genres_of(pool: &PgPool, id: Uuid) -> Vec<String> {
    sqlx::query_scalar("SELECT genres FROM media_items WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("genres")
}

#[tokio::test]
async fn update_item_self_heals_empty_genres() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool, "poster_refresh_genres_user").await;

    let item = CreateMediaItem {
        provider: "manual".to_string(),
        external_id: Uuid::new_v4().to_string(),
        media_type: "movie".to_string(),
        title: "Genre Heal Movie".to_string(),
        ..Default::default()
    };
    ctx.state
        .tracking
        .add_to_list(user_id, &item, "planned")
        .await
        .expect("seed manual row");

    let row = load_row(&ctx.pool, &item.external_id).await;
    assert!(genres_of(&ctx.pool, row.id).await.is_empty());

    // (a) empty genres + provider genres -> self-heal.
    let updated = update_item(
        &ctx.pool,
        &row,
        &CreateMediaItem {
            genres: vec!["Action".to_string(), "Drama".to_string()],
            ..Default::default()
        },
    )
    .await
    .expect("update_item (a)");
    assert!(updated, "empty genres must be repopulated");
    assert_eq!(
        genres_of(&ctx.pool, row.id).await,
        vec!["Action".to_string(), "Drama".to_string()]
    );

    // (b) non-empty genres must NEVER be overwritten (respect manual edits).
    let updated = update_item(
        &ctx.pool,
        &row,
        &CreateMediaItem {
            genres: vec!["Comedy".to_string()],
            ..Default::default()
        },
    )
    .await
    .expect("update_item (b)");
    assert!(!updated, "existing genres must not be overwritten");
    assert_eq!(
        genres_of(&ctx.pool, row.id).await,
        vec!["Action".to_string(), "Drama".to_string()]
    );

    // (c) empty genres + empty provider genres -> no-op.
    let updated = update_item(&ctx.pool, &row, &CreateMediaItem::default())
        .await
        .expect("update_item (c)");
    assert!(!updated, "empty -> empty genres must be a no-op");
}
