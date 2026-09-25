mod common;

use mediatracker::services::chapters::{
    count_read, format_chapter, get_chapter, get_chapters, parse_chapter, set_read,
    store_chapters_mu, update_progress_from_read,
};
use uuid::Uuid;

const SERIES_ID: i64 = 999_999_992;
const FIXTURE_USERNAME: &str = "test_chapter_persistence_user";
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

    sqlx::query("DELETE FROM series_chapters WHERE external_id = $1")
        .bind(SERIES_ID.to_string())
        .execute(&ctx.pool)
        .await
        .expect("delete stale chapters");

    (ctx, user_id)
}

fn series_str() -> String {
    SERIES_ID.to_string()
}

async fn insert_chapter(ctx: &common::TestContext, ch_num_10: i32) {
    sqlx::query(
        r#"
        INSERT INTO series_chapters
            (provider, external_id, chapter_number)
        VALUES
            ('mangaupdates', $1, $2)
        ON CONFLICT (provider, external_id, chapter_number) DO NOTHING
        "#,
    )
    .bind(series_str())
    .bind(ch_num_10)
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
    .bind(series_str())
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

/// Reads the per-user chapter progress row. `None` means no row exists (unread).
async fn read_progress_read(
    ctx: &common::TestContext,
    user_id: Uuid,
    ch_num_10: i32,
) -> Option<bool> {
    let row: Option<(bool,)> = sqlx::query_as(
        "SELECT read FROM user_chapter_progress \
         WHERE user_id = $1 AND provider = 'mangaupdates' AND external_id = $2 AND chapter_number = $3",
    )
    .bind(user_id)
    .bind(series_str())
    .bind(ch_num_10)
    .fetch_optional(&ctx.pool)
    .await
    .expect("read progress read");
    row.map(|(v,)| v)
}

async fn read_progress_read_at(
    ctx: &common::TestContext,
    user_id: Uuid,
    ch_num_10: i32,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let row: Option<(Option<chrono::DateTime<chrono::Utc>>,)> = sqlx::query_as(
        "SELECT read_at FROM user_chapter_progress \
         WHERE user_id = $1 AND provider = 'mangaupdates' AND external_id = $2 AND chapter_number = $3",
    )
    .bind(user_id)
    .bind(series_str())
    .bind(ch_num_10)
    .fetch_optional(&ctx.pool)
    .await
    .expect("read read_at");
    row.and_then(|(v,)| v)
}

// --- Unit tests (no DB) -----------------------

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

// --- DB integration tests ---------------------

#[tokio::test]
async fn set_read_roundtrip() {
    let (ctx, user_id) = setup().await;
    insert_chapter(&ctx, 10).await;

    let updated = set_read(&ctx.pool, user_id, "mangaupdates", &series_str(), 10, true)
        .await
        .expect("set true");
    assert!(updated);
    assert!(
        read_progress_read_at(&ctx, user_id, 10).await.is_some(),
        "read_at must be set"
    );
    assert_eq!(read_progress_read(&ctx, user_id, 10).await, Some(true));

    let updated = set_read(&ctx.pool, user_id, "mangaupdates", &series_str(), 10, false)
        .await
        .expect("set false");
    assert!(updated);
    assert!(
        read_progress_read_at(&ctx, user_id, 10).await.is_none(),
        "read_at must be cleared"
    );
}

#[tokio::test]
async fn set_read_bulk_fills_below_and_cascades_above() {
    let (ctx, user_id) = setup().await;
    for n in 1..=5 {
        insert_chapter(&ctx, n * 10).await;
    }

    set_read(&ctx.pool, user_id, "mangaupdates", &series_str(), 30, true)
        .await
        .expect("mark 3");
    for n in 1..=3 {
        assert_eq!(
            read_progress_read(&ctx, user_id, n * 10).await,
            Some(true),
            "ch {n} must be read after bulk-fill from ch 3"
        );
    }
    for n in 4..=5 {
        assert_eq!(
            read_progress_read(&ctx, user_id, n * 10).await,
            None,
            "ch {n} must remain unread (above the bulk-fill point)"
        );
    }

    set_read(&ctx.pool, user_id, "mangaupdates", &series_str(), 30, false)
        .await
        .expect("unread 3");
    for n in 1..=2 {
        assert_eq!(
            read_progress_read(&ctx, user_id, n * 10).await,
            Some(true),
            "ch {n} must stay read (below the un-check point)"
        );
    }
    // ch 3 had a progress row (it was bulk-filled), so un-checking flips it to
    // read=false. ch 4-5 never had a row: per-user progress treats an absent
    // row as unread, so they must simply not be read.
    assert_eq!(
        read_progress_read(&ctx, user_id, 30).await,
        Some(false),
        "ch 3 must cascade-unread to the un-check point"
    );
    for n in 4..=5 {
        assert_ne!(
            read_progress_read(&ctx, user_id, n * 10).await,
            Some(true),
            "ch {n} must not be read (absent progress row)"
        );
    }
}

#[tokio::test]
async fn count_read_returns_max_chapter_number_div_10() {
    let (ctx, user_id) = setup().await;
    insert_chapter(&ctx, 10).await;
    insert_chapter(&ctx, 20).await;
    insert_chapter(&ctx, 30).await;

    set_read(&ctx.pool, user_id, "mangaupdates", &series_str(), 30, true)
        .await
        .expect("mark 3");

    let n = count_read(&ctx.pool, user_id, "mangaupdates", &series_str())
        .await
        .expect("count");
    assert_eq!(n, 3, "must return highest read ch / 10");
}

#[tokio::test]
async fn update_progress_from_read_uses_greatest_semantics() {
    let (ctx, user_id) = setup().await;

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
    let (ctx, user_id) = setup().await;
    let inserted = store_chapters_mu(&ctx.pool, SERIES_ID, 5)
        .await
        .expect("store 5");
    assert_eq!(inserted, 5, "must insert 5 chapters");

    let chapters = get_chapters(&ctx.pool, "mangaupdates", &series_str(), user_id)
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 5);
    assert_eq!(chapters[0].chapter_number, 10);
    assert_eq!(chapters[4].chapter_number, 50);

    let ch = get_chapter(&ctx.pool, "mangaupdates", &series_str(), 30, user_id)
        .await
        .expect("get ch 3")
        .expect("ch 3 must exist");
    assert!(!ch.read);
}

