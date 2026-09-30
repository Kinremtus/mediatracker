//! Integration tests for the unified tracked-release calendar feed
//! (`ReleaseScheduleService::get_calendar_month`): status filter, family
//! filter, dedup and the year section.

mod common;

use chrono::NaiveDate;
use mediatracker::services::release_schedule::ReleaseScheduleService;
use uuid::Uuid;

const USER: &str = "test_calendar_feed";

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
    .bind("test_calendar_feed@example.com")
    .execute(&ctx.pool)
    .await
    .expect("create fixture user");
    sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(USER)
        .fetch_one(&ctx.pool)
        .await
        .expect("get user id")
}

#[allow(clippy::too_many_arguments)]
async fn seed_media(
    ctx: &common::TestContext,
    provider: &str,
    external_id: &str,
    media_type: &str,
    title: &str,
    aired_from: Option<NaiveDate>,
    year: Option<i16>,
    shikimori_id: Option<i64>,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO media_items \
             (provider, external_id, media_type, title, aired_from, year, shikimori_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (provider, external_id) DO UPDATE SET \
             media_type = EXCLUDED.media_type, title = EXCLUDED.title, \
             aired_from = EXCLUDED.aired_from, year = EXCLUDED.year, \
             shikimori_id = EXCLUDED.shikimori_id \
         RETURNING id",
    )
    .bind(provider)
    .bind(external_id)
    .bind(media_type)
    .bind(title)
    .bind(aired_from)
    .bind(year)
    .bind(shikimori_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("upsert media item")
}

async fn track(ctx: &common::TestContext, user: Uuid, media_id: Uuid, status: &str) {
    sqlx::query(
        "INSERT INTO tracking_entries (user_id, media_id, status) VALUES ($1, $2, $3) \
         ON CONFLICT (user_id, media_id) DO UPDATE SET status = EXCLUDED.status",
    )
    .bind(user)
    .bind(media_id)
    .bind(status)
    .execute(&ctx.pool)
    .await
    .expect("upsert tracking entry");
}

async fn seed_release(
    ctx: &common::TestContext,
    provider: &str,
    external_id: &str,
    season: i32,
    episode: i32,
    date: NaiveDate,
) {
    let air = date.and_hms_opt(12, 0, 0).unwrap().and_utc();
    sqlx::query(
        "INSERT INTO release_schedule \
             (provider, external_id, season_number, episode_number, air_date, title) \
         VALUES ($1, $2, $3, $4, $5, 'Release') \
         ON CONFLICT (provider, external_id, season_number, episode_number) \
         DO UPDATE SET air_date = EXCLUDED.air_date",
    )
    .bind(provider)
    .bind(external_id)
    .bind(season)
    .bind(episode)
    .bind(air)
    .execute(&ctx.pool)
    .await
    .expect("upsert release row");
}

#[tokio::test]
async fn dropped_is_excluded_and_paused_is_included() {
    let ctx = common::TestContext::new().await;
    let user = seed_user(&ctx).await;

    let dropped = seed_media(
        &ctx,
        "shikimori",
        "cal-drop",
        "anime",
        "Dropped",
        None,
        None,
        None,
    )
    .await;
    track(&ctx, user, dropped, "dropped").await;
    seed_release(
        &ctx,
        "shikimori",
        "cal-drop",
        0,
        2,
        NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
    )
    .await;

    let paused = seed_media(
        &ctx,
        "shikimori",
        "cal-pause",
        "anime",
        "Paused",
        None,
        None,
        None,
    )
    .await;
    track(&ctx, user, paused, "paused").await;
    seed_release(
        &ctx,
        "shikimori",
        "cal-pause",
        0,
        3,
        NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
    )
    .await;

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let view = service
        .get_calendar_month(user, 2026, 10, "")
        .await
        .expect("calendar month");

    let titles: Vec<&str> = view
        .weeks
        .iter()
        .flat_map(|w| w.events.iter())
        .map(|e| e.title.as_str())
        .collect();
    assert_eq!(titles, vec!["Paused"], "dropped excluded, paused included");
}

#[tokio::test]
async fn family_filter_limits_events_but_counts_all() {
    let ctx = common::TestContext::new().await;
    let user = seed_user(&ctx).await;

    let game = seed_media(
        &ctx,
        "rawg",
        "cal-game",
        "game",
        "Game",
        Some(NaiveDate::from_ymd_opt(2026, 10, 10).unwrap()),
        None,
        None,
    )
    .await;
    track(&ctx, user, game, "in_progress").await;

    let movie = seed_media(
        &ctx,
        "tmdb",
        "cal-movie",
        "movie",
        "Movie",
        Some(NaiveDate::from_ymd_opt(2026, 10, 11).unwrap()),
        None,
        None,
    )
    .await;
    track(&ctx, user, movie, "in_progress").await;

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let view = service
        .get_calendar_month(user, 2026, 10, "games")
        .await
        .expect("calendar month");

    let titles: Vec<&str> = view
        .weeks
        .iter()
        .flat_map(|w| w.events.iter())
        .map(|e| e.title.as_str())
        .collect();
    assert_eq!(titles, vec!["Game"]);

    let games = view.family_tabs.iter().find(|t| t.key == "games").unwrap();
    let movies = view.family_tabs.iter().find(|t| t.key == "movies").unwrap();
    assert_eq!(games.count, 1);
    assert_eq!(
        movies.count, 1,
        "counts cover all families, not just selected"
    );
    assert!(games.active);
    assert!(!movies.active);
}

#[tokio::test]
async fn year_only_planned_media_lands_in_year_section() {
    let ctx = common::TestContext::new().await;
    let user = seed_user(&ctx).await;

    // planned, year set, no exact date -> year section
    let manga = seed_media(
        &ctx,
        "mangaupdates",
        "cal-manga",
        "manga",
        "Manga",
        None,
        Some(2026),
        None,
    )
    .await;
    track(&ctx, user, manga, "planned").await;

    // planned, no exact date, but has a release_schedule row -> excluded
    let manga2 = seed_media(
        &ctx,
        "mangaupdates",
        "cal-manga2",
        "manga",
        "Manga2",
        None,
        Some(2026),
        None,
    )
    .await;
    track(&ctx, user, manga2, "planned").await;
    seed_release(
        &ctx,
        "mangaupdates",
        "cal-manga2",
        0,
        1,
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap(),
    )
    .await;

    // in_progress with year -> excluded (year section is planned-only)
    let game = seed_media(
        &ctx,
        "rawg",
        "cal-game2",
        "game",
        "Game",
        None,
        Some(2026),
        None,
    )
    .await;
    track(&ctx, user, game, "in_progress").await;

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let view = service
        .get_calendar_month(user, 2026, 10, "")
        .await
        .expect("calendar month");

    let titles: Vec<&str> = view.year_section.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(titles, vec!["Manga"]);
    assert_eq!(view.year_section_title, "Выйдет когда-то в 2026");
}

#[tokio::test]
async fn episode_wins_over_same_day_premiere_for_mal_tracked_anime() {
    let ctx = common::TestContext::new().await;
    let user = seed_user(&ctx).await;

    let day = NaiveDate::from_ymd_opt(2026, 10, 5).unwrap();
    let anime = seed_media(
        &ctx,
        "mal",
        "cal-anime",
        "anime",
        "Anime",
        Some(day),
        None,
        Some(21),
    )
    .await;
    track(&ctx, user, anime, "in_progress").await;
    seed_release(&ctx, "shikimori", "21", 1, 1, day).await;

    let service = ReleaseScheduleService::new(ctx.pool.clone());
    let view = service
        .get_calendar_month(user, 2026, 10, "")
        .await
        .expect("calendar month");

    let events: Vec<_> = view.weeks.iter().flat_map(|w| w.events.iter()).collect();
    assert_eq!(events.len(), 1, "premiere deduped against episode");
    assert!(events[0].is_episode());
    assert_eq!(
        events[0].provider, "mal",
        "drawer targets tracked media_items"
    );
    assert_eq!(events[0].canonical_key, "shikimori:21");
}
