mod common;

use mediatracker::services::episodes::{
    count_watched, get_episode, get_episode_states, get_episodes, set_watched,
    update_progress_from_watched,
};
use uuid::Uuid;

const MAL_ID: i64 = 999_999_991;
const FIXTURE_USERNAME: &str = "test_episode_persistence_user";
const FIXTURE_EMAIL: &str = "<SECRET: MailDetector>";

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
    .bind(FIXTURE_EMAIL)
    .execute(&ctx.pool)
    .await
    .expect("create fixture user");

    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .fetch_one(&ctx.pool)
        .await
        .expect("get user id");

    sqlx::query("DELETE FROM anime_episodes WHERE external_id = $1")
        .bind(MAL_ID.to_string())
        .execute(&ctx.pool)
        .await
        .expect("delete stale episodes");

    (ctx, user_id)
}

fn mal_str() -> String {
    MAL_ID.to_string()
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

async fn fixture_tracking(ctx: &common::TestContext, user_id: Uuid, initial_progress: i32) -> Uuid {
    let (media_id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO media_items
            (provider, external_id, media_type, title, mal_id, episodes)
        VALUES
            ('mal', $1, 'anime', 'Persistence Anime', $2, 24)
        RETURNING id
        "#,
    )
    .bind(mal_str())
    .bind(MAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("insert media_items");

    sqlx::query(
        r#"
        INSERT INTO tracking_entries
            (user_id, media_id, status, progress)
        VALUES
            ($1, $2, 'in_progress', $3)
        "#,
    )
    .bind(user_id)
    .bind(media_id)
    .bind(initial_progress)
    .execute(&ctx.pool)
    .await
    .expect("insert tracking_entries");

    media_id
}

async fn read_progress(ctx: &common::TestContext, user_id: Uuid, media_id: Uuid) -> i32 {
    let (p,): (i32,) = sqlx::query_as(
        "SELECT progress FROM tracking_entries WHERE user_id = $1 AND media_id = $2",
    )
    .bind(user_id)
    .bind(media_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("read progress");
    p
}

/// Reads the per-user progress row. `None` means no row exists (i.e. unwatched).
async fn read_progress_watched(ctx: &common::TestContext, user_id: Uuid, n: i32) -> Option<bool> {
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT watched FROM user_episode_progress \
         WHERE user_id = $1 AND provider = 'mal' AND external_id = $2 AND episode_number = $3",
    )
    .bind(user_id)
    .bind(mal_str())
    .bind(n)
    .fetch_optional(&ctx.pool)
    .await
    .expect("read progress watched");
    row.map(|(v,)| v)
}

async fn read_progress_watched_at(
    ctx: &common::TestContext,
    user_id: Uuid,
    n: i32,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let row: Option<(Option<chrono::DateTime<chrono::Utc>>,)> = sqlx::query_as(
        "SELECT watched_at FROM user_episode_progress \
         WHERE user_id = $1 AND provider = 'mal' AND external_id = $2 AND episode_number = $3",
    )
    .bind(user_id)
    .bind(mal_str())
    .bind(n)
    .fetch_optional(&ctx.pool)
    .await
    .expect("read watched_at");
    row.and_then(|(v,)| v)
}

#[tokio::test]
async fn set_watched_roundtrip() {
    let (ctx, user_id) = setup().await;
    insert_episode(&ctx, 1).await;

    let updated = set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 1, true)
        .await
        .expect("set true");
    assert!(
        updated,
        "set_watched(true) should return true when row exists"
    );
    assert!(
        read_progress_watched_at(&ctx, user_id, 1).await.is_some(),
        "watched_at must be set"
    );
    assert_eq!(read_progress_watched(&ctx, user_id, 1).await, Some(true));

    let updated = set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 1, false)
        .await
        .expect("set false");
    assert!(
        updated,
        "set_watched(false) should still return true when row exists"
    );
    assert!(
        read_progress_watched_at(&ctx, user_id, 1).await.is_none(),
        "watched_at must be cleared on unwatch"
    );

    // Fresh context with no episodes: non-existent episode must report 0 rows affected.
    let (empty_ctx, empty_user) = setup().await;
    let updated = set_watched(
        &empty_ctx.pool,
        empty_user,
        "mal",
        &mal_str(),
        MAL_ID,
        9999,
        true,
    )
    .await
    .expect("set on missing");
    assert!(!updated, "set_watched on missing episode must return false");
}

