//! Integration coverage for the MAL -> Shikimori resolver's SQL layer.
//! The HTTP orchestration is unit-tested separately; here we prove the finder
//! selects exactly the tracked, unresolved anime and that a `shikimori_id`
//! update removes a row from the unresolved set.

mod common;

use mediatracker::services::anime_identity::find_unresolved_tracked_anime;
use uuid::Uuid;

const USER: &str = "test_anime_identity";
const EMAIL: &str = "test_anime_identity@example.com";
const MAL_ID: i64 = 900_001_337;
const EXTERNAL_ID: &str = "mal-identity-001";

async fn seed_user(ctx: &common::TestContext) -> Uuid {
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(USER)
        .execute(&ctx.pool)
        .await
        .expect("delete fixture user");

    sqlx::query(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash', 'user')",
    )
    .bind(USER)
    .bind(EMAIL)
    .execute(&ctx.pool)
    .await
    .expect("create fixture user");

    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(USER)
        .fetch_one(&ctx.pool)
        .await
        .expect("get user id")
}

async fn seed_unresolved_anime(ctx: &common::TestContext, user: Uuid) -> Uuid {
    let media_id: Uuid = sqlx::query_scalar(
        "INSERT INTO media_items (provider, external_id, media_type, title, mal_id, shikimori_id) \
         VALUES ('mal', $1, 'anime', 'Identity Anime', $2, NULL) \
         ON CONFLICT (provider, external_id) DO UPDATE SET mal_id = EXCLUDED.mal_id, \
             shikimori_id = NULL \
         RETURNING id",
    )
    .bind(EXTERNAL_ID)
    .bind(MAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("upsert media item");

    sqlx::query(
        "INSERT INTO tracking_entries (user_id, media_id, status) VALUES ($1, $2, 'in_progress') \
         ON CONFLICT (user_id, media_id) DO UPDATE SET status = EXCLUDED.status",
    )
    .bind(user)
    .bind(media_id)
    .execute(&ctx.pool)
    .await
    .expect("upsert tracking entry");

    media_id
}

#[tokio::test]
async fn finder_lists_unresolved_tracked_anime_then_excludes_resolved() {
    let ctx = common::TestContext::new().await;
    let user = seed_user(&ctx).await;
    let media_id = seed_unresolved_anime(&ctx, user).await;

    let found = find_unresolved_tracked_anime(&ctx.pool, 50)
        .await
        .expect("finder must not fail");
    assert!(
        found.iter().any(|c| c.id == media_id && c.mal_id == MAL_ID),
        "unresolved tracked anime must be found"
    );

    // Simulate the resolver persisting the Shikimori id.
    sqlx::query("UPDATE media_items SET shikimori_id = $1 WHERE id = $2")
        .bind(MAL_ID)
        .bind(media_id)
        .execute(&ctx.pool)
        .await
        .expect("persist shikimori_id");

    let after = find_unresolved_tracked_anime(&ctx.pool, 50)
        .await
        .expect("finder must not fail");
    assert!(
        !after.iter().any(|c| c.id == media_id),
        "resolved anime must drop out of the unresolved set"
    );
}
