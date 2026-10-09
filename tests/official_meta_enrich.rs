//! `apply_meta` contract: chapter rows land under the resolved chapter key, the
//! `media_items.chapters` counter is raise-only, and MU orphans are cleaned up
//! without touching read / manual rows.

mod common;

use std::sync::{Arc, Mutex};

use chrono::NaiveDate;
use mediatracker::services::chapter_meta_enrich::{
    MetaFetcher, apply_meta, enrich_official_meta_for_key, enrich_official_meta_with,
};
use mediatracker::services::external::official_meta::{ChapterMeta, MetaFuture, OfficialMeta};

const PROVIDER: &str = "mangaupdates";
const EXTERNAL_ID: &str = "1";
/// Meta-capable source used for the binding fixtures (`SOURCES_WITH_META`).
const SOURCE: &str = "bilibili";
const SOURCE_ID: &str = "mc32437";

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
            is_bonus: false,
        }],
        count: Some(1),
        update_schedule: update_schedule.map(str::to_string),
        next_update_at: None,
    }
}

fn main_chapter(number_x100: i32) -> ChapterMeta {
    ChapterMeta {
        number_x100,
        title: Some(format!("Chapter {number_x100}")),
        release_date: None,
        is_bonus: false,
    }
}

fn bonus_chapter(number_x100: i32) -> ChapterMeta {
    ChapterMeta {
        number_x100,
        title: Some(format!("Bonus {number_x100}")),
        release_date: None,
        is_bonus: true,
    }
}

/// A `media_items` row (manga, no counter yet) plus one MU-skeleton chapter.
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

async fn bind(ctx: &common::TestContext, source: &str, source_id: &str) {
    sqlx::query(
        "INSERT INTO media_source_ids (provider, external_id, source, source_id) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .bind(source)
    .bind(source_id)
    .execute(&ctx.pool)
    .await
    .expect("insert media_source_ids binding");
}

/// Seed a chapter row with an explicit `origin` / `read` state.
async fn seed_chapter(
    ctx: &common::TestContext,
    provider: &str,
    external_id: &str,
    number_x100: i32,
    origin: &str,
    read: bool,
    is_bonus: bool,
) {
    sqlx::query(
        "INSERT INTO series_chapters \
             (provider, external_id, chapter_number, origin, read, read_at, is_bonus) \
         VALUES ($1, $2, $3, $4, $5, CASE WHEN $5 THEN NOW() ELSE NULL END, $6)",
    )
    .bind(provider)
    .bind(external_id)
    .bind(number_x100)
    .bind(origin)
    .bind(read)
    .bind(is_bonus)
    .execute(&ctx.pool)
    .await
    .expect("seed series_chapters row");
}

async fn seed_user(ctx: &common::TestContext) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ('meta-enrich', 'meta-enrich@example.com', 'fakehash', 'user') \
         RETURNING id",
    )
    .fetch_one(&ctx.pool)
    .await
    .expect("insert user")
}

async fn seed_read_progress(
    ctx: &common::TestContext,
    user_id: uuid::Uuid,
    provider: &str,
    external_id: &str,
    number_x100: i32,
) {
    sqlx::query(
        "INSERT INTO user_chapter_progress \
             (user_id, provider, external_id, chapter_number, read, read_at) \
         VALUES ($1, $2, $3, $4, TRUE, NOW())",
    )
    .bind(user_id)
    .bind(provider)
    .bind(external_id)
    .bind(number_x100)
    .execute(&ctx.pool)
    .await
    .expect("seed user_chapter_progress");
}

/// `(provider, external_id, chapter_number, origin, is_bonus)` of every row.
async fn chapter_rows(ctx: &common::TestContext) -> Vec<(String, String, i32, String, bool)> {
    sqlx::query_as(
        "SELECT provider, external_id, chapter_number, origin, is_bonus \
         FROM series_chapters ORDER BY provider, external_id, chapter_number, origin",
    )
    .fetch_all(&ctx.pool)
    .await
    .expect("read series_chapters")
}