#[tokio::test]
async fn set_watched_bulk_fills_below_on_watch_and_cascades_above_on_unwatch() {
    let (ctx, user_id) = setup().await;
    for n in 1..=5 {
        insert_episode(&ctx, n).await;
    }

    let updated = set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 3, true)
        .await
        .expect("mark 3");
    assert!(updated);
    for n in 1..=3 {
        assert_eq!(
            read_progress_watched(&ctx, user_id, n).await,
            Some(true),
            "ep {n} must be watched after bulk-fill from ep 3"
        );
    }
    for n in 4..=5 {
        assert_eq!(
            read_progress_watched(&ctx, user_id, n).await,
            None,
            "ep {n} must remain unwatched (above the bulk-fill point)"
        );
    }

    let updated = set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 5, true)
        .await
        .expect("mark 5");
    assert!(updated);
    for n in 1..=5 {
        assert_eq!(
            read_progress_watched(&ctx, user_id, n).await,
            Some(true),
            "ep {n} must be watched after bulk-fill to ep 5"
        );
    }

    let updated = set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 3, false)
        .await
        .expect("unwatch 3");
    assert!(updated);
    for n in 1..=2 {
        assert_eq!(
            read_progress_watched(&ctx, user_id, n).await,
            Some(true),
            "ep {n} must stay watched (below the un-check point)"
        );
    }
    for n in 3..=5 {
        assert_eq!(
            read_progress_watched(&ctx, user_id, n).await,
            Some(false),
            "ep {n} must cascade-unwatch to the un-check point"
        );
    }

    let updated = set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 1, false)
        .await
        .expect("unwatch 1");
    assert!(updated);
    for n in 1..=2 {
        assert_eq!(
            read_progress_watched(&ctx, user_id, n).await,
            Some(false),
            "ep {n} unwatched"
        );
    }
}

#[tokio::test]
async fn count_watched_returns_max_episode_number() {
    let (ctx, user_id) = setup().await;
    for n in 1..=5 {
        insert_episode(&ctx, n).await;
    }
    insert_episode(&ctx, 10).await;

    set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 5, true)
        .await
        .expect("mark 5");
    let n = count_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID)
        .await
        .expect("count");
    assert_eq!(
        n, 5,
        "must return the highest watched episode number, not the count"
    );

    set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 10, true)
        .await
        .expect("mark 10");
    let n = count_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID)
        .await
        .expect("count after 10");
    assert_eq!(n, 10);

    set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 10, false)
        .await
        .expect("unwatch 10");
    let n = count_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID)
        .await
        .expect("count after unwatch");
    assert_eq!(n, 5);
}

#[tokio::test]
async fn get_episodes_and_states_are_user_scoped() {
    let (ctx, user_id) = setup().await;
    for n in 1..=3 {
        insert_episode(&ctx, n).await;
    }
    set_watched(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID, 2, true)
        .await
        .expect("mark 2");

    let episodes = get_episodes(&ctx.pool, "mal", &mal_str(), user_id, "mal", &mal_str())
        .await
        .expect("get episodes");
    assert_eq!(episodes.len(), 3);
    assert!(episodes[0].watched, "ep 1 watched via bulk-fill");
    assert!(episodes[1].watched, "ep 2 watched");
    assert!(!episodes[2].watched, "ep 3 unwatched");

    let states = get_episode_states(&ctx.pool, user_id, "mal", &mal_str(), MAL_ID)
        .await
        .expect("get states");
    assert_eq!(states, vec![(1, true), (2, true), (3, false)]);

    let ep = get_episode(&ctx.pool, MAL_ID, 1, user_id, "mal", &mal_str())
        .await
        .expect("get episode")
        .expect("ep 1 exists");
    assert!(ep.watched);
}

#[tokio::test]
async fn update_progress_from_watched_uses_greatest_semantics() {
    let (ctx, user_id) = setup().await;

    let media_id = fixture_tracking(&ctx, user_id, 5).await;
    update_progress_from_watched(&ctx.pool, user_id, media_id, 10)
        .await
        .expect("update to 10");
    assert_eq!(read_progress(&ctx, user_id, media_id).await, 10);

    update_progress_from_watched(&ctx.pool, user_id, media_id, 3)
        .await
        .expect("update to 3");
    assert_eq!(
        read_progress(&ctx, user_id, media_id).await,
        10,
        "progress must never regress below the current value"
    );

    update_progress_from_watched(&ctx.pool, user_id, media_id, 12)
        .await
        .expect("update to 12");
    assert_eq!(read_progress(&ctx, user_id, media_id).await, 12);
}
