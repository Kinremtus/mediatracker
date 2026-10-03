//! Batch 12 contract tests: migration 027 columns, `apply_meta` merge
//! semantics (COALESCE + schedule), raise-only invariance and the source
//! registry whitelist.
//!
//! Each test runs against its own throwaway Postgres container
//! (`common::TestContext`), so the fixtures below never collide with the
//! other integration suites.

mod common;

use std::sync::Arc;

use chrono::NaiveDate;
use mediatracker::services::chapter_enrich::enrich_with_providers;
use mediatracker::services::chapter_meta_enrich::apply_meta;
use mediatracker::services::external::chapter_count::{
    ChapterCount, ChapterCountFuture, ChapterCountProvider,
};
use mediatracker::services::external::official_meta::{ChapterMeta, OfficialMeta};
use mediatracker::services::source_ids::{KNOWN_SOURCES, is_known_source, set_source_id};

const PROVIDER: &str = "mangaupdates";
const EXTERNAL_ID: &str = "official-sources-contract";

/// Minimal `OfficialMeta` builder for the merge tests.
fn meta(
    title: Option<&str>,
    release_date: Option<NaiveDate>,
    update_schedule: Option<&str>,
    next_update_at: Option<NaiveDate>,
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
        next_update_at,
    }
}

/// Fixed-reading provider: no network, reports `total` unconditionally.
struct FixedProvider {
    name: &'static str,
    total: i32,
}

impl ChapterCountProvider for FixedProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    fn fetch<'a>(&'a self, _external_id: &'a str) -> ChapterCountFuture<'a> {
        let total = self.total;
        Box::pin(async move { Ok(Some(ChapterCount { total, source: "auto:test" })) })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manga"]
    }
}

async fn setup(chapters: i32) -> common::TestContext {
    let ctx = common::TestContext::new().await;

    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title, chapters) \
         VALUES ($1, $2, 'manga', 'Official Sources Contract', $3)",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .bind(chapters)
    .execute(&ctx.pool)
    .await
    .expect("insert media_items");

    ctx
}

async fn chapter_row(
    pool: &sqlx::PgPool,
) -> (Option<String>, Option<NaiveDate>, Option<String>, Option<NaiveDate>) {
    sqlx::query_as(
        "SELECT title_en, release_date, update_schedule, next_update_at \
         FROM series_chapters sc \
         LEFT JOIN media_items mi \
           ON mi.provider = sc.provider AND mi.external_id = sc.external_id \
         WHERE sc.provider = $1 AND sc.external_id = $2 AND sc.chapter_number = 100",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(pool)
    .await
    .expect("read chapter + schedule row")
}

async fn item_columns(
    pool: &sqlx::PgPool,
) -> (Option<i32>, Option<String>, Option<NaiveDate>) {
    sqlx::query_as(
        "SELECT chapters, update_schedule, next_update_at \
         FROM media_items WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(pool)
    .await
    .expect("read media_items row")
}

/// Migration 027 must have landed: `update_schedule` (TEXT) and
/// `next_update_at` (DATE) on `media_items`.
#[tokio::test]
async fn migration_027_columns_exist() {
    let ctx = common::TestContext::new().await;

    let cols = sqlx::query_as::<_, (String, String)>(
        "SELECT column_name, data_type FROM information_schema.columns \
         WHERE table_name = 'media_items' \
           AND column_name IN ('update_schedule', 'next_update_at') \
         ORDER BY column_name",
    )
    .fetch_all(&ctx.pool)
    .await
    .expect("query information_schema");

    assert_eq!(
        cols,
        vec![
            ("next_update_at".to_string(), "date".to_string()),
            ("update_schedule".to_string(), "text".to_string()),
        ],
        "migration 027 must add both official-source columns"
    );
}

/// `apply_meta` COALESCE rules: NULL fields fill in, non-NULL fields survive;
/// schedule writes only non-empty values and `None` never erases.
#[tokio::test]
async fn apply_meta_coalesce_and_schedule_semantics() {
    let ctx = setup(0).await;
    let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    let next = NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();

    // Row starts NULL/NULL.
    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number) \
         VALUES ($1, $2, 100)",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("insert bare chapter");

    // 1. NULL title + NULL date fill from the source; schedule + next land.
    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        &meta(Some("Official"), Some(date), Some("매주 월요일"), Some(next)),
    )
    .await
    .expect("first apply_meta");

    let (title, release, schedule, next_at) = chapter_row(&ctx.pool).await;
    assert_eq!(title.as_deref(), Some("Official"), "NULL title fills");
    assert_eq!(release, Some(date), "NULL date fills");
    assert_eq!(schedule.as_deref(), Some("매주 월요일"));
    assert_eq!(next_at, Some(next));

    // 2. A second source reading must not clobber stored non-NULL values.
    let later = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        &meta(
            Some("Clobber Attempt"),
            Some(later),
            Some("毎週 金曜日 19:00"),
            None,
        ),
    )
    .await
    .expect("second apply_meta");

    let (title, release, schedule, next_at) = chapter_row(&ctx.pool).await;
    assert_eq!(title.as_deref(), Some("Official"), "non-NULL title survives");
    assert_eq!(release, Some(date), "non-NULL date survives");
    assert_eq!(schedule.as_deref(), Some("毎週 金曜日 19:00"), "non-empty schedule updates");
    assert_eq!(next_at, Some(next), "None next_update_at does not erase");

    // 3. A source without a schedule must not wipe the stored one.
    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        &meta(None, None, None, None),
    )
    .await
    .expect("third apply_meta");

    let (_, _, schedule, next_at) = chapter_row(&ctx.pool).await;
    assert_eq!(
        schedule.as_deref(),
        Some("毎週 金曜日 19:00"),
        "absent schedule must not overwrite the stored value"
    );
    assert_eq!(next_at, Some(next), "absent next date must not erase");
}

