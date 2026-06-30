mod common;

use mediatracker::services::chapters::{
    count_read, format_chapter, get_chapter, get_chapters, parse_chapter,
    set_read, store_chapters_mu, update_progress_from_read,
};
use uuid::Uuid;

const SERIES_ID: i64 = 999_999_992;
const FIXTURE_USERNAME: &str = "test_chapter_persistence_user";
const FIXTURE_EMAIL: &str = "test@example.com";

async fn setup() -> common::TestContext {
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

    sqlx::query("DELETE FROM series_chapters WHERE external_id = $1")
        .bind(SERIES_ID.to_string())
        .execute(&ctx.pool)
        .await
        .expect("delete stale chapters");

    ctx
}

async fn insert_chapter(ctx: &common::TestContext, ch_num_10: i32, read: bool) {
    sqlx::query(
        r#"
        INSERT INTO series_chapters
            (provider, external_id, chapter_number, read, read_at)
        VALUES
            ('mangaupdates', $1, $2, $3, CASE WHEN $3 THEN NOW() ELSE NULL END)
        ON CONFLICT (provider, external_id, chapter_number) DO UPDATE
            SET read = EXCLUDED.read,
                read_at = EXCLUDED.read_at
        "#,
    )
    .bind(SERIES_ID.to_string())
    .bind(ch_num_10)
    .bind(read)
    .execute(&ctx.pool)
    .await
    .expect("insert chapter");
}

