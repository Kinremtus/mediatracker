mod common;

use chrono::{DateTime, Utc};
use mediatracker::services::auth::hash_token;
use mediatracker::services::cleanup::run_cleanup_once;
use sqlx::PgPool;
use uuid::Uuid;

async fn seed_user(pool: &PgPool) -> Uuid {
    let suffix = Uuid::new_v4();
    sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO users (username, email, password_hash) VALUES ($1, $2, 'test-hash') RETURNING id",
    )
    .bind(format!("cleanup-{suffix}"))
    .bind(format!("cleanup-{suffix}@example.com"))
    .fetch_one(pool)
    .await
    .expect("seed user")
}

#[tokio::test]
async fn cleanup_prunes_expired_rows_and_keeps_live_ones() {
    let ctx = common::TestContext::new().await;
    let user_id = seed_user(&ctx.pool).await;

    let live_raw = format!("live-{}", Uuid::new_v4());
    let live_hash = hash_token(&live_raw);

    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, expires_at) \
         VALUES ($1, $2, NOW() - INTERVAL '1 hour')",
    )
    .bind(user_id)
    .bind(format!("expired-{}", Uuid::new_v4()))
    .execute(&ctx.pool)
    .await
    .expect("insert expired session");

    sqlx::query(
        "INSERT INTO sessions (user_id, token_hash, expires_at) \
         VALUES ($1, $2, NOW() + INTERVAL '1 day')",
    )
    .bind(user_id)
    .bind(&live_hash)
    .execute(&ctx.pool)
    .await
    .expect("insert live session");

    sqlx::query(
        "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) \
         VALUES ($1, $2, NOW() - INTERVAL '1 hour')",
    )
    .bind(user_id)
    .bind(format!("reset-expired-{}", Uuid::new_v4()))
    .execute(&ctx.pool)
    .await
    .expect("insert expired reset token");

    sqlx::query(
        "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at, used_at) \
         VALUES ($1, $2, NOW() + INTERVAL '1 hour', NOW() - INTERVAL '2 days')",
    )
    .bind(user_id)
    .bind(format!("reset-used-{}", Uuid::new_v4()))
    .execute(&ctx.pool)
    .await
    .expect("insert spent reset token");

    sqlx::query(
        "INSERT INTO notification_log (user_id, provider, external_id, episode_number, created_at) \
         VALUES ($1, 'shikimori', 'old', 1, NOW() - INTERVAL '120 days')",
    )
    .bind(user_id)
    .execute(&ctx.pool)
    .await
    .expect("insert old notification row");

    sqlx::query(
        "INSERT INTO notification_log (user_id, provider, external_id, episode_number) \
         VALUES ($1, 'shikimori', 'fresh', 1)",
    )
    .bind(user_id)
    .execute(&ctx.pool)
    .await
    .expect("insert fresh notification row");

    let stats = run_cleanup_once(&ctx.pool).await;
    assert_eq!(stats.sessions, 1, "exactly the expired session is removed");
    assert_eq!(
        stats.reset_tokens, 2,
        "expired and spent reset tokens are removed"
    );
    assert_eq!(
        stats.notifications, 1,
        "only notification rows past retention are removed"
    );

    // A live session still authenticates and has its last_seen_at touched.
    let session = ctx
        .state
        .auth
        .get_session(&live_raw)
        .await
        .expect("live session still resolves");
    assert!(session.expires_at > Utc::now());

    let touched: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT last_seen_at FROM sessions WHERE token_hash = $1")
            .bind(&live_hash)
            .fetch_one(&ctx.pool)
            .await
            .expect("read session row");
    assert!(touched.is_some(), "last_seen_at is written on first read");
}

#[tokio::test]
async fn cleanup_reports_zero_when_nothing_to_prune() {
    let ctx = common::TestContext::new().await;
    let stats = run_cleanup_once(&ctx.pool).await;
    assert_eq!(stats.sessions, 0);
    assert_eq!(stats.reset_tokens, 0);
    assert_eq!(stats.notifications, 0);
}
