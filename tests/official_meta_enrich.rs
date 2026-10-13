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
            label: None,
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
        label: None,
    }
}

fn bonus_chapter(number_x100: i32) -> ChapterMeta {
    ChapterMeta {
        number_x100,
        title: Some(format!("Bonus {number_x100}")),
        release_date: None,
        is_bonus: true,
        label: None,
    }
}

/// A bilibili split part row: main (`479-1` -> 47901) or a duplicate bonus.
fn part_chapter(number_x100: i32, is_bonus: bool) -> ChapterMeta {
    let label = format!("{}-{}", number_x100 / 100, number_x100 % 100);
    ChapterMeta {
        number_x100,
        title: Some(label.clone()),
        release_date: None,
        is_bonus,
        label: Some(label),
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

/// `(chapter_number, chapter_label)` of every main row, oldest-first.
async fn main_labels(ctx: &common::TestContext) -> Vec<(i32, Option<String>)> {
    sqlx::query_as(
        "SELECT chapter_number, chapter_label FROM series_chapters \
         WHERE NOT is_bonus ORDER BY chapter_number ASC",
    )
    .fetch_all(&ctx.pool)
    .await
    .expect("read main rows")
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

/// The same source's detailed walk supersedes its own count-provider reading:
/// `auto:bilibili` (coarse, counts bonus rows) may be LOWERED by the walk's
/// main-only count — but only by the bonus rows the walk actually found.
#[tokio::test]
async fn same_source_meta_replaces_its_count_provider_reading() {
    let ctx = setup().await;
    sqlx::query(
        "UPDATE media_items SET chapters = 9, chapters_source = 'auto:bilibili' \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed count-provider reading");

    let mut m = meta(None, None, None);
    m.count = Some(7);
    // 7 main chapters + 2 bonus rows: the coarse 9 = 7 mains + 2 bonuses, so
    // the delta the walk justifies is exactly 2.
    m.chapters = (1..=7)
        .map(|n| main_chapter(n * 100))
        .chain(std::iter::repeat_with(|| bonus_chapter(0)).take(2))
        .collect();
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(7), Some("official_bilibili".to_string())),
        "the walk's main-only count may lower its own provider's reading"
    );
}

/// The meta's own earlier reading may be lowered too: when a row is
/// reclassified from main to bonus (a newer afterword marker), the stored
/// `official_<source>` counter sits one bonus above the walk's main count.
#[tokio::test]
async fn official_meta_lowers_its_own_previous_reading() {
    let ctx = setup().await;
    sqlx::query(
        "UPDATE media_items SET chapters = 2, chapters_source = 'official_bilibili' \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed official reading");

    let mut m = meta(None, None, None);
    m.count = Some(1);
    // One main + one bonus: the earlier pass counted the afterword as a main,
    // so the walk's main-only count is one lower (bounded by that bonus row).
    m.chapters = vec![main_chapter(100), bonus_chapter(0)];
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(1), Some("official_bilibili".to_string())),
        "a reclassified row may lower the official's own reading"
    );
}

/// A truncated walk must NOT lower the counter: the gap between the coarse
/// reading and the walk count may not exceed the bonus rows the walk found.
#[tokio::test]
async fn truncated_walk_does_not_lower_the_counter() {
    let ctx = setup().await;
    sqlx::query(
        "UPDATE media_items SET chapters = 9, chapters_source = 'auto:bilibili' \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed count-provider reading");

    let mut m = meta(None, None, None);
    m.count = Some(7);
    // No bonus rows: a 9 -> 7 gap would be unjustified, so the walk is treated
    // as truncated and the coarse reading stays.
    m.chapters = (1..=7).map(|n| main_chapter(n * 100)).collect();
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(9), Some("auto:bilibili".to_string())),
        "a truncated walk leaves the coarse reading alone"
    );
}

/// An equal count still promotes the provenance (`auto:<source>` ->
/// `official_<source>`), which is what disables the provider chain for good.
#[tokio::test]
async fn equal_count_promotes_provenance() {
    let ctx = setup().await;
    sqlx::query(
        "UPDATE media_items SET chapters = 9, chapters_source = 'auto:bilibili' \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed count-provider reading");

    let mut m = meta(None, None, None);
    m.count = Some(9);
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(9), Some("official_bilibili".to_string())),
        "the same value is still rewritten with the official provenance"
    );
}

