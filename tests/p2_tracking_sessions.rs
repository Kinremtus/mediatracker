//! Regression tests for P2 remediation:
//!   P2-F: the batched `is_tracked` lookup must return only the pairs the user
//!         actually tracks (replaces the per-result N+1 query in search).
//!   P2-K: changing a password must invalidate every *other* session while the
//!         current one keeps working (stolen cookies die with the old password).

mod common;

use mediatracker::services::auth::hash_token;
use uuid::Uuid;

const USER: &str = "test_p2_tracking_sessions";
const EMAIL: &str = "p2@example.com";

async fn create_user(ctx: &common::TestContext) -> Uuid {
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

#[tokio::test]
async fn find_tracked_media_returns_only_tracked_pairs() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx).await;

    // Two media items; the user tracks only the first.
    let tracked_id: Uuid = sqlx::query_scalar(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ('tmdb', 'p2-tracked', 'movie', 'Tracked') RETURNING id",
    )
    .fetch_one(&ctx.pool)
    .await
    .expect("insert tracked media");

    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ('tmdb', 'p2-untracked', 'movie', 'Untracked')",
    )
    .execute(&ctx.pool)
    .await
    .expect("insert untracked media");

    sqlx::query(
        "INSERT INTO tracking_entries (user_id, media_id, status) \
         VALUES ($1, $2, 'in_progress')",
    )
    .bind(user)
    .bind(tracked_id)
    .execute(&ctx.pool)
    .await
    .expect("insert tracking entry");

    let pairs = vec![
        ("tmdb".to_string(), "p2-tracked".to_string()),
        ("tmdb".to_string(), "p2-untracked".to_string()),
        ("tmdb".to_string(), "p2-missing".to_string()),
    ];
    let found = ctx
        .state
        .tracking
        .find_tracked_media(user, &pairs)
        .await
        .expect("batch tracked lookup");

    assert_eq!(found.len(), 1, "only the tracked pair should be returned");
    assert!(found.contains(&("tmdb".to_string(), "p2-tracked".to_string())));
}

#[tokio::test]
async fn delete_other_sessions_keeps_current() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx).await;

    let current = hash_token("current-token");
    let other = hash_token("other-token");
    for token_hash in [&current, &other] {
        sqlx::query(
            "INSERT INTO sessions (user_id, token_hash, expires_at) \
             VALUES ($1, $2, NOW() + INTERVAL '1 day')",
        )
        .bind(user)
        .bind(token_hash)
        .execute(&ctx.pool)
        .await
        .expect("insert session");
    }

    let removed = ctx
        .state
        .auth
        .delete_other_sessions(user, &current)
        .await
        .expect("delete other sessions");
    assert_eq!(removed, 1, "exactly the other session must be removed");

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE user_id = $1")
        .bind(user)
        .fetch_one(&ctx.pool)
        .await
        .expect("count sessions");
    assert_eq!(remaining, 1, "only the current session must remain");

    let kept: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE user_id = $1 AND token_hash = $2)",
    )
    .bind(user)
    .bind(&current)
    .fetch_one(&ctx.pool)
    .await
    .expect("check current session survives");
    assert!(kept, "the current session must survive");
}