async fn counter_columns(ctx: &common::TestContext) -> (Option<i32>, Option<String>) {
    sqlx::query_as(
        "SELECT chapters, chapters_source FROM media_items \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_one(&ctx.pool)
    .await
    .expect("read media_items counter")
}

#[tokio::test]
async fn coalesce_preserves_existing_title_and_fills_date() {
    let ctx = setup().await;
    let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();

    let written = apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        "test",
        &meta(Some("New"), Some(date), None),
    )
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
        "test",
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

    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        "test",
        &meta(None, None, None),
    )
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

/// Without a meta-capable binding the chapter rows stay under the MU key, and
/// the merged MU row keeps `origin='mu'` (the source is only stamped on inserts).
#[tokio::test]
async fn without_binding_rows_stay_under_the_mu_key() {
    let ctx = setup().await;

    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        SOURCE,
        &meta(Some("Fresh"), None, None),
    )
    .await
    .expect("apply_meta");

    let rows = chapter_rows(&ctx).await;
    assert_eq!(
        rows,
        vec![(
            PROVIDER.to_string(),
            EXTERNAL_ID.to_string(),
            100,
            "mu".to_string(),
            false
        )],
        "the merged MU skeleton row keeps origin='mu' and its title"
    );
}

/// A meta-capable binding wins: rows are written under the source key and the
/// MU row is left alone (no key moving, no duplicate under the MU key).
#[tokio::test]
async fn binding_moves_new_rows_under_the_source_key() {
    let ctx = setup().await;
    bind(&ctx, SOURCE, SOURCE_ID).await;

    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        SOURCE,
        &meta(Some("Official title"), None, None),
    )
    .await
    .expect("apply_meta");

    let rows = chapter_rows(&ctx).await;
    assert_eq!(
        rows,
        vec![
            (
                SOURCE.to_string(),
                SOURCE_ID.to_string(),
                100,
                SOURCE.to_string(),
                false
            ),
            (
                PROVIDER.to_string(),
                EXTERNAL_ID.to_string(),
                100,
                "mu".to_string(),
                false
            ),
        ],
        "the official row belongs to the bound source key, the MU row is untouched"
    );
}

/// The counter is raised (raise-only) and stamped with `official_<source>`.
#[tokio::test]
async fn counter_is_raised_and_stamped_with_the_source() {
    let ctx = setup().await;
    let mut m = meta(None, None, None);
    m.count = Some(9);

    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(9), Some("official_bilibili".to_string())),
        "a higher official count raises chapters and stamps its source"
    );

    // A lower reading never lowers it.
    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        SOURCE,
        &meta(None, None, None),
    )
    .await
    .expect("apply lower count");
    assert_eq!(
        counter_columns(&ctx).await.0,
        Some(9),
        "raise-only: a lower official count must not lower chapters"
    );
    assert_eq!(
        counter_columns(&ctx).await.1,
        Some("official_bilibili".to_string()),
        "chapters_source is written ONLY together with a raise -> a declined \
         raise must keep the previous stamp"
    );
}

/// A hand-set counter (`chapters_manual = TRUE`) is never overwritten.
#[tokio::test]
async fn manual_counter_is_never_overwritten() {
    let ctx = setup().await;
    sqlx::query(
        "UPDATE media_items SET chapters = 5, chapters_manual = TRUE \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("mark the counter manual");

    let mut m = meta(None, None, None);
    m.count = Some(9);
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(5), None),
        "a manual counter must survive even a higher official count"
    );
}

/// Drawer preview: no `media_items` row yet -> chapters are still stored under
/// the source key, only the counter/schedule are deferred.
#[tokio::test]
async fn chapters_are_stored_without_a_media_items_row() {
    let ctx = common::TestContext::new().await;
    bind(&ctx, SOURCE, SOURCE_ID).await;

    let m = meta(Some("Preview"), None, Some("매주 금요일"));
    // meta() already sets count: Some(1); explicit assignment removed as dead
    let written = apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta without media_items");
    assert_eq!(written, 1);

    assert_eq!(
        chapter_rows(&ctx).await,
        vec![(
            SOURCE.to_string(),
            SOURCE_ID.to_string(),
            100,
            SOURCE.to_string(),
            false
        )],
        "chapter rows must be written under the resolved key"
    );

    let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_items")
        .fetch_one(&ctx.pool)
        .await
        .expect("count media_items");
    assert_eq!(items, 0, "the fixture must not create a media_items row");
}

