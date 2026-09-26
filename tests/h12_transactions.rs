//! Regression tests for P1-10 (H12): batched writes and transactions.
//!
//! Covers:
//!   (a) `add_to_list` under two concurrent calls leaves exactly one
//!       media_items row and one tracking_entries row (race-safe upsert);
//!   (b) `store_chapters_mu` inserts the whole skeleton in one statement,
//!       syncs `media_items.chapters`, and is idempotent on re-run;
//!   (c) `update_entry` mirrors the tracking progress onto the *current
//!       user's* TMDB episode rows only (direct set semantics);
//!   (d) chapter read state is isolated between users.

mod common;

use mediatracker::models::media_item::CreateMediaItem;
use mediatracker::models::tracking_entry::UpdateTracking;
use mediatracker::services::chapters::{
    count_read, get_chapter_states, set_read, store_chapters_mu,
};
use mediatracker::services::tracking::TrackingService;
use uuid::Uuid;

fn media(provider: &str, external_id: &str, media_type: &str, title: &str) -> CreateMediaItem {
    CreateMediaItem {
        provider: provider.to_string(),
        external_id: external_id.to_string(),
        media_type: media_type.to_string(),
        title: title.to_string(),
        ..Default::default()
    }
}

async fn create_user(ctx: &common::TestContext, username: &str, email: &str) -> Uuid {
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(username)
        .execute(&ctx.pool)
        .await
        .expect("delete fixture user");

    sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash', 'user') RETURNING id",
    )
    .bind(username)
    .bind(email)
    .fetch_one(&ctx.pool)
    .await
    .expect("create fixture user")
}

async fn tmdb_watched(
    ctx: &common::TestContext,
    user: Uuid,
    external_id: &str,
) -> Vec<(i32, i32, bool)> {
    sqlx::query_as(
        "SELECT season_number, episode_number, watched \
         FROM user_tmdb_episode_progress \
         WHERE user_id = $1 AND external_id = $2 \
         ORDER BY season_number, episode_number",
    )
    .bind(user)
    .bind(external_id)
    .fetch_all(&ctx.pool)
    .await
    .expect("read tmdb progress")
}

#[tokio::test]
async fn add_to_list_is_race_safe() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx, "h12_race_user", "h12_race@example.com").await;
    let svc = TrackingService::new(ctx.pool.clone());
    let media = media("tmdb", "h12-race-1", "series", "Race Title");

    // Two concurrent adds for the same title: the old SELECT-then-INSERT
    // could have both missed and then one would fail on the UNIQUE
    // (provider, external_id). The upsert must let both succeed.
    let (first, second) = tokio::join!(
        svc.add_to_list(user, &media, "in_progress"),
        svc.add_to_list(user, &media, "in_progress"),
    );
    first.expect("first add_to_list");
    second.expect("second add_to_list");

    let media_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM media_items \
         WHERE provider = 'tmdb' AND external_id = 'h12-race-1'",
    )
    .fetch_one(&ctx.pool)
    .await
    .expect("count media");
    let entry_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*)::bigint FROM tracking_entries WHERE user_id = $1")
            .bind(user)
            .fetch_one(&ctx.pool)
            .await
            .expect("count entries");

    assert_eq!(media_count, 1, "exactly one media_items row");
    assert_eq!(entry_count, 1, "exactly one tracking_entries row");
}

#[tokio::test]
async fn store_chapters_mu_batches_and_syncs_media_items() {
    let ctx = common::TestContext::new().await;
    let series_id: i64 = 777_000_123;
    let external_id = series_id.to_string();

    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ('mangaupdates', $1, 'manga', 'H12 Manga')",
    )
    .bind(&external_id)
    .execute(&ctx.pool)
    .await
    .expect("insert media item");

    let inserted = store_chapters_mu(&ctx.pool, series_id, 5)
        .await
        .expect("store skeleton");
    assert_eq!(inserted, 5, "5 chapters inserted in one statement");

    let numbers: Vec<i32> = sqlx::query_scalar(
        "SELECT chapter_number FROM series_chapters \
         WHERE provider = 'mangaupdates' AND external_id = $1 \
         ORDER BY chapter_number",
    )
    .bind(&external_id)
    .fetch_all(&ctx.pool)
    .await
    .expect("read chapters");
    assert_eq!(numbers, vec![10, 20, 30, 40, 50]);

    let chapters: Option<i32> = sqlx::query_scalar(
        "SELECT chapters FROM media_items \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(&external_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("read media chapters");
    assert_eq!(
        chapters,
        Some(5),
        "media_items.chapters synced in the same tx"
    );

    // Idempotent: a second run inserts nothing.
    let again = store_chapters_mu(&ctx.pool, series_id, 5)
        .await
        .expect("re-run skeleton");
    assert_eq!(again, 0, "re-run must insert nothing");
}

