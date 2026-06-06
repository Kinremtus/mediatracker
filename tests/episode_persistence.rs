// Persistence roundtrip tests for EpisodeService functions
// (set_watched, count_watched, update_progress_from_watched).
//
// Verifies DB-level invariants that the HTMX toggle handler
// depends on:
//   * `set_watched` flips the bit, sets/clears `watched_at`, and
//     returns `false` for non-existent episodes.
//   * `count_watched` returns the MAX watched episode number (0 if
//     nothing is watched).
//   * `update_progress_from_watched` uses GREATEST semantics —
//     manual progress or a higher watched count never regresses.
//
// We do NOT call Jikan here; episodes are inserted directly via SQL.
// Live integration with the toggle handler is covered by Stage B
// browser testing.
//
// They are #[ignore] by default because they require a live
// Postgres. Run with:
//
//   TEST_DATABASE_URL=postgres://Kin@localhost/tracker_test \
//     cargo test --test episode_persistence -- --ignored

use mediatracker::services::episodes::{
    count_watched, set_watched, update_progress_from_watched,
};
use sqlx::PgPool;
use uuid::Uuid;

const MAL_ID: i64 = 999_999_991; // unique per test
const FIXTURE_USERNAME: &str = "test_episode_persistence_user";
const FIXTURE_EMAIL: &str = "test_episode_persistence@example.com";

fn require_db_url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL not set. Run with: \
             TEST_DATABASE_URL=postgres://Kin@localhost/tracker_test \
             cargo test --test episode_persistence -- --ignored"
        );
    })
}

async fn setup() -> PgPool {
    let url = require_db_url();
    let pool = PgPool::connect(&url).await.expect("connect to TEST_DATABASE_URL");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    // Wipe any prior fixture data (FK CASCADE clears dependent rows).
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .execute(&pool)
        .await
        .expect("delete fixture user");

    sqlx::query(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash_for_episode_persistence_test', 'user')",
    )
    .bind(FIXTURE_USERNAME)
    .bind(FIXTURE_EMAIL)
    .execute(&pool)
    .await
    .expect("create fixture user");

    // Clean any leftover episodes from a previous run with the same MAL_ID.
    sqlx::query("DELETE FROM anime_episodes WHERE external_id = $1")
        .bind(MAL_ID.to_string())
        .execute(&pool)
        .await
        .expect("delete stale episodes");

    pool
}

async fn insert_episode(pool: &PgPool, n: i32, watched: bool) {
    sqlx::query(
        r#"
        INSERT INTO anime_episodes
            (provider, external_id, episode_number, title_en, air_date, watched, watched_at)
        VALUES
            ('mal', $1, $2, $3, '2025-01-01', $4, CASE WHEN $4 THEN NOW() ELSE NULL END)
        ON CONFLICT (provider, external_id, episode_number) DO UPDATE
            SET watched = EXCLUDED.watched,
                watched_at = EXCLUDED.watched_at
        "#,
    )
    .bind(MAL_ID.to_string())
    .bind(n)
    .bind(format!("Episode {n}"))
    .bind(watched)
    .execute(pool)
    .await
    .expect("insert episode");
}

async fn fixture_tracking(
    pool: &PgPool,
    user_id: Uuid,
    initial_progress: i32,
) -> Uuid {
    let (media_id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO media_items
            (provider, external_id, media_type, title, mal_id, episodes)
        VALUES
            ('mal', $1, 'anime', 'Persistence Anime', $1, 24)
        RETURNING id
        "#,
    )
    .bind(MAL_ID.to_string())
    .fetch_one(pool)
    .await
    .expect("insert media_items");

    sqlx::query(
        r#"
        INSERT INTO tracking_entries
            (user_id, media_id, status, progress, score, is_rewatching)
        VALUES
            ($1, $2, 'in_progress', $3, NULL, FALSE)
        "#,
    )
    .bind(user_id)
    .bind(media_id)
    .bind(initial_progress)
    .execute(pool)
    .await
    .expect("insert tracking_entries");

    media_id
}

async fn read_progress(pool: &PgPool, user_id: Uuid, media_id: Uuid) -> i32 {
    let (p,): (i32,) =
        sqlx::query_as("SELECT progress FROM tracking_entries WHERE user_id = $1 AND media_id = $2")
            .bind(user_id)
            .bind(media_id)
            .fetch_one(pool)
            .await
            .expect("read progress");
    p
}