/// Orphan cleanup removes only unread `origin='mu'` rows the official answer no
/// longer mentions. Read rows, rows with a read progress entry and manual rows
/// survive.
#[tokio::test]
async fn orphan_cleanup_spares_read_manual_and_progress_rows() {
    let ctx = setup().await;
    let user_id = seed_user(&ctx).await;

    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 700, "mu", false, false).await; // orphan
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 800, "mu", false, false).await; // progress
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 900, "manual", false, false).await;
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 1000, "mu", true, false).await;
    seed_read_progress(&ctx, user_id, PROVIDER, EXTERNAL_ID, 800).await;

    apply_meta(
        &ctx.pool,
        PROVIDER,
        EXTERNAL_ID,
        SOURCE,
        &meta(None, None, None),
    )
    .await
    .expect("apply_meta");

    let numbers: Vec<i32> = chapter_rows(&ctx)
        .await
        .into_iter()
        .map(|(_, _, n, _, _)| n)
        .collect();
    assert_eq!(
        numbers,
        vec![100, 800, 900, 1000],
        "only the unread, untracked, origin='mu' orphan (700) may be removed"
    );
}

/// An empty chapter list must not wipe the MU fallback skeleton.
#[tokio::test]
async fn empty_chapter_list_deletes_nothing() {
    let ctx = setup().await;
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 700, "mu", false, false).await;

    // A meta with NO chapters at all (schedule-only answer: `is_useful_meta` still
    // lets it through, so `apply_meta` runs).
    let mut m = meta(None, None, Some("매주 월요일"));
    m.chapters.clear();
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    let numbers: Vec<i32> = chapter_rows(&ctx)
        .await
        .into_iter()
        .map(|(_, _, n, _, _)| n)
        .collect();
    assert_eq!(numbers, vec![100, 700], "no answer, no cleanup");
}

/// Bonus rows have no unique key, so a re-apply must rewrite this source's
/// bonuses instead of duplicating them — and a manual bonus must survive.
#[tokio::test]
async fn reapply_rewrites_bonus_rows_instead_of_duplicating_them() {
    let ctx = setup().await;
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 0, "manual", false, true).await;

    let mut m = meta(None, None, None);
    m.chapters = vec![main_chapter(100), bonus_chapter(0)];

    let first = apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("first apply_meta");
    assert_eq!(first, 2);
    let after_first = chapter_rows(&ctx).await;

    let second = apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("second apply_meta");
    assert_eq!(second, 2, "every row is rewritten, so every row counts");
    assert_eq!(
        chapter_rows(&ctx).await,
        after_first,
        "a re-apply must not duplicate bonus rows"
    );

    let bonuses: Vec<(i32, String)> = after_first
        .iter()
        .filter(|(_, _, _, _, is_bonus)| *is_bonus)
        .map(|(_, _, n, origin, _)| (*n, origin.clone()))
        .collect();
    assert_eq!(
        bonuses,
        vec![(0, "bilibili".to_string()), (0, "manual".to_string()),],
        "the manual bonus is kept next to the source's own bonus"
    );
}

/// A chapters-less answer (schedule-only) must not delete this source's stored
/// bonuses: `is_useful_meta` still lets such a meta through, so an unconditional
/// DELETE would wipe bonuses the source never re-sent (source changed its
/// layout / reported only the schedule).
#[tokio::test]
async fn empty_chapter_list_keeps_stored_bonus_rows() {
    let ctx = setup().await;

    let mut with_bonus = meta(None, None, None);
    with_bonus.chapters = vec![main_chapter(100), bonus_chapter(0)];
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &with_bonus)
        .await
        .expect("apply meta with a bonus");

    // Same source, same item, but this time it answered with a schedule only.
    let mut schedule_only = meta(None, None, Some("매주 월요일"));
    schedule_only.chapters.clear();
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &schedule_only)
        .await
        .expect("apply schedule-only meta");

    let bonuses: Vec<(i32, String)> = chapter_rows(&ctx)
        .await
        .into_iter()
        .filter(|(_, _, _, _, is_bonus)| *is_bonus)
        .map(|(_, _, n, origin, _)| (n, origin))
        .collect();
    assert_eq!(
        bonuses,
        vec![(0, SOURCE.to_string())],
        "an answer without chapters must not wipe already-stored bonuses"
    );
}

