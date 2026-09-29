//! Synthetic-episode fallback: when the provider returns an empty
//! episode list but the count is known, we fill 1..=count without
//! clobbering real rows.

mod common;

use mediatracker::services::episodes::{get_episodes, store_synthetic_episodes};
use uuid::Uuid;

const MAL_ID: i64 = 999_999_993;
const FIXTURE_USERNAME: &str = "test_synthetic_episodes_user";

fn mal_str() -> String {
    MAL_ID.to_string()
}

async fn setup() -> (common::TestContext, Uuid) {
    let ctx = common::TestContext::new().await;

    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .execute(&ctx.pool)
        .await
        .expect("delete fixture user");

    sqlx::query(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash', 'user')",
    )
    .bind(FIXTURE_USERNAME)
    .bind("<SECRET: MailDetector>")
    .execute(&ctx.pool)
    .await
    .expect("create fixture user");

    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .fetch_one(&ctx.pool)
        .await
        .expect("get user id");

    sqlx::query("DELETE FROM anime_episodes WHERE external_id = $1")
        .bind(mal_str())
        .execute(&ctx.pool)
        .await
        .expect("delete stale episodes");

    (ctx, user_id)
}

#[tokio::test]
async fn store_synthetic_episodes_inserts_full_range() {
    let (ctx, user_id) = setup().await;

    store_synthetic_episodes(&ctx.pool, MAL_ID, 5)
        .await
        .expect("store synthetic");

    let episodes = get_episodes(&ctx.pool, "mal", &mal_str(), user_id, "mal", &mal_str())
        .await
        .expect("read episodes");
    let numbers: Vec<i32> = episodes.iter().map(|e| e.episode_number).collect();
    assert_eq!(numbers, vec![1, 2, 3, 4, 5]);
    assert!(
        episodes
            .iter()
            .all(|e| e.title_en.is_none() && e.air_date.is_none()),
        "synthetic rows must carry no metadata"
    );
}

#[tokio::test]
async fn store_synthetic_episodes_preserves_existing_real_rows() {
    let (ctx, _user) = setup().await;

    // A real row already exists (e.g. provider returned a partial list).
    sqlx::query(
        r#"
        INSERT INTO anime_episodes
            (provider, external_id, episode_number, title_en)
        VALUES ('mal', $1, 1, 'Real Episode One')
        "#,
    )
    .bind(mal_str())
    .execute(&ctx.pool)
    .await
    .expect("insert real row");

    store_synthetic_episodes(&ctx.pool, MAL_ID, 3)
        .await
        .expect("store synthetic");

    let title: Option<String> = sqlx::query_scalar(
        "SELECT title_en FROM anime_episodes \
         WHERE provider = 'mal' AND external_id = $1 AND episode_number = 1",
    )
    .bind(mal_str())
    .fetch_one(&ctx.pool)
    .await
    .expect("read title");
    assert_eq!(
        title.as_deref(),
        Some("Real Episode One"),
        "real row must not be clobbered by the synthetic fill"
    );

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM anime_episodes WHERE provider = 'mal' AND external_id = $1",
    )
    .bind(mal_str())
    .fetch_one(&ctx.pool)
    .await
    .expect("count rows");
    assert_eq!(count, 3, "synthetic fill completes the remaining range");
}

#[tokio::test]
async fn store_synthetic_episodes_syncs_media_items_denominator() {
    let (ctx, _user) = setup().await;

    sqlx::query(
        r#"
        INSERT INTO media_items
            (provider, external_id, media_type, title, mal_id, episodes)
        VALUES ('mal', $1, 'anime', 'Synthetic Anime', $2, 0)
        "#,
    )
    .bind(mal_str())
    .bind(MAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("insert media_items");

    store_synthetic_episodes(&ctx.pool, MAL_ID, 7)
        .await
        .expect("store synthetic");

    let episodes: Option<i32> = sqlx::query_scalar(
        "SELECT episodes FROM media_items WHERE provider = 'mal' AND external_id = $1",
    )
    .bind(mal_str())
    .fetch_one(&ctx.pool)
    .await
    .expect("read denominator");
    assert_eq!(episodes, Some(7));
}