async fn read_watched_at(
    pool: &PgPool,
    n: i32,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let row: Option<(Option<chrono::DateTime<chrono::Utc>>,)> = sqlx::query_as(
        "SELECT watched_at FROM anime_episodes WHERE external_id = $1 AND episode_number = $2",
    )
    .bind(MAL_ID.to_string())
    .bind(n)
    .fetch_optional(pool)
    .await
    .expect("read watched_at");
    row.and_then(|(v,)| v)
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn set_watched_roundtrip() {
    let pool = setup().await;
    insert_episode(&pool, 1, false).await;

    // Toggle ON: should report the row was updated and stamp watched_at.
    let updated = set_watched(&pool, MAL_ID, 1, true).await.expect("set true");
    assert!(updated, "set_watched(true) should return true when row exists");
    assert!(read_watched_at(&pool, 1).await.is_some(), "watched_at must be set");

    // Toggle OFF: row still exists, watched_at is cleared.
    let updated = set_watched(&pool, MAL_ID, 1, false).await.expect("set false");
    assert!(updated, "set_watched(false) should still return true when row exists");
    assert!(read_watched_at(&pool, 1).await.is_none(), "watched_at must be cleared on unwatch");

    // Non-existent episode: should report "no row updated".
    let updated = set_watched(&pool, MAL_ID, 9999, true).await.expect("set on missing");
    assert!(!updated, "set_watched on missing episode must return false");
}

/// Bulk-fill semantics: marking ep N watched also marks 1..N watched,
/// matching MAL / AniList UX for long series. Unwatch stays single-row
/// so removing one earlier watched ep doesn't silently take down the
/// later ones the user explicitly marked.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn set_watched_fills_below_on_watch_but_not_on_unwatch() {
    let pool = setup().await;
    for n in 1..=5 {
        insert_episode(&pool, n, false).await;
    }

    // Mark ep 3 watched → 1, 2, 3 all watched.
    let updated = set_watched(&pool, MAL_ID, 3, true).await.expect("mark 3");
    assert!(updated);
    for n in 1..=3 {
        assert!(
            read_watched_at(&pool, n).await.is_some(),
            "ep {n} must be watched after bulk-fill from ep 3"
        );
    }
    for n in 4..=5 {
        assert!(
            read_watched_at(&pool, n).await.is_none(),
            "ep {n} must remain unwatched (above the bulk-fill point)"
        );
    }

    // Mark ep 5 watched → 1..5 all watched.
    let updated = set_watched(&pool, MAL_ID, 5, true).await.expect("mark 5");
    assert!(updated);
    for n in 1..=5 {
        assert!(
            read_watched_at(&pool, n).await.is_some(),
            "ep {n} must be watched after bulk-fill to ep 5"
        );
    }

    // Unwatch ep 2 → ONLY ep 2 cleared, 1, 3, 4, 5 still watched.
    let updated = set_watched(&pool, MAL_ID, 2, false).await.expect("unwatch 2");
    assert!(updated);
    assert!(read_watched_at(&pool, 2).await.is_none(), "ep 2 unwatched");
    for n in [1, 3, 4, 5] {
        assert!(
            read_watched_at(&pool, n).await.is_some(),
            "ep {n} must stay watched after unwatching ep 2"
        );
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn count_watched_returns_max_episode_number() {
    let pool = setup().await;
    insert_episode(&pool, 1, false).await;
    insert_episode(&pool, 2, true).await;
    insert_episode(&pool, 3, false).await;
    insert_episode(&pool, 4, true).await;
    insert_episode(&pool, 5, true).await;

    let n = count_watched(&pool, MAL_ID).await.expect("count");
    assert_eq!(n, 5, "must return the highest watched episode number, not the count");

    // Mark a higher episode watched → MAX goes up.
    insert_episode(&pool, 10, true).await;
    let n = count_watched(&pool, MAL_ID).await.expect("count after 10");
    assert_eq!(n, 10);

    // Unwatch the top → MAX drops to the next highest watched.
    set_watched(&pool, MAL_ID, 10, false).await.expect("unwatch 10");
    let n = count_watched(&pool, MAL_ID).await.expect("count after unwatch");
    assert_eq!(n, 5);
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn update_progress_from_watched_uses_greatest_semantics() {
    let pool = setup().await;

    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .fetch_one(&pool)
        .await
        .expect("get fixture user id");

    // Progress=5, then watched count 10 → progress should become 10.
    let media_id = fixture_tracking(&pool, user_id, 5).await;
    update_progress_from_watched(&pool, user_id, media_id, 10)
        .await
        .expect("update to 10");
    assert_eq!(read_progress(&pool, user_id, media_id).await, 10);

    // Now watched count drops to 3 (e.g. user un-checked episodes).
    // Progress must NOT regress — GREATEST(10, 3) = 10.
    update_progress_from_watched(&pool, user_id, media_id, 3)
        .await
        .expect("update to 3");
    assert_eq!(
        read_progress(&pool, user_id, media_id).await,
        10,
        "progress must never regress below the current value"
    );

    // Watched count above progress → bump up.
    update_progress_from_watched(&pool, user_id, media_id, 12)
        .await
        .expect("update to 12");
    assert_eq!(read_progress(&pool, user_id, media_id).await, 12);
}
