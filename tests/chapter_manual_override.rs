mod common;

use mediatracker::services::chapters::{
    clear_manual_chapters, count_read, get_chapter_meta, get_chapters, set_manual_chapters,
    set_read,
};
use uuid::Uuid;

const SERIES_ID: i64 = 999_999_991;
const FIXTURE_USERNAME: &str = "test_chapter_manual_user";

async fn setup() -> (common::TestContext, Uuid) {
    let ctx = common::TestContext::new().await;
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .execute(&ctx.pool)
        .await
        .expect("delete fixture user");
    sqlx::query(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, 'test_chapter_manual@example.com', 'fakehash', 'user')",
    )
    .bind(FIXTURE_USERNAME)
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
    sqlx::query("DELETE FROM media_items WHERE provider = 'mangaupdates' AND external_id = $1")
        .bind(SERIES_ID.to_string())
        .execute(&ctx.pool)
        .await
        .expect("delete stale media item");

    // Auto count = 145, skeleton = 145.
    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title, chapters) \
         VALUES ('mangaupdates', $1, 'manhwa', 'Manual Override Manga', 145)",
    )
    .bind(SERIES_ID.to_string())
    .execute(&ctx.pool)
    .await
    .expect("insert media item");
    let nums: Vec<i32> = (1..=145).map(|c| c * 100).collect();
    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number) \
         SELECT 'mangaupdates', $1, u.chapter_number FROM UNNEST($2::int[]) AS u(chapter_number) \
         ON CONFLICT DO NOTHING",
    )
    .bind(SERIES_ID.to_string())
    .bind(&nums)
    .execute(&ctx.pool)
    .await
    .expect("insert skeleton");

    (ctx, user_id)
}

fn series_str() -> String {
    SERIES_ID.to_string()
}

#[tokio::test]
async fn manual_override_sets_flag_and_extends_skeleton() {
    let (ctx, user_id) = setup().await;

    set_manual_chapters(&ctx.pool, "mangaupdates", &series_str(), 157)
        .await
        .expect("set manual 157");

    let meta = get_chapter_meta(&ctx.pool, "mangaupdates", &series_str())
        .await
        .expect("meta")
        .expect("row");
    assert_eq!(meta.0, Some(157));
    assert!(meta.1, "chapters_manual must be TRUE");
    assert_eq!(meta.2.as_deref(), Some("manual"));

    // Skeleton extended: chapters 146..157 (stored x100) now exist and are tickable.
    let chapters = get_chapters(&ctx.pool, "mangaupdates", &series_str(), user_id)
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 157);
    assert_eq!(chapters.last().unwrap().chapter_number, 15700);
}

#[tokio::test]
async fn set_read_beyond_skeleton_records_exact_chapter_when_manual() {
    let (ctx, user_id) = setup().await;
    set_manual_chapters(&ctx.pool, "mangaupdates", &series_str(), 157)
        .await
        .expect("set manual 157");

    // Chapter 150 was beyond the original 145 skeleton.
    let updated = set_read(
        &ctx.pool,
        user_id,
        "mangaupdates",
        &series_str(),
        15000,
        true,
    )
    .await
    .expect("mark 150");
    assert!(updated);

    let n = count_read(&ctx.pool, user_id, "mangaupdates", &series_str())
        .await
        .expect("count");
    assert_eq!(n, 150, "highest read chapter must be 150");
}

#[tokio::test]
async fn reset_clears_flag_but_keeps_count() {
    let (ctx, _user_id) = setup().await;
    set_manual_chapters(&ctx.pool, "mangaupdates", &series_str(), 157)
        .await
        .expect("set manual");
    clear_manual_chapters(&ctx.pool, "mangaupdates", &series_str())
        .await
        .expect("clear");

    let meta = get_chapter_meta(&ctx.pool, "mangaupdates", &series_str())
        .await
        .expect("meta")
        .expect("row");
    assert!(!meta.1, "flag cleared");
    assert_eq!(meta.2, None, "source cleared");
    assert_eq!(meta.0, Some(157), "count kept until next auto refresh");
}
