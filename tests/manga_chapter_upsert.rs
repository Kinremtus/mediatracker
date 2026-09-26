//! Behavioural tests for MangaDex chapter upsert:
//! fractional chapters are created, duplicates never appear, and repeated
//! enrichment preserves already-filled columns.

mod common;

use mediatracker::services::chapters::{
    MdChapterRow, get_chapters, store_chapters_mu, upsert_series_chapters,
};
use uuid::Uuid;

const SERIES_ID: i64 = 999_999_991;
const MEDIA_TITLE: &str = "Upsert Manga";

fn series_str() -> String {
    SERIES_ID.to_string()
}

async fn setup() -> common::TestContext {
    let ctx = common::TestContext::new().await;
    let ext = series_str();

    sqlx::query("DELETE FROM series_chapters WHERE external_id = $1")
        .bind(&ext)
        .execute(&ctx.pool)
        .await
        .expect("cleanup chapters");
    sqlx::query("DELETE FROM media_items WHERE provider = 'mangaupdates' AND external_id = $1")
        .bind(&ext)
        .execute(&ctx.pool)
        .await
        .expect("cleanup media");

    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ('mangaupdates', $1, 'manga', $2)",
    )
    .bind(&ext)
    .bind(MEDIA_TITLE)
    .execute(&ctx.pool)
    .await
    .expect("insert media_items");

    ctx
}

fn fractional_rows() -> Vec<MdChapterRow> {
    vec![
        MdChapterRow {
            chapter_number: 550,
            title_en: Some("5.5 Special".to_string()),
            title_ru: None,
            volume: Some(1),
            release_date: chrono::NaiveDate::from_ymd_opt(2019, 4, 1),
        },
        MdChapterRow {
            chapter_number: 10110,
            title_en: Some("101.1 Extra".to_string()),
            title_ru: Some("101.1 Экстра".to_string()),
            volume: Some(11),
            release_date: None,
        },
    ]
}

#[tokio::test]
async fn upsert_adds_fractional_chapters_and_is_idempotent() {
    let ctx = setup().await;
    let ext = series_str();

    // Integer skeleton first (as created from MangaUpdates latest_chapter).
    store_chapters_mu(&ctx.pool, SERIES_ID, 3)
        .await
        .expect("skeleton");

    let first = upsert_series_chapters(&ctx.pool, "mangaupdates", &ext, &fractional_rows())
        .await
        .expect("upsert");
    assert_eq!(first, 2, "both fractional chapters inserted");

    let user = Uuid::nil();
    let chapters = get_chapters(&ctx.pool, "mangaupdates", &ext, user)
        .await
        .expect("read chapters");
    let mut numbers: Vec<i32> = chapters.iter().map(|c| c.chapter_number).collect();
    numbers.sort_unstable();
    assert_eq!(
        numbers,
        vec![100, 200, 300, 550, 10110],
        "3 skeleton + 2 fractional, ordered by stored number"
    );

    let extra = chapters
        .iter()
        .find(|c| c.chapter_number == 10110)
        .expect("101.1 present");
    assert_eq!(extra.title_en.as_deref(), Some("101.1 Extra"));
    assert_eq!(extra.title_ru.as_deref(), Some("101.1 Экстра"));
    assert_eq!(extra.volume, Some(11));
    assert_eq!(extra.formatted(), "101.1");

    let special = chapters
        .iter()
        .find(|c| c.chapter_number == 550)
        .expect("5.5 present");
    assert_eq!(
        special.release_date,
        chrono::NaiveDate::from_ymd_opt(2019, 4, 1)
    );
    assert_eq!(special.formatted(), "5.5");

    // Re-run: no new rows; COALESCE keeps the first non-null title.
    let again = upsert_series_chapters(&ctx.pool, "mangaupdates", &ext, &fractional_rows())
        .await
        .expect("re-upsert");
    assert_eq!(again, 2, "two input rows touched, none inserted");

    let chapters = get_chapters(&ctx.pool, "mangaupdates", &ext, Uuid::nil())
        .await
        .expect("read chapters again");
    assert_eq!(
        chapters.len(),
        5,
        "repeated enrich must not create duplicates"
    );
    let extra = chapters
        .iter()
        .find(|c| c.chapter_number == 10110)
        .expect("101.1 still present");
    assert_eq!(
        extra.title_en.as_deref(),
        Some("101.1 Extra"),
        "existing title preserved (COALESCE, not overwrite)"
    );
}