async fn fixture_tracking(ctx: &common::TestContext, user_id: Uuid, initial_progress: i32) -> Uuid {
    let (media_id,): (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO media_items
            (provider, external_id, media_type, title)
        VALUES
            ('mangaupdates', $1, 'manga', 'Persistence Manga')
        RETURNING id
        "#,
    )
    .bind(SERIES_ID.to_string())
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

async fn read_read_at(ctx: &common::TestContext, ch_num_10: i32) -> Option<chrono::DateTime<chrono::Utc>> {
    let row: Option<(Option<chrono::DateTime<chrono::Utc>>,)> = sqlx::query_as(
        "SELECT read_at FROM series_chapters WHERE external_id = $1 AND chapter_number = $2",
    )
    .bind(SERIES_ID.to_string())
    .bind(ch_num_10)
    .fetch_optional(&ctx.pool)
    .await
    .expect("read read_at");
    row.and_then(|(v,)| v)
}

// ─── Unit tests (no DB) ───────────────────────

#[test]
fn format_chapter_integer() {
    assert_eq!(format_chapter(10), "1");
    assert_eq!(format_chapter(20), "2");
    assert_eq!(format_chapter(100), "10");
}

#[test]
fn format_chapter_fractional() {
    assert_eq!(format_chapter(105), "10.5");
    assert_eq!(format_chapter(250), "25");
    assert_eq!(format_chapter(101), "10.1");
}

#[test]
fn parse_chapter_integer() {
    assert_eq!(parse_chapter("1"), Some(10));
    assert_eq!(parse_chapter("10"), Some(100));
}

#[test]
fn parse_chapter_fractional() {
    assert_eq!(parse_chapter("10.5"), Some(105));
    assert_eq!(parse_chapter("1.1"), Some(11));
}

#[test]
fn parse_chapter_invalid() {
    assert_eq!(parse_chapter("abc"), None);
    assert_eq!(parse_chapter(""), None);
    assert_eq!(parse_chapter("1.2.3"), None);
}

// ─── DB integration tests ─────────────────────

#[tokio::test]
async fn set_read_roundtrip() {
    let ctx = setup().await;
    insert_chapter(&ctx, 10, false).await;

    let updated = set_read(&ctx.pool, "mangaupdates", &SERIES_ID.to_string(), 10, true)
        .await
        .expect("set true");
    assert!(updated);
    assert!(read_read_at(&ctx, 10).await.is_some(), "read_at must be set");

    let updated = set_read(&ctx.pool, "mangaupdates", &SERIES_ID.to_string(), 10, false)
        .await
        .expect("set false");
    assert!(updated);
    assert!(read_read_at(&ctx, 10).await.is_none(), "read_at must be cleared");
}

#[tokio::test]
async fn set_read_bulk_fills_below_and_cascades_above() {
    let ctx = setup().await;
    for n in 1..=5 {
        insert_chapter(&ctx, n * 10, false).await;
    }

    set_read(&ctx.pool, "mangaupdates", &SERIES_ID.to_string(), 30, true)
        .await
        .expect("mark 3");
    for n in 1..=3 {
        assert!(
            read_read_at(&ctx, n * 10).await.is_some(),
            "ch {n} must be read after bulk-fill from ch 3"
        );
    }
    for n in 4..=5 {
        assert!(
            read_read_at(&ctx, n * 10).await.is_none(),
            "ch {n} must remain unread (above the bulk-fill point)"
        );
    }

    set_read(&ctx.pool, "mangaupdates", &SERIES_ID.to_string(), 30, false)
        .await
        .expect("unread 3");
    for n in 1..=2 {
        assert!(
            read_read_at(&ctx, n * 10).await.is_some(),
            "ch {n} must stay read (below the un-check point)"
        );
    }
    for n in 3..=5 {
        assert!(
            read_read_at(&ctx, n * 10).await.is_none(),
            "ch {n} must cascade-unread to the un-check point"
        );
    }
}

#[tokio::test]
async fn count_read_returns_max_chapter_number_div_10() {
    let ctx = setup().await;
    insert_chapter(&ctx, 10, false).await;
    insert_chapter(&ctx, 20, true).await;
    insert_chapter(&ctx, 30, true).await;

    let n = count_read(&ctx.pool, "mangaupdates", &SERIES_ID.to_string())
        .await
        .expect("count");
    assert_eq!(n, 3, "must return highest read ch / 10");
}

#[tokio::test]
async fn update_progress_from_read_uses_greatest_semantics() {
    let ctx = setup().await;
    let user_id: Uuid = sqlx::query_scalar("SELECT id FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .fetch_one(&ctx.pool)
        .await
        .expect("get fixture user id");

    let media_id = fixture_tracking(&ctx, user_id, 5).await;
    update_progress_from_read(&ctx.pool, user_id, media_id, 10)
        .await
        .expect("update to 10");
    assert_eq!(read_progress(&ctx, user_id, media_id).await, 10);

    update_progress_from_read(&ctx.pool, user_id, media_id, 3)
        .await
        .expect("update to 3");
    assert_eq!(
        read_progress(&ctx, user_id, media_id).await,
        10,
        "progress must never regress"
    );
}

#[tokio::test]
async fn store_chapters_mu_creates_skeleton() {
    let ctx = setup().await;
    let inserted = store_chapters_mu(&ctx.pool, SERIES_ID, 5).await.expect("store 5");
    assert_eq!(inserted, 5, "must insert 5 chapters");

    let chapters = get_chapters(&ctx.pool, "mangaupdates", &SERIES_ID.to_string())
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 5);
    assert_eq!(chapters[0].chapter_number, 10);
    assert_eq!(chapters[4].chapter_number, 50);

    let ch = get_chapter(&ctx.pool, "mangaupdates", &SERIES_ID.to_string(), 30)
        .await
        .expect("get ch 3")
        .expect("ch 3 must exist");
    assert!(!ch.read);
}

#[tokio::test]
async fn store_chapters_mu_idempotent() {
    let ctx = setup().await;
    store_chapters_mu(&ctx.pool, SERIES_ID, 3).await.expect("store 3 first time");
    let inserted = store_chapters_mu(&ctx.pool, SERIES_ID, 3).await.expect("store 3 again");
    assert_eq!(inserted, 0, "second store must be idempotent (0 rows affected)");

    let chapters = get_chapters(&ctx.pool, "mangaupdates", &SERIES_ID.to_string())
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 3, "still exactly 3 chapters");
}

#[tokio::test]
async fn fractional_chapter_roundtrip() {
    let ctx = setup().await;
    insert_chapter(&ctx, 105, false).await;

    let ch = get_chapter(&ctx.pool, "mangaupdates", &SERIES_ID.to_string(), 105)
        .await
        .expect("get 10.5")
        .expect("must exist");
    assert_eq!(ch.chapter_number, 105);
    assert!(!ch.read);

    set_read(&ctx.pool, "mangaupdates", &SERIES_ID.to_string(), 105, true)
        .await
        .expect("read 10.5");
    let ch = get_chapter(&ctx.pool, "mangaupdates", &SERIES_ID.to_string(), 105)
        .await
        .expect("get 10.5 again")
        .expect("must exist");
    assert!(ch.read);
}
