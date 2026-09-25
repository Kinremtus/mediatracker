//! Progress isolation tests for P0-D: per-user progress tables are the single
//! source of truth for watched/read state.
//!
//! Covers:
//!   (a) one user marking an episode watched does NOT affect another user;
//!   (b) provider-scoped matching: an episode tracked under `shikimori` is not
//!       reported watched through the `mal` progress map unless a `mal`
//!       progress row exists.

mod common;

use mediatracker::services::episodes::{
    count_watched, get_episode_states, get_episodes, set_watched,
};
use uuid::Uuid;

const MAL_ID: i64 = 999_999_881;
const SHIKI_ID: &str = "999999881";
const USER_A: &str = "test_progress_isolation_a";
const USER_B: &str = "test_progress_isolation_b";
const USER_C: &str = "test_progress_isolation_c";
const EMAIL_A: &str = "progress_iso_a@example.com";
const EMAIL_B: &str = "progress_iso_b@example.com";
const EMAIL_C: &str = "progress_iso_c@example.com";

fn mal_str() -> String {
    MAL_ID.to_string()
}

async fn create_user(ctx: &common::TestContext, username: &str, email: &str) -> Uuid {
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(username)
        .execute(&ctx.pool)
        .await
        .expect("delete fixture user");

    sqlx::query(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash', 'user')",
    )
    .bind(username)
    .bind(email)
    .execute(&ctx.pool)
    .await
    .expect("create fixture user");

    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(username)
        .fetch_one(&ctx.pool)
        .await
        .expect("get user id")
}

async fn insert_episode(ctx: &common::TestContext, n: i32) {
    sqlx::query(
        r#"
        INSERT INTO anime_episodes
            (provider, external_id, episode_number, title_en, air_date)
        VALUES
            ('mal', $1, $2, $3, '2025-01-01')
        ON CONFLICT (provider, external_id, episode_number) DO NOTHING
        "#,
    )
    .bind(mal_str())
    .bind(n)
    .bind(format!("Episode {n}"))
    .execute(&ctx.pool)
    .await
    .expect("insert episode");
}

#[tokio::test]
async fn watched_state_is_isolated_between_users() {
    let ctx = common::TestContext::new().await;
    let user_a = create_user(&ctx, USER_A, EMAIL_A).await;
    let user_b = create_user(&ctx, USER_B, EMAIL_B).await;

    for n in 1..=3 {
        insert_episode(&ctx, n).await;
    }

    // A watches up to ep 2 under its own (`mal`) progress key.
    let updated = set_watched(&ctx.pool, user_a, "mal", &mal_str(), MAL_ID, 2, true)
        .await
        .expect("A marks ep 2");
    assert!(updated);

    let a_states = get_episode_states(&ctx.pool, user_a, "mal", &mal_str(), MAL_ID)
        .await
        .expect("A states");
    assert_eq!(a_states, vec![(1, true), (2, true), (3, false)]);

    // B must see nothing watched.
    let b_states = get_episode_states(&ctx.pool, user_b, "mal", &mal_str(), MAL_ID)
        .await
        .expect("B states");
    assert_eq!(
        b_states,
        vec![(1, false), (2, false), (3, false)],
        "user B must not inherit user A's watched state"
    );

    let b_episodes = get_episodes(&ctx.pool, "mal", &mal_str(), user_b, "mal", &mal_str())
        .await
        .expect("B episodes");
    assert!(
        b_episodes.iter().all(|e| !e.watched),
        "all of B's episodes must be unwatched"
    );

    let b_count = count_watched(&ctx.pool, user_b, "mal", &mal_str(), MAL_ID)
        .await
        .expect("B count");
    assert_eq!(b_count, 0, "B must have zero watched episodes");

    let a_count = count_watched(&ctx.pool, user_a, "mal", &mal_str(), MAL_ID)
        .await
        .expect("A count");
    assert_eq!(a_count, 2);
}

#[tokio::test]
async fn watched_state_is_provider_scoped() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx, USER_C, EMAIL_C).await;

    for n in 1..=3 {
        insert_episode(&ctx, n).await;
    }

    // Catalog episodes live under ('mal', MAL_ID); the user browses/tracks the
    // show through the shikimori URL key, so progress is written there.
    let updated = set_watched(&ctx.pool, user, "shikimori", SHIKI_ID, MAL_ID, 2, true)
        .await
        .expect("mark via shikimori");
    assert!(updated);

    // Provider-scoped read through shikimori sees the progress.
    let shiki_episodes = get_episodes(&ctx.pool, "mal", &mal_str(), user, "shikimori", SHIKI_ID)
        .await
        .expect("shikimori-scoped episodes");
    assert_eq!(
        shiki_episodes.iter().filter(|e| e.watched).count(),
        2,
        "shikimori progress map must show 2 watched episodes"
    );

    // The mal-scoped read must NOT fall back to the shikimori rows.
    let mal_episodes = get_episodes(&ctx.pool, "mal", &mal_str(), user, "mal", &mal_str())
        .await
        .expect("mal-scoped episodes");
    assert!(
        mal_episodes.iter().all(|e| !e.watched),
        "mal progress map must not leak shikimori watched state"
    );

    assert_eq!(
        count_watched(&ctx.pool, user, "shikimori", SHIKI_ID, MAL_ID)
            .await
            .expect("shikimori count"),
        2
    );
    assert_eq!(
        count_watched(&ctx.pool, user, "mal", &mal_str(), MAL_ID)
            .await
            .expect("mal count"),
        0,
        "no mal progress rows exist"
    );
}