#[tokio::test]
async fn update_entry_mirrors_progress_to_user_tmdb_episodes() {
    let ctx = common::TestContext::new().await;
    let user_a = create_user(&ctx, "h12_tmdb_a", "h12_tmdb_a@example.com").await;
    let user_b = create_user(&ctx, "h12_tmdb_b", "h12_tmdb_b@example.com").await;
    let svc = TrackingService::new(ctx.pool.clone());

    let ext = "h12-tmdb-1";
    let entry_a = svc
        .add_to_list(
            user_a,
            &media("tmdb", ext, "series", "H12 Show"),
            "in_progress",
        )
        .await
        .expect("add user A");
    svc.add_to_list(
        user_b,
        &media("tmdb", ext, "series", "H12 Show"),
        "in_progress",
    )
    .await
    .expect("add user B");

    for (season, episode) in [(1, 1), (1, 2), (1, 3), (2, 1), (2, 2)] {
        sqlx::query(
            "INSERT INTO tmdb_episodes (external_id, season_number, episode_number) \
             VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
        )
        .bind(ext)
        .bind(season)
        .bind(episode)
        .execute(&ctx.pool)
        .await
        .expect("insert tmdb episode");
    }

    svc.update_entry(
        entry_a.id,
        user_a,
        &UpdateTracking {
            status: None,
            rating: None,
            progress: Some(3),
        },
    )
    .await
    .expect("update entry");

    let watched = tmdb_watched(&ctx, user_a, ext).await;
    assert_eq!(
        watched,
        vec![
            (1, 1, true),
            (1, 2, true),
            (1, 3, true),
            (2, 1, false),
            (2, 2, false),
        ],
        "first `progress` episodes watched, ordered by (season, episode)"
    );

    let b_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM user_tmdb_episode_progress WHERE user_id = $1",
    )
    .bind(user_b)
    .fetch_one(&ctx.pool)
    .await
    .expect("count user B");
    assert_eq!(b_rows, 0, "sync must only touch the updating user");

    // Direct (not greatest) semantics: lowering progress un-watches.
    svc.update_entry(
        entry_a.id,
        user_a,
        &UpdateTracking {
            status: None,
            rating: None,
            progress: Some(1),
        },
    )
    .await
    .expect("lower progress");

    let watched = tmdb_watched(&ctx, user_a, ext).await;
    assert_eq!(
        watched,
        vec![
            (1, 1, true),
            (1, 2, false),
            (1, 3, false),
            (2, 1, false),
            (2, 2, false),
        ],
        "lowering progress clears later episodes"
    );
}

#[tokio::test]
async fn chapter_read_state_is_isolated_between_users() {
    let ctx = common::TestContext::new().await;
    let user_a = create_user(&ctx, "h12_ch_a", "h12_ch_a@example.com").await;
    let user_b = create_user(&ctx, "h12_ch_b", "h12_ch_b@example.com").await;

    let series_id: i64 = 777_000_456;
    let external_id = series_id.to_string();
    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ('mangaupdates', $1, 'manga', 'Iso Manga')",
    )
    .bind(&external_id)
    .execute(&ctx.pool)
    .await
    .expect("insert media item");
    store_chapters_mu(&ctx.pool, series_id, 3)
        .await
        .expect("skeleton");

    // A reads up to chapter 2 (stored as 20): bulk-fill 10 and 20.
    assert!(
        set_read(&ctx.pool, user_a, "mangaupdates", &external_id, 20, true)
            .await
            .expect("A marks chapter 2")
    );

    assert_eq!(
        count_read(&ctx.pool, user_a, "mangaupdates", &external_id)
            .await
            .expect("A count"),
        2,
        "A read chapters 1 and 2 (count_read returns MAX/10)"
    );
    assert_eq!(
        count_read(&ctx.pool, user_b, "mangaupdates", &external_id)
            .await
            .expect("B count"),
        0,
        "user B must not inherit A's read state"
    );

    assert_eq!(
        get_chapter_states(&ctx.pool, user_a, "mangaupdates", &external_id)
            .await
            .expect("A states"),
        vec![(10, true), (20, true), (30, false)]
    );
    assert_eq!(
        get_chapter_states(&ctx.pool, user_b, "mangaupdates", &external_id)
            .await
            .expect("B states"),
        vec![(10, false), (20, false), (30, false)]
    );
}
