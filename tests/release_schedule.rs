//! Regression tests for the release-schedule remediation:
//!   P1-3: `SELECT DISTINCT ON (r.id) ... ORDER BY r.air_date` used to raise
//!         Postgres error 42P10, so the calendar/releases were always empty.
//!   P1-4: `notify_new_episodes` must send each episode at most once even when
//!         it runs repeatedly (atomic claim via the notification_log unique
//!         index).

mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};

use chrono::{Duration, Utc};
use mediatracker::services::notifications::TelegramNotifier;
use mediatracker::services::release_schedule::ReleaseScheduleService;
use uuid::Uuid;

const PROVIDER: &str = "shikimori";
const EXTERNAL_ID: &str = "regression-release-001";
const USER: &str = "test_release_schedule";
const EMAIL: &str = "test_release_schedule@example.com";

async fn create_user(ctx: &common::TestContext, telegram_chat_id: Option<&str>) -> Uuid {
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(USER)
        .execute(&ctx.pool)
        .await
        .expect("delete fixture user");

    sqlx::query(
        "INSERT INTO users (username, email, password_hash, role, \
         telegram_chat_id, telegram_notifications_enabled) \
         VALUES ($1, $2, 'fakehash', 'user', $3, $4)",
    )
    .bind(USER)
    .bind(EMAIL)
    .bind(telegram_chat_id)
    .bind(telegram_chat_id.is_some())
    .execute(&ctx.pool)
    .await
    .expect("create fixture user");

    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(USER)
        .fetch_one(&ctx.pool)
        .await
        .expect("get user id")
}

async fn track_media(ctx: &common::TestContext, user_id: Uuid, status: &str) {
    let media_id: Uuid = sqlx::query_scalar(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ($1, $2, 'anime', 'Regression Anime') \
         ON CONFLICT (provider, external_id) DO UPDATE SET title = EXCLUDED.title \
         RETURNING id",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("upsert media item");

    sqlx::query(
        "INSERT INTO tracking_entries (user_id, media_id, status) VALUES ($1, $2, $3) \
         ON CONFLICT (user_id, media_id) DO UPDATE SET status = EXCLUDED.status",
    )
    .bind(user_id)
    .bind(media_id)
    .bind(status)
    .execute(&ctx.pool)
    .await
    .expect("upsert tracking entry");
}

async fn insert_release(
    ctx: &common::TestContext,
    air_date: chrono::DateTime<Utc>,
    episode: i32,
    season: i32,
) {
    sqlx::query(
        "INSERT INTO release_schedule \
             (provider, external_id, season_number, episode_number, air_date, title) \
         VALUES ($1, $2, $3, $4, $5, 'Regression Anime') \
         ON CONFLICT (provider, external_id, season_number, episode_number) \
         DO UPDATE SET air_date = EXCLUDED.air_date",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .bind(season)
    .bind(episode)
    .bind(air_date)
    .execute(&ctx.pool)
    .await
    .expect("upsert release schedule row");
}

/// Minimal one-shot-per-request HTTP stub that always answers `200 {"ok":true}`.
/// Spawned in a background thread; the OS reclaims it when the test process exits.
fn spawn_stub_telegram() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub telegram");
    let addr = listener.local_addr().expect("stub addr");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => break,
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = r#"{"ok":true,"result":{"message_id":1}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    addr
}

#[tokio::test]
async fn upcoming_releases_query_returns_row_without_distinct_on_error() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx, None).await;
    track_media(&ctx, user, "in_progress").await;
    insert_release(&ctx, Utc::now() + Duration::days(1), 12, 0).await;

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let rows = service
        .get_upcoming_for_user(user, 7)
        .await
        .expect("upcoming query must not fail (P1-3 regression)");

    assert_eq!(rows.len(), 1, "expected exactly one upcoming release");
    assert_eq!(rows[0].episode_number, 12);
    assert_eq!(rows[0].provider, PROVIDER);
}

#[tokio::test]
async fn releases_by_date_range_returns_row() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx, None).await;
    track_media(&ctx, user, "planned").await;

    let from = Utc::now();
    let to = from + Duration::days(7);
    insert_release(&ctx, from + Duration::days(2), 3, 0).await;

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let rows = service
        .get_by_date_range(user, from, to)
        .await
        .expect("date-range query must not fail (P1-3 regression)");

    assert_eq!(rows.len(), 1, "expected exactly one release in range");
    assert_eq!(rows[0].episode_number, 3);
}

#[tokio::test]
async fn notify_new_episodes_claims_notification_atomically() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx, Some("123456")).await;
    track_media(&ctx, user, "in_progress").await;
    // Aired one hour ago: inside the "last 2 hours" notification window.
    insert_release(&ctx, Utc::now() - Duration::hours(1), 12, 0).await;

    let addr = spawn_stub_telegram();
    let notifier =
        TelegramNotifier::with_base_url("test-token".to_string(), format!("http://{addr}"));

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let first = service
        .notify_new_episodes(&notifier)
        .await
        .expect("first notification pass");
    let second = service
        .notify_new_episodes(&notifier)
        .await
        .expect("second notification pass");

    assert_eq!(first, 1, "first pass should send one notification");
    assert_eq!(second, 0, "second pass must not re-send the same episode");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notification_log \
         WHERE user_id = $1 AND provider = $2 AND external_id = $3 AND episode_number = 12",
    )
    .bind(user)
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("count notification_log rows");

    assert_eq!(count, 1, "exactly one notification_log row expected");
}