/// A hand-set counter blocks the supersede path too (`chapters_manual`).
#[tokio::test]
async fn manual_counter_blocks_the_supersede() {
    let ctx = setup().await;
    sqlx::query(
        "UPDATE media_items SET chapters = 9, chapters_manual = TRUE, \
             chapters_source = 'auto:bilibili' \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed manual count-provider reading");

    let mut m = meta(None, None, None);
    m.count = Some(7);
    m.chapters = (1..=7)
        .map(|n| main_chapter(n * 100))
        .chain(std::iter::repeat_with(|| bonus_chapter(0)).take(2))
        .collect();
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(9), Some("auto:bilibili".to_string())),
        "a manual counter survives even a justified supersede"
    );
}

/// After the supersede, a larger walk still raises through the normal path.
#[tokio::test]
async fn walk_growth_raises_after_supersede() {
    let ctx = setup().await;
    sqlx::query(
        "UPDATE media_items SET chapters = 7, chapters_source = 'official_bilibili' \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed official reading");

    let mut m = meta(None, None, None);
    m.count = Some(9);
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(9), Some("official_bilibili".to_string())),
        "a grown walk raises normally once the official source owns the counter"
    );
}

/// A foreign source's counter stays raise-only: only `auto:<source>` of the
/// SAME source may be superseded by the walk.
#[tokio::test]
async fn foreign_source_reading_stays_raise_only() {
    let ctx = setup().await;
    sqlx::query(
        "UPDATE media_items SET chapters = 9, chapters_source = 'auto:naver' \
         WHERE provider = $1 AND external_id = $2",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .execute(&ctx.pool)
    .await
    .expect("seed foreign count-provider reading");

    let mut m = meta(None, None, None);
    m.count = Some(7);
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    assert_eq!(
        counter_columns(&ctx).await,
        (Some(9), Some("auto:naver".to_string())),
        "another source's lower reading must not lower the counter"
    );
}

/// Drawer preview: no `media_items` row yet -> chapters are still stored under
/// the source key, only the counter/schedule are deferred.
#[tokio::test]
async fn chapters_are_stored_without_a_media_items_row() {
    let ctx = common::TestContext::new().await;
    bind(&ctx, SOURCE, SOURCE_ID).await;

    let m = meta(Some("Preview"), None, Some("매주 금요일"));
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

/// A re-apply also drops THIS source's own stale main rows: a row the source no
/// longer reports (a reclassified afterword) must not linger next to the bonus
/// that replaced it. Rows still in the answer, read rows, rows carrying read
/// progress and other sources' rows survive.
#[tokio::test]
async fn orphan_cleanup_removes_the_sources_own_stale_main() {
    let ctx = setup().await;
    let user_id = seed_user(&ctx).await;

    // The walk reports 100 (seeded by `setup`) and 200.
    let mut m = meta(None, None, None);
    m.chapters.push(main_chapter(200));
    m.count = Some(2);

    // Still in the answer -> kept even though it is this source's row.
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 200, SOURCE, false, false).await;
    // Not in the answer -> the source's own unread main row is removed.
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 22300, SOURCE, false, false).await;
    // ... but read state is never sacrificed.
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 22400, SOURCE, true, false).await;
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 22500, SOURCE, false, false).await;
    seed_read_progress(&ctx, user_id, PROVIDER, EXTERNAL_ID, 22500).await;
    // Another source's row is never touched.
    seed_chapter(&ctx, PROVIDER, EXTERNAL_ID, 22600, "kakao", false, false).await;

    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    let numbers: Vec<i32> = chapter_rows(&ctx)
        .await
        .into_iter()
        .map(|(_, _, n, _, _)| n)
        .collect();
    assert_eq!(
        numbers,
        vec![100, 200, 22400, 22500, 22600],
        "only the source's own unread stale main (22300) may be removed"
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

/// Bonus rows have no conflict target (the partial unique index covers main
/// rows only), so a re-apply must rewrite this source's bonuses instead of
/// duplicating them — and a manual bonus must survive. Each source bonus gets
/// its own negative counter (migration 029); the manual one keeps its number.
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
        vec![(-100, "bilibili".to_string()), (0, "manual".to_string()),],
        "the source bonus gets a negative counter, the manual bonus is kept next to it"
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
        vec![(-100, SOURCE.to_string())],
        "an answer without chapters must not wipe already-stored bonuses"
    );
}

/// Every bonus row gets its own negative counter (oldest-first), so a single
/// bonus can be marked read on its own; main rows keep their positive number.
#[tokio::test]
async fn bonus_rows_get_distinct_negative_numbers() {
    let ctx = setup().await;

    let mut m = meta(None, None, None);
    m.chapters = vec![
        main_chapter(100),
        bonus_chapter(0),
        bonus_chapter(0),
        main_chapter(200),
    ];
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    let mut bonuses: Vec<i32> = chapter_rows(&ctx)
        .await
        .into_iter()
        .filter(|(_, _, _, _, is_bonus)| *is_bonus)
        .map(|(_, _, n, _, _)| n)
        .collect();
    bonuses.sort();
    assert_eq!(
        bonuses,
        vec![-200, -100],
        "two bonus rows must get two distinct negative counters"
    );
}

