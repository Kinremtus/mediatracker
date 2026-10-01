//! `sync_media_items_chapters` must not raise `media_items.chapters` above a manual pin.
mod common;

use mediatracker::services::chapters::store_chapters_mu;

const SERIES_ID: i64 = 999_999_993;

async fn setup() -> common::TestContext {
    let ctx = common::TestContext::new().await;
    let series = SERIES_ID.to_string();
    sqlx::query("DELETE FROM series_chapters WHERE external_id = $1")
        .bind(&series)
        .execute(&ctx.pool)
        .await
        .expect("delete stale chapters");
    sqlx::query("DELETE FROM media_items WHERE provider = 'mangaupdates' AND external_id = $1")
        .bind(&series)
        .execute(&ctx.pool)
        .await
        .expect("delete stale media item");
    ctx
}

fn series_str() -> String {
    SERIES_ID.to_string()
}

async fn insert_item(ctx: &common::TestContext, chapters: i32, manual: bool) {
    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title, chapters, chapters_manual) \
         VALUES ('mangaupdates', $1, 'manhwa', 'Sync Guard Fixture', $2, $3)",
    )
    .bind(series_str())
    .bind(chapters)
    .bind(manual)
    .execute(&ctx.pool)
    .await
    .expect("insert media item");
}

async fn chapters(ctx: &common::TestContext) -> i32 {
    sqlx::query_scalar(
        "SELECT chapters FROM media_items WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(series_str())
    .fetch_one(&ctx.pool)
    .await
    .expect("read chapters")
}

#[tokio::test]
async fn sync_does_not_raise_above_manual_pin() {
    let ctx = setup().await;
    insert_item(&ctx, 145, true).await;

    // store_chapters_mu materialises 1..=200 and calls sync_media_items_chapters.
    store_chapters_mu(&ctx.pool, SERIES_ID, 200)
        .await
        .expect("store 200");

    assert_eq!(
        chapters(&ctx).await,
        145,
        "manual pin must survive the skeleton sync"
    );
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::bigint FROM series_chapters \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(series_str())
    .fetch_one(&ctx.pool)
    .await
    .expect("count skeleton");
    assert_eq!(n, 200, "the skeleton itself must still materialise");
}

#[tokio::test]
async fn sync_still_raises_when_not_manual() {
    let ctx = setup().await;
    insert_item(&ctx, 145, false).await;

    store_chapters_mu(&ctx.pool, SERIES_ID, 200)
        .await
        .expect("store 200");

    assert_eq!(
        chapters(&ctx).await,
        200,
        "non-manual count raises normally"
    );
}