/// Raise-only invariance: a lower reading from `enrich_*` and any
/// `apply_meta` call leave `media_items.chapters` untouched.
#[tokio::test]
async fn chapters_count_is_raise_only() {
    let ctx = setup(100).await;
    set_source_id(&ctx.pool, PROVIDER, EXTERNAL_ID, "kakao", "contract-bind")
        .await
        .expect("bind source id");

    // Lower reading (60 < 100): no write at all.
    let applied =
        enrich_with_providers(&ctx.pool, PROVIDER, EXTERNAL_ID, vec![Arc::new(FixedProvider {
            name: "kakao",
            total: 60,
        })])
        .await;
    assert_eq!(applied, None, "a lower reading must be ignored");
    let (chapters, _, _) = item_columns(&ctx.pool).await;
    assert_eq!(chapters, Some(100), "lower reading must not lower chapters");

    // `apply_meta` never writes `chapters` at all.
    let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        &meta(Some("Official"), Some(date), Some("不定时更新"), None),
    )
    .await
    .expect("apply_meta");
    let (chapters, schedule, _) = item_columns(&ctx.pool).await;
    assert_eq!(chapters, Some(100), "apply_meta must not touch chapters");
    assert_eq!(schedule.as_deref(), Some("不定时更新"));

    // Sanity: a higher reading still raises (the invariance is raise-only,
    // not freeze).
    let applied =
        enrich_with_providers(&ctx.pool, PROVIDER, EXTERNAL_ID, vec![Arc::new(FixedProvider {
            name: "kakao",
            total: 120,
        })])
        .await;
    assert_eq!(applied, Some(120));
    let (chapters, _, _) = item_columns(&ctx.pool).await;
    assert_eq!(chapters, Some(120), "a higher reading still raises");
}

/// Registry: every `KNOWN_SOURCES` key passes `is_known_source`; `narou` is
/// not a key (it reuses `syosetu`).
#[test]
fn registry_accepts_every_known_source_and_rejects_narou() {
    for source in KNOWN_SOURCES {
        assert!(is_known_source(source), "{source} must be a known source");
    }
    assert!(!is_known_source("narou"), "narou reuses syosetu and must not be bindable");
    assert!(!is_known_source(""), "empty source is never known");
}