/// A numbered bonus keeps its original number as the display label; unnumbered
/// specials get a running counter. The stored key stays a hidden negative.
#[tokio::test]
async fn bonus_labels_use_original_number_or_counter() {
    let ctx = setup().await;

    let mut m = meta(None, None, None);
    m.chapters = vec![
        main_chapter(100),
        bonus_chapter(4950), // "49.5"
        bonus_chapter(0),    // unnumbered special
        bonus_chapter(0),    // unnumbered special
    ];
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    let rows: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT chapter_number, chapter_label FROM series_chapters \
         WHERE is_bonus ORDER BY chapter_number ASC",
    )
    .fetch_all(&ctx.pool)
    .await
    .expect("read bonus rows");

    assert_eq!(
        rows,
        vec![
            (-300, Some("49.5".to_string())),
            (-200, Some("1".to_string())),
            (-100, Some("2".to_string())),
        ],
        "numbered bonus keeps its number, unnumbered get 1,2,3..."
    );
}

/// A bonus that duplicates a main number keeps that number as its label.
#[tokio::test]
async fn duplicate_number_bonus_keeps_its_number_as_label() {
    let ctx = setup().await;

    let mut m = meta(None, None, None);
    m.chapters = vec![main_chapter(200), bonus_chapter(200), main_chapter(300)];
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    let rows: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT chapter_number, chapter_label FROM series_chapters \
         WHERE is_bonus ORDER BY chapter_number ASC",
    )
    .fetch_all(&ctx.pool)
    .await
    .expect("read bonus rows");

    assert_eq!(
        rows,
        vec![(-100, Some("2".to_string()))],
        "the duplicate keeps its original number as the display label"
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

/// A split part (`479-1` -> 47901) is a main row with an `N-p` label; a
/// re-apply keeps it (COALESCE) instead of blanking it. Whole chapters and
/// sources without split parts stay label-free.
#[tokio::test]
async fn bilibili_part_main_rows_get_and_keep_their_label() {
    let ctx = setup().await;

    let mut m = meta(None, None, None);
    m.chapters = vec![
        main_chapter(47900),
        part_chapter(47901, false),
        main_chapter(48000),
    ];
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    let rows = main_labels(&ctx).await;
    assert_eq!(
        rows,
        vec![
            (47900, None),
            (47901, Some("479-1".to_string())),
            (48000, None),
        ],
        "only the part row carries a label (the unread MU skeleton 100 was cleaned up)"
    );

    // Re-apply: the label must survive (ON CONFLICT keeps the existing one).
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("re-apply");
    assert_eq!(
        main_labels(&ctx).await,
        rows,
        "a re-apply reproduces the same labels"
    );
}

/// A duplicate split part becomes a bonus row but keeps its `N-p` label
/// (regression: it used to fall back to the x100 artefact `479.01`).
#[tokio::test]
async fn duplicate_part_keeps_its_part_label() {
    let ctx = setup().await;

    let mut m = meta(None, None, None);
    m.chapters = vec![part_chapter(47901, false), part_chapter(47901, true)];
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, SOURCE, &m)
        .await
        .expect("apply_meta");

    let bonus: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT chapter_number, chapter_label FROM series_chapters \
         WHERE is_bonus ORDER BY chapter_number ASC",
    )
    .fetch_all(&ctx.pool)
    .await
    .expect("read bonus rows");

    assert_eq!(
        bonus,
        vec![(-100, Some("479-1".to_string()))],
        "the duplicate part keeps its `N-p` label, not `479.01`"
    );
}

/// Sources without split parts keep main rows label-free: a fractional main
/// row (a "(part 2)" upload) must not be relabeled as `N-p`.
#[tokio::test]
async fn fractional_main_rows_stay_label_free() {
    let ctx = setup().await;

    let mut m = meta(None, None, None);
    m.chapters = vec![main_chapter(1050)]; // chapter 10.5
    apply_meta(&ctx.pool, PROVIDER, EXTERNAL_ID, "mangaupdates", &m)
        .await
        .expect("apply_meta");

    let label: Option<String> =
        sqlx::query_scalar("SELECT chapter_label FROM series_chapters WHERE chapter_number = 1050")
            .fetch_one(&ctx.pool)
            .await
            .expect("read label");
    assert_eq!(label, None, "only split parts get an `N-p` label");
}