#[allow(clippy::too_many_arguments)]
async fn track_media_item(
    ctx: &common::TestContext,
    user_id: Uuid,
    provider: &str,
    external_id: &str,
    media_type: &str,
    mal_id: Option<i64>,
    shikimori_id: Option<i64>,
    status: &str,
) -> Uuid {
    let media_id: Uuid = sqlx::query_scalar(
        "INSERT INTO media_items (provider, external_id, media_type, title, mal_id, shikimori_id) \
         VALUES ($1, $2, $3, 'Title', $4, $5) \
         ON CONFLICT (provider, external_id) DO UPDATE SET title = EXCLUDED.title, \
             mal_id = EXCLUDED.mal_id, shikimori_id = EXCLUDED.shikimori_id \
         RETURNING id",
    )
    .bind(provider)
    .bind(external_id)
    .bind(media_type)
    .bind(mal_id)
    .bind(shikimori_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("upsert media item");

    sqlx::query(
        "INSERT INTO tracking_entries (user_id, media_id, status) VALUES ($1, $2, $3) \
         ON CONFLICT (user_id, media_id) DO UPDATE SET status = EXCLUDED.status",
    )
    .bind(user_id)
    .bind(media_id)
    .bind(status)
    .execute(&ctx.pool)
    .await
    .expect("upsert tracking entry");

    media_id
}

/// MAL-tracked anime with a resolved `shikimori_id` must match Shikimori
/// releases through the OR-arm.
#[tokio::test]
async fn mal_anime_matches_shikimori_release_via_shikimori_id() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx, None).await;
    track_media_item(
        &ctx,
        user,
        "mal",
        "mal-match-001",
        "anime",
        Some(21),
        Some(21),
        "in_progress",
    )
    .await;

    sqlx::query(
        "INSERT INTO release_schedule \
             (provider, external_id, season_number, episode_number, air_date, title) \
         VALUES ('shikimori', '21', 0, 5, $1, 'Matched Anime')",
    )
    .bind(Utc::now() + Duration::days(1))
    .execute(&ctx.pool)
    .await
    .expect("insert shikimori release");

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let rows = service
        .get_upcoming_for_user(user, 7)
        .await
        .expect("query must not fail");

    assert_eq!(rows.len(), 1, "MAL anime must match its Shikimori release");
    assert_eq!(rows[0].episode_number, 5);
    assert_eq!(rows[0].season_number, 0);
}

/// TMDB series rows must expose their season number.
#[tokio::test]
async fn tmdb_season_row_returns_season_number() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx, None).await;
    track_media_item(
        &ctx,
        user,
        "tmdb",
        "tv-season-001",
        "series",
        None,
        None,
        "planned",
    )
    .await;

    sqlx::query(
        "INSERT INTO release_schedule \
             (provider, external_id, season_number, episode_number, air_date, title) \
         VALUES ('tmdb', 'tv-season-001', 2, 5, $1, 'Seasoned Show')",
    )
    .bind(Utc::now() + Duration::days(3))
    .execute(&ctx.pool)
    .await
    .expect("insert tmdb release");

    let from = Utc::now();
    let to = from + Duration::days(7);
    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let rows = service
        .get_by_date_range(user, from, to)
        .await
        .expect("date-range query must not fail");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].season_number, 2);
    assert_eq!(rows[0].episode_number, 5);
    assert_eq!(rows[0].episode_label(), "S2 E5");
}

/// The calendar widened its filter to `status <> 'dropped'`; the home widget
/// must keep the original `in_progress`/`planned` filter.
#[tokio::test]
async fn upcoming_widget_excludes_paused_and_completed() {
    let ctx = common::TestContext::new().await;
    let user = create_user(&ctx, None).await;

    // paused + completed should not surface in the widget.
    for (status, id) in [("paused", "widget-paused"), ("completed", "widget-done")] {
        track_media_item(&ctx, user, "tmdb", id, "series", None, None, status).await;
        sqlx::query(
            "INSERT INTO release_schedule \
                 (provider, external_id, season_number, episode_number, air_date, title) \
             VALUES ('tmdb', $1, 1, 1, $2, 'Widget')",
        )
        .bind(id)
        .bind(Utc::now() + Duration::days(1))
        .execute(&ctx.pool)
        .await
        .expect("insert release");
    }

    // in_progress should surface.
    track_media_item(
        &ctx,
        user,
        "tmdb",
        "widget-live",
        "series",
        None,
        None,
        "in_progress",
    )
    .await;
    sqlx::query(
        "INSERT INTO release_schedule \
             (provider, external_id, season_number, episode_number, air_date, title) \
         VALUES ('tmdb', 'widget-live', 1, 1, $1, 'Widget')",
    )
    .bind(Utc::now() + Duration::days(1))
    .execute(&ctx.pool)
    .await
    .expect("insert release");

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let rows = service
        .get_upcoming_for_user(user, 10)
        .await
        .expect("upcoming");

    assert_eq!(rows.len(), 1, "only in_progress should appear");
    assert_eq!(rows[0].external_id, "widget-live");
}