#[tokio::test]
async fn store_chapters_mu_idempotent() {
    let (ctx, user_id) = setup().await;
    store_chapters_mu(&ctx.pool, SERIES_ID, 3)
        .await
        .expect("store 3 first time");
    let inserted = store_chapters_mu(&ctx.pool, SERIES_ID, 3)
        .await
        .expect("store 3 again");
    assert_eq!(
        inserted, 0,
        "second store must be idempotent (0 rows affected)"
    );

    let chapters = get_chapters(&ctx.pool, "mangaupdates", &series_str(), user_id)
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 3, "still exactly 3 chapters");
}

#[tokio::test]
async fn fractional_chapter_roundtrip() {
    let (ctx, user_id) = setup().await;
    insert_chapter(&ctx, 105).await;

    let ch = get_chapter(&ctx.pool, "mangaupdates", &series_str(), 105, user_id)
        .await
        .expect("get 10.5")
        .expect("must exist");
    assert_eq!(ch.chapter_number, 105);
    assert!(!ch.read);

    set_read(&ctx.pool, user_id, "mangaupdates", &series_str(), 105, true)
        .await
        .expect("read 10.5");
    let ch = get_chapter(&ctx.pool, "mangaupdates", &series_str(), 105, user_id)
        .await
        .expect("get 10.5 again")
        .expect("must exist");
    assert!(ch.read);
}