/// Offline chain fetcher: returns a canned `OfficialMeta` and records which
/// source ids the chain asked for (no network).
struct FixtureMetaFetcher {
    meta: OfficialMeta,
    seen: Arc<Mutex<Vec<(String, String)>>>,
}

impl FixtureMetaFetcher {
    fn new(meta: OfficialMeta) -> Self {
        Self {
            meta,
            seen: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn seen(&self) -> Vec<(String, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl MetaFetcher for FixtureMetaFetcher {
    fn fetch<'a>(&'a self, source: &'a str, source_id: &'a str) -> MetaFuture<'a> {
        let seen = self.seen.clone();
        let meta = self.meta.clone();
        let source = source.to_string();
        let source_id = source_id.to_string();
        Box::pin(async move {
            seen.lock().unwrap().push((source, source_id));
            Ok(meta)
        })
    }
}

/// Prod regression: the drawer rendered before the title was tracked (no
/// `media_items` row), so `enrich_official_meta_for_key` bailed out with Ok(0)
/// and the MU skeleton (271) stayed the only chapter source. With the drawer's
/// `media_type` hint the chain must run anyway and write the official rows
/// under the resolved key; only the counter/schedule stay deferred.
#[tokio::test]
async fn drawer_preview_without_media_items_row_still_writes_official_rows() {
    let ctx = common::TestContext::new().await;
    bind(&ctx, SOURCE, SOURCE_ID).await;

    let mut m = meta(Some("Official"), None, Some("매주 금요일"));
    m.count = Some(281);
    m.chapters = vec![main_chapter(100), bonus_chapter(0)];
    let fetcher = FixtureMetaFetcher::new(m);

    let written = enrich_official_meta_with(&ctx.pool, PROVIDER, EXTERNAL_ID, "manhua", &fetcher)
        .await
        .expect("chain must run without a media_items row");
    assert!(written > 0, "official rows must be written");

    assert_eq!(
        fetcher.seen(),
        vec![(SOURCE.to_string(), SOURCE_ID.to_string())],
        "the chain must fetch the bound source id"
    );

    let rows = chapter_rows(&ctx).await;
    assert_eq!(rows.len(), 2, "one main row + one bonus row");
    assert!(
        rows.iter()
            .all(|(p, e, _, _, _)| p == SOURCE && e == SOURCE_ID),
        "every row must land under the resolved official key, got {rows:?}"
    );

    let items: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_items")
        .fetch_one(&ctx.pool)
        .await
        .expect("count media_items");
    assert_eq!(items, 0, "the preview must not create a media_items row");
}

/// Old behaviour kept: no media_type hint and no row -> no-op, zero network.
#[tokio::test]
async fn for_key_without_row_and_hint_stays_a_no_op() {
    let ctx = common::TestContext::new().await;
    bind(&ctx, SOURCE, SOURCE_ID).await;

    let written = enrich_official_meta_for_key(&ctx.pool, PROVIDER, EXTERNAL_ID, None)
        .await
        .expect("a missing media_items row is a normal state");
    assert_eq!(written, 0, "without a hint a missing row must stay a no-op");
    assert!(
        chapter_rows(&ctx).await.is_empty(),
        "a no-op must not write chapter rows"
    );
}

/// The hint alone is not enough: with no meta-capable binding the chain finds
/// nothing and stays a no-op (zero network).
#[tokio::test]
async fn for_key_with_hint_and_without_binding_is_a_no_op() {
    let ctx = common::TestContext::new().await;

    let written = enrich_official_meta_for_key(&ctx.pool, PROVIDER, EXTERNAL_ID, Some("manhua"))
        .await
        .expect("a missing binding is a normal state");
    assert_eq!(written, 0);
}
