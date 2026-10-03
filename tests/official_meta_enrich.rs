mod common;

use chrono::NaiveDate;
use mediatracker::services::chapter_meta_enrich::apply_meta;
use mediatracker::services::external::official_meta::{ChapterMeta, OfficialMeta};

const PROVIDER: &str = "mangaupdates";
const EXTERNAL_ID: &str = "1";

fn meta(
    title: Option<&str>,
    release_date: Option<NaiveDate>,
    update_schedule: Option<&str>,
) -> OfficialMeta {
    OfficialMeta {
        source: "test".into(),
        chapters: vec![ChapterMeta {
            number_x100: 100,
            title: title.map(str::to_string),
            release_date,
        }],
        count: Some(1),
        update_schedule: update_schedule.map(str::to_string),
        next_update_at: None,
    }
}

async fn setup() -> common::TestContext {
    let ctx = common::TestContext::new().await;

    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ($1, $2, 'manga', 'Enrich Fixture')",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("insert media_items");

    sqlx::query(
        "INSERT INTO series_chapters \
             (provider, external_id, chapter_number, title_en) \
         VALUES ($1, $2, 100, 'Existing')",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("insert series_chapters");

    ctx
}

#[tokio::test]
async fn coalesce_preserves_existing_title_and_fills_date() {
    let ctx = setup().await;
    let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();

    let written = apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, &meta(Some("New"), Some(date), None))
        .await
        .expect("apply_meta");
    assert_eq!(written, 1, "the single chapter row must be written");

    let (title, release): (Option<String>, Option<NaiveDate>) = sqlx::query_as(
        "SELECT title_en, release_date FROM series_chapters \
         WHERE provider = $1 AND external_id = $2 AND chapter_number = 100",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("read chapter");

    assert_eq!(
        title.as_deref(),
        Some("Existing"),
        "COALESCE must never clobber a non-NULL title"
    );
    assert_eq!(release, Some(date), "release_date must be filled in");
}

#[tokio::test]
async fn schedule_is_set_then_preserved_when_absent() {
    let ctx = setup().await;

    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        &meta(None, None, Some("매주 월요일")),
    )
    .await
    .expect("apply schedule");

    let schedule: Option<String> = sqlx::query_scalar(
        "SELECT update_schedule FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("read schedule");
    assert_eq!(schedule.as_deref(), Some("매주 월요일"));

    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, &meta(None, None, None))
        .await
        .expect("apply empty schedule");

    let schedule: Option<String> = sqlx::query_scalar(
        "SELECT update_schedule FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("read schedule again");
    assert_eq!(
        schedule.as_deref(),
        Some("매주 월요일"),
        "an absent schedule must not overwrite the stored value"
    );
}
