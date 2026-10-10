//! Batch 6: every chapter reader/writer in `services::chapters` resolves the
//! incoming MangaUpdates key through `official_resolve::chapter_key` before
//! touching `series_chapters` / `user_chapter_progress` (design :73-78, :102-103).
//!
//! Invariant under test: with a meta-capable binding, rows live under the
//! official key and the MU-key callers still find them (and write progress
//! there). Without a binding, everything stays under the MU key exactly as
//! before.

mod common;

use std::sync::{Arc, Mutex};

use mediatracker::routes::media::chapter_counts;
use mediatracker::services::chapter_meta_enrich::merge_mu_duplicates;
use mediatracker::services::chapters::{
    ChapterTitleGap, count_read, get_chapter, get_chapter_states, get_chapters,
    list_titles_needing_chapter_titles, set_manual_chapters, set_read, store_chapters_mu,
};
use uuid::Uuid;

const FIXTURE_USERNAME: &str = "test_chapter_resolution_user";
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

    (ctx, user_id)
}

async fn add_media_item(pool: &sqlx::PgPool, mu_id: &str, media_type: &str) {
    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title) \
         VALUES ('mangaupdates', $1, $2, $3)",
    )
    .bind(mu_id)
    .bind(media_type)
    .bind(format!("Resolution fixture {mu_id}"))
    .execute(pool)
    .await
    .expect("insert media_items row");
}

async fn bind(pool: &sqlx::PgPool, mu_id: &str, source: &str, source_id: &str) {
    sqlx::query(
        "INSERT INTO media_source_ids (provider, external_id, source, source_id) \
         VALUES ('mangaupdates', $1, $2, $3)",
    )
    .bind(mu_id)
    .bind(source)
    .bind(source_id)
    .execute(pool)
    .await
    .expect("insert binding");
}

/// Chapter row under an explicit key. `origin` defaults to 'mu' so the
/// `store_chapters_mu` guard is only triggered where a test asks for it.
async fn insert_chapter(
    pool: &sqlx::PgPool,
    provider: &str,
    external_id: &str,
    chapter_number: i32,
    origin: &str,
    is_bonus: bool,
) {
    sqlx::query(
        "INSERT INTO series_chapters \
             (provider, external_id, chapter_number, origin, is_bonus) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(provider)
    .bind(external_id)
    .bind(chapter_number)
    .bind(origin)
    .bind(is_bonus)
    .execute(pool)
    .await
    .expect("insert series_chapters row");
}

async fn insert_progress(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    provider: &str,
    external_id: &str,
    chapter_number: i32,
    read: bool,
) {
    sqlx::query(
        "INSERT INTO user_chapter_progress \
             (user_id, provider, external_id, chapter_number, read, read_at) \
         VALUES ($1, $2, $3, $4, $5, NOW())",
    )
    .bind(user_id)
    .bind(provider)
    .bind(external_id)
    .bind(chapter_number)
    .bind(read)
    .execute(pool)
    .await
    .expect("insert user_chapter_progress row");
}

/// All progress rows of a user for one key, as `(chapter_number, read)`.
async fn progress_rows(
    pool: &sqlx::PgPool,
    user_id: Uuid,
    provider: &str,
    external_id: &str,
) -> Vec<(i32, bool)> {
    let rows: Vec<(i32, bool)> = sqlx::query_as(
        "SELECT chapter_number, read FROM user_chapter_progress \
         WHERE user_id = $1 AND provider = $2 AND external_id = $3 \
         ORDER BY chapter_number ASC",
    )
    .bind(user_id)
    .bind(provider)
    .bind(external_id)
    .fetch_all(pool)
    .await
    .expect("read progress rows");
    rows
}

async fn chapter_row_count(pool: &sqlx::PgPool, provider: &str, external_id: &str) -> i64 {
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM series_chapters WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_one(pool)
    .await
    .expect("count chapter rows");
    count
}

// --- a) with a binding: MU-key callers see and write the official rows ------

#[tokio::test]
async fn binding_makes_mu_key_calls_hit_official_rows() {
    let (ctx, user_id) = setup().await;
    let mu_id = "910000001";
    add_media_item(&ctx.pool, mu_id, "manhua").await;
    bind(&ctx.pool, mu_id, "bilibili", "mc-res-1").await;

    for n in 1..=5 {
        insert_chapter(
            &ctx.pool,
            "bilibili",
            "mc-res-1",
            n * 100,
            "official_bilibili",
            false,
        )
        .await;
    }
    // Progress that predates this batch's writes, already on the official key.
    insert_progress(&ctx.pool, user_id, "bilibili", "mc-res-1", 100, true).await;

    // Readers called with the MU key must see the official rows.
    let chapters = get_chapters(&ctx.pool, "mangaupdates", mu_id, user_id)
        .await
        .expect("get chapters");
    assert_eq!(
        chapters.len(),
        5,
        "official rows must be visible from MU key"
    );
    assert!(
        chapters[0].read,
        "seeded progress must survive the key transition"
    );
    assert!(
        chapters[1..].iter().all(|c| !c.read),
        "untouched chapters must stay unread"
    );

    let states = get_chapter_states(&ctx.pool, user_id, "mangaupdates", mu_id)
        .await
        .expect("get chapter states");
    assert_eq!(states.len(), 5);
    assert_eq!(states[0], (100, true));
    assert_eq!(states[1], (200, false));

    // Writer called with the MU key must fill progress on the official key.
    let updated = set_read(&ctx.pool, user_id, "mangaupdates", mu_id, 300, true)
        .await
        .expect("mark ch 3 read");
    assert!(updated);
    assert_eq!(
        progress_rows(&ctx.pool, user_id, "bilibili", "mc-res-1").await,
        vec![(100, true), (200, true), (300, true)],
        "progress rows must land under the resolved key"
    );
    assert!(
        progress_rows(&ctx.pool, user_id, "mangaupdates", mu_id)
            .await
            .is_empty(),
        "nothing may be written under the unresolved MU key"
    );

    let n = count_read(&ctx.pool, user_id, "mangaupdates", mu_id)
        .await
        .expect("count read");
    assert_eq!(n, 3, "count_read must resolve too");

    let ch = get_chapter(&ctx.pool, "mangaupdates", mu_id, 300, user_id)
        .await
        .expect("get ch 3")
        .expect("ch 3 must exist under the official key");
    assert!(ch.read);
    assert!(!ch.is_bonus);

    // Un-checking cascades on the same key.
    set_read(&ctx.pool, user_id, "mangaupdates", mu_id, 300, false)
        .await
        .expect("mark ch 3 unread");
    assert_eq!(
        progress_rows(&ctx.pool, user_id, "bilibili", "mc-res-1").await,
        vec![(100, true), (200, true), (300, false)],
    );
    assert_eq!(
        count_read(&ctx.pool, user_id, "mangaupdates", mu_id)
            .await
            .expect("count after uncheck"),
        2
    );
}

#[tokio::test]
async fn manual_skeleton_lands_under_the_resolved_key() {
    let (ctx, user_id) = setup().await;
    let mu_id = "910000002";
    add_media_item(&ctx.pool, mu_id, "manhua").await;
    bind(&ctx.pool, mu_id, "bilibili", "mc-res-2").await;

    set_manual_chapters(&ctx.pool, "mangaupdates", mu_id, 4)
        .await
        .expect("set manual chapters");

    assert_eq!(
        chapter_row_count(&ctx.pool, "bilibili", "mc-res-2").await,
        4,
        "skeleton must join the resolved rows"
    );
    assert_eq!(
        chapter_row_count(&ctx.pool, "mangaupdates", mu_id).await,
        0,
        "nothing under the MU key"
    );

    // The manual value itself still lives on media_items (MU identity), and the
    // chapters_manual flag is read through the same MU key by set_read.
    let chapters_manual: bool = sqlx::query_scalar(
        "SELECT COALESCE(chapters_manual, FALSE) FROM media_items \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(mu_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("read media_items flag");
    assert!(chapters_manual);

    set_read(&ctx.pool, user_id, "mangaupdates", mu_id, 400, true)
        .await
        .expect("mark ch 4");
    assert_eq!(
        progress_rows(&ctx.pool, user_id, "bilibili", "mc-res-2").await,
        vec![(100, true), (200, true), (300, true), (400, true)]
    );

    // Follow-up B: chapter 5 is NOT in the skeleton (manual count is 4), yet a
    // manual `set_read` must still record it. The bulk fill covers 1..=4 via the
    // resolved key, and the direct insert of chapter 500 only happens if the
    // `chapters_manual` lookup goes through the incoming MU key.
    set_read(&ctx.pool, user_id, "mangaupdates", mu_id, 500, true)
        .await
        .expect("mark ch 5 past the skeleton");
    assert_eq!(
        progress_rows(&ctx.pool, user_id, "bilibili", "mc-res-2").await,
        vec![
            (100, true),
            (200, true),
            (300, true),
            (400, true),
            (500, true)
        ],
        "chapter 5 must be recorded even though the skeleton stops at 4"
    );
    assert_eq!(
        progress_rows(&ctx.pool, user_id, "mangaupdates", mu_id).await,
        Vec::new(),
        "progress rows must never land under the MU key"
    );
}

// --- b) without a binding: unchanged behaviour ------------------------------

#[tokio::test]
async fn without_binding_everything_stays_under_the_mu_key() {
    let (ctx, user_id) = setup().await;
    let mu_id = "910000003";
    add_media_item(&ctx.pool, mu_id, "manhua").await;

    for n in 1..=3 {
        insert_chapter(&ctx.pool, "mangaupdates", mu_id, n * 100, "mu", false).await;
    }
    insert_progress(&ctx.pool, user_id, "mangaupdates", mu_id, 100, true).await;

    let chapters = get_chapters(&ctx.pool, "mangaupdates", mu_id, user_id)
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 3);
    assert!(chapters[0].read);

    set_read(&ctx.pool, user_id, "mangaupdates", mu_id, 200, true)
        .await
        .expect("mark ch 2");
    assert_eq!(
        progress_rows(&ctx.pool, user_id, "mangaupdates", mu_id).await,
        vec![(100, true), (200, true)],
    );
    assert_eq!(
        count_read(&ctx.pool, user_id, "mangaupdates", mu_id)
            .await
            .expect("count"),
        2
    );
    assert_eq!(
        get_chapter_states(&ctx.pool, user_id, "mangaupdates", mu_id)
            .await
            .expect("states"),
        vec![(100, true), (200, true), (300, false)]
    );

    set_manual_chapters(&ctx.pool, "mangaupdates", mu_id, 5)
        .await
        .expect("manual chapters");
    assert_eq!(chapter_row_count(&ctx.pool, "mangaupdates", mu_id).await, 5);

    let inserted = store_chapters_mu(&ctx.pool, 910000004, 3)
        .await
        .expect("store skeleton without binding");
    assert_eq!(inserted, 3, "unbound series keeps writing under the MU key");
    assert_eq!(
        chapter_row_count(&ctx.pool, "mangaupdates", "910000004").await,
        3
    );
}

// --- c) store_chapters_mu yields to official rows ---------------------------

#[tokio::test]
async fn store_chapters_mu_skips_when_official_rows_exist() {
    let (ctx, _user_id) = setup().await;
    let series_id: i64 = 910000005;
    let mu_id = series_id.to_string();
    add_media_item(&ctx.pool, &mu_id, "manhua").await;
    bind(&ctx.pool, &mu_id, "bilibili", "mc-res-5").await;

    // The async official enrichment got there first.
    insert_chapter(
        &ctx.pool,
        "bilibili",
        "mc-res-5",
        100,
        "official_bilibili",
        false,
    )
    .await;

    let inserted = store_chapters_mu(&ctx.pool, series_id, 5)
        .await
        .expect("store skeleton");
    assert_eq!(inserted, 0, "MU skeleton must yield to official rows");
    assert_eq!(
        chapter_row_count(&ctx.pool, "bilibili", "mc-res-5").await,
        1,
        "official rows must be untouched"
    );
    assert_eq!(
        chapter_row_count(&ctx.pool, "mangaupdates", &mu_id).await,
        0,
        "no skeleton may appear under the MU key"
    );
}

#[tokio::test]
async fn store_chapters_mu_fills_resolved_key_without_official_rows() {
    let (ctx, user_id) = setup().await;
    let series_id: i64 = 910000006;
    let mu_id = series_id.to_string();
    add_media_item(&ctx.pool, &mu_id, "manhua").await;
    bind(&ctx.pool, &mu_id, "bilibili", "mc-res-6").await;

    let inserted = store_chapters_mu(&ctx.pool, series_id, 3)
        .await
        .expect("store skeleton");
    assert_eq!(
        inserted, 3,
        "fallback skeleton is written under the official key"
    );
    assert_eq!(
        chapter_row_count(&ctx.pool, "bilibili", "mc-res-6").await,
        3
    );
    assert_eq!(
        chapter_row_count(&ctx.pool, "mangaupdates", &mu_id).await,
        0
    );

    // The media_items counter is still synced through the MU identity, so the
    // reader sees the same rows the denominator promises.
    let chapters: Option<i32> = sqlx::query_scalar(
        "SELECT chapters FROM media_items WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(&mu_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("read media_items.chapters");
    assert_eq!(chapters, Some(3));

    assert_eq!(
        get_chapters(&ctx.pool, "mangaupdates", &mu_id, user_id)
            .await
            .expect("get chapters")
            .len(),
        3
    );
}

// --- d) count_read ignores bonus chapters ----------------------------------

#[tokio::test]
async fn count_read_ignores_bonus_chapters() {
    let (ctx, user_id) = setup().await;
    let mu_id = "910000007";
    add_media_item(&ctx.pool, mu_id, "manhua").await;

    insert_chapter(&ctx.pool, "mangaupdates", mu_id, 500, "mu", false).await;
    insert_chapter(
        &ctx.pool,
        "mangaupdates",
        mu_id,
        9900,
        "official_bilibili",
        true,
    )
    .await;
    insert_progress(&ctx.pool, user_id, "mangaupdates", mu_id, 500, true).await;
    insert_progress(&ctx.pool, user_id, "mangaupdates", mu_id, 9900, true).await;

    let n = count_read(&ctx.pool, user_id, "mangaupdates", mu_id)
        .await
        .expect("count read");
    assert_eq!(n, 5, "bonus chapter 99 must not inflate read progress");

    // The bonus row is still listed for the UI (batch 7 renders it separately).
    let chapters = get_chapters(&ctx.pool, "mangaupdates", mu_id, user_id)
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 2);
    assert!(!chapters[0].is_bonus);
    assert!(chapters[1].is_bonus);
}

// --- e) follow-up A: the raise skeleton must land under the resolved key -----

/// Injectable count provider: reports a fixed total and records the id it was
/// asked about (same mock shape as `tests/source_id_mapping.rs`).
struct FixedCountProvider {
    name: &'static str,
    total: i32,
    seen: Arc<Mutex<Vec<String>>>,
}

impl mediatracker::services::external::chapter_count::ChapterCountProvider for FixedCountProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    fn fetch<'a>(
        &'a self,
        external_id: &'a str,
    ) -> mediatracker::services::external::chapter_count::ChapterCountFuture<'a> {
        let seen = self.seen.clone();
        let id = external_id.to_string();
        let total = self.total;
        Box::pin(async move {
            seen.lock().unwrap().push(id);
            Ok(Some(
                mediatracker::services::external::chapter_count::ChapterCount {
                    total,
                    source: "auto:bilibili",
                },
            ))
        })
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["manhua"]
    }
}

#[tokio::test]
async fn enrich_skeleton_lands_under_the_resolved_key() {
    let (ctx, _user_id) = setup().await;
    let mu_id = "910000008";
    add_media_item(&ctx.pool, mu_id, "manhua").await;
    bind(&ctx.pool, mu_id, "bilibili", "mc-enrich-1").await;

    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let provider: Arc<dyn mediatracker::services::external::chapter_count::ChapterCountProvider> =
        Arc::new(FixedCountProvider {
            name: "bilibili",
            total: 5,
            seen: seen.clone(),
        });

    let applied = mediatracker::services::chapter_enrich::enrich_with_providers(
        &ctx.pool,
        "mangaupdates",
        mu_id,
        vec![provider],
    )
    .await;
    assert_eq!(applied, Some(5), "raise must apply");

    // The provider must be queried with its OWN id, never the MU skeleton id.
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["mc-enrich-1".to_string()],
        "provider must be called with the bound id"
    );

    assert_eq!(
        chapter_row_count(&ctx.pool, "bilibili", "mc-enrich-1").await,
        5,
        "skeleton must land under the resolved key so readers can see it"
    );
    assert_eq!(
        chapter_row_count(&ctx.pool, "mangaupdates", mu_id).await,
        0,
        "no origin='mu' rows may survive under the MU key"
    );

    // The count itself is still raised on the MU-identity media_items row.
    let (chapters, manual, source): (Option<i32>, bool, Option<String>) = sqlx::query_as(
        "SELECT chapters, chapters_manual, chapters_source FROM media_items \
         WHERE provider = 'mangaupdates' AND external_id = $1",
    )
    .bind(mu_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("read media_items count state");
    assert_eq!(chapters, Some(5));
    assert!(!manual);
    assert_eq!(source.as_deref(), Some("auto:bilibili"));
}

// --- g) drawer counts read the resolved key ---------------------------------

/// Drawer badge counts: one query, main/bonus split, under the resolved key.
#[tokio::test]
async fn drawer_counts_split_main_and_bonus_under_the_official_key() {
    let (ctx, _user_id) = setup().await;
    let mu_id = "910000040";
    add_media_item(&ctx.pool, mu_id, "manhua").await;
    bind(&ctx.pool, mu_id, "bilibili", "mc-counts-1").await;

    insert_chapter(
        &ctx.pool,
        "bilibili",
        "mc-counts-1",
        100,
        "official_bilibili",
        false,
    )
    .await;
    insert_chapter(
        &ctx.pool,
        "bilibili",
        "mc-counts-1",
        200,
        "official_bilibili",
        false,
    )
    .await;
    insert_chapter(
        &ctx.pool,
        "bilibili",
        "mc-counts-1",
        0,
        "official_bilibili",
        true,
    )
    .await;

    assert_eq!(
        chapter_counts(&ctx.pool, "mangaupdates", mu_id, "manhua").await,
        (2, 1),
        "main and bonus rows under the official key must be split"
    );

    // Non-chapter media types never touch the table.
    assert_eq!(
        chapter_counts(&ctx.pool, "mangaupdates", mu_id, "movie").await,
        (0, 0),
        "a movie drawer has no chapter counts"
    );

    // An unknown key degrades to zero instead of erroring.
    assert_eq!(
        chapter_counts(&ctx.pool, "mangaupdates", "910000041", "manhua").await,
        (0, 0),
        "an unknown key must degrade to zero"
    );
}

// --- f) bulk-enrichment / backfill selections see the resolved rows ---------

/// `media_items` row with an explicit `created_at` so the oldest-first order of
/// the shared selection is deterministic.
async fn add_media_item_at(
    pool: &sqlx::PgPool,
    provider: &str,
    external_id: &str,
    media_type: &str,
    age_minutes: i32,
) {
    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title, created_at) \
         VALUES ($1, $2, $3, $4, NOW() - make_interval(mins => $5))",
    )
    .bind(provider)
    .bind(external_id)
    .bind(media_type)
    .bind(format!("Selection fixture {provider}/{external_id}"))
    .bind(age_minutes)
    .execute(pool)
    .await
    .expect("insert media_items row");
}

/// Chapter row carrying an English title (the column the selection probes).
async fn insert_titled_chapter(
    pool: &sqlx::PgPool,
    provider: &str,
    external_id: &str,
    chapter_number: i32,
    title: &str,
) {
    sqlx::query(
        "INSERT INTO series_chapters \
             (provider, external_id, chapter_number, title_en, origin) \
         VALUES ($1, $2, $3, $4, 'official_bilibili')",
    )
    .bind(provider)
    .bind(external_id)
    .bind(chapter_number)
    .bind(title)
    .execute(pool)
    .await
    .expect("insert titled series_chapters row");
}

/// External ids of the selection, sorted — order-independent assertions.
fn selected_ids(rows: Vec<(String, String, String)>) -> Vec<String> {
    let mut ids: Vec<String> = rows.into_iter().map(|(_, id, _)| id).collect();
    ids.sort();
    ids
}

/// The admin bulk pass must judge a bound title by its OFFICIAL rows: with the
/// old `sc.provider = mi.provider` predicate a titled bound title looks like a
/// title with no chapters at all and is re-enriched (extra network pass).
#[tokio::test]
async fn admin_selection_sees_titles_under_the_resolved_key() {
    let (ctx, _user_id) = setup().await;

    // Titled, but the rows live under the official key: already enriched.
    let titled_bound = "910000020";
    add_media_item(&ctx.pool, titled_bound, "manhua").await;
    bind(&ctx.pool, titled_bound, "bilibili", "mc-sel-titled").await;
    for n in 1..=3 {
        insert_titled_chapter(
            &ctx.pool,
            "bilibili",
            "mc-sel-titled",
            n * 100,
            &format!("Chapter {n}"),
        )
        .await;
    }

    // Untitled rows under the official key: the one that needs the pass.
    let untitled_bound = "910000021";
    add_media_item(&ctx.pool, untitled_bound, "manhua").await;
    bind(&ctx.pool, untitled_bound, "bilibili", "mc-sel-untitled").await;
    for n in 1..=3 {
        insert_chapter(
            &ctx.pool,
            "bilibili",
            "mc-sel-untitled",
            n * 100,
            "official_bilibili",
            false,
        )
        .await;
    }

    // No binding at all: unchanged behaviour, rows stay under the MU key.
    let unbound = "910000022";
    add_media_item(&ctx.pool, unbound, "manhua").await;
    for n in 1..=3 {
        insert_chapter(&ctx.pool, "mangaupdates", unbound, n * 100, "mu", false).await;
    }

    // Non-MU provider: `chapter_key` returns it unchanged, so its own key holds
    // the rows. Selected when no provider filter is given (admin), skipped by
    // the backfill (which filters `--provider`).
    add_media_item_at(&ctx.pool, "mangadex", "md-sel-1", "manga", 1).await;
    for n in 1..=2 {
        insert_chapter(&ctx.pool, "mangadex", "md-sel-1", n * 100, "mu", false).await;
    }

    let rows =
        list_titles_needing_chapter_titles(&ctx.pool, None, ChapterTitleGap::NoTitledRows, None)
            .await
            .expect("admin selection");
    assert_eq!(
        selected_ids(rows),
        vec![
            untitled_bound.to_string(),
            unbound.to_string(),
            "md-sel-1".to_string()
        ],
        "a fully titled bound title must not look like a title without chapters"
    );

    let rows = list_titles_needing_chapter_titles(
        &ctx.pool,
        Some("mangaupdates"),
        ChapterTitleGap::NoTitledRows,
        None,
    )
    .await
    .expect("admin selection filtered by provider");
    assert_eq!(
        selected_ids(rows),
        vec![untitled_bound.to_string(), unbound.to_string()],
        "the provider filter must exclude other providers"
    );
}

/// The MangaDex backfill must NOT skip bound titles: their rows sit under the
/// official key, so the old EXISTS predicate found nothing and left their
/// chapter titles unfilled forever. `limit` also applies after the predicate.
#[tokio::test]
async fn backfill_selection_finds_untitled_rows_under_the_resolved_key() {
    let (ctx, _user_id) = setup().await;

    // Oldest first, and both need the backfill.
    let older = "910000030";
    add_media_item_at(&ctx.pool, "mangaupdates", older, "manhua", 30).await;
    bind(&ctx.pool, older, "bilibili", "mc-sel-older").await;
    for n in 1..=3 {
        insert_chapter(
            &ctx.pool,
            "bilibili",
            "mc-sel-older",
            n * 100,
            "official_bilibili",
            false,
        )
        .await;
    }

    // Partially titled: 2 rows named, one bare -> still needs the pass.
    let mixed = "910000031";
    add_media_item_at(&ctx.pool, "mangaupdates", mixed, "manhua", 20).await;
    bind(&ctx.pool, mixed, "bilibili", "mc-sel-mixed").await;
    insert_chapter(
        &ctx.pool,
        "bilibili",
        "mc-sel-mixed",
        100,
        "official_bilibili",
        false,
    )
    .await;
    insert_titled_chapter(&ctx.pool, "bilibili", "mc-sel-mixed", 200, "Chapter 2").await;
    insert_chapter(
        &ctx.pool,
        "bilibili",
        "mc-sel-mixed",
        300,
        "official_bilibili",
        false,
    )
    .await;

    // Fully titled and the OLDEST of the three (age 40, so oldest-first order is
    // done, older 30, mixed 20): the predicate has to drop it before the limit is
    // ever consulted, otherwise it would be the first candidate examined.
    let done = "910000032";
    add_media_item_at(&ctx.pool, "mangaupdates", done, "manhua", 40).await;
    bind(&ctx.pool, done, "bilibili", "mc-sel-done").await;
    for n in 1..=3 {
        insert_titled_chapter(
            &ctx.pool,
            "bilibili",
            "mc-sel-done",
            n * 100,
            &format!("Chapter {n}"),
        )
        .await;
    }

    let rows = list_titles_needing_chapter_titles(
        &ctx.pool,
        Some("mangaupdates"),
        ChapterTitleGap::UntitledRows,
        None,
    )
    .await
    .expect("backfill selection");
    let ids = rows.into_iter().map(|(_, id, _)| id).collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![older.to_string(), mixed.to_string()],
        "bound titles with untitled official rows must be selected, oldest first"
    );

    // `limit` applies after the predicate. Had the limit been taken before it,
    // the single slot would have been spent on `done` (oldest, fully titled) and
    // `older` would have been missed; the predicate drops `done` first, so the
    // result is unchanged and still `older`.
    let rows = list_titles_needing_chapter_titles(
        &ctx.pool,
        Some("mangaupdates"),
        ChapterTitleGap::UntitledRows,
        Some(1),
    )
    .await
    .expect("limited backfill selection");
    assert_eq!(
        rows.into_iter().map(|(_, id, _)| id).collect::<Vec<_>>(),
        vec![older.to_string()],
    );
}

// --- merge: shadowed MU duplicates fold into the official rows -------------

#[tokio::test]
async fn merge_folds_read_mu_duplicates_into_official_parts() {
    let (ctx, user_id) = setup().await;
    let mu_id = "77001";
    add_media_item(&ctx.pool, mu_id, "manhua").await;
    bind(&ctx.pool, mu_id, "bilibili", "mc-merge").await;

    // Rows live under the resolved (bilibili) key: official part rows 201/202
    // (chapter 2 split in two) next to the old MU row 200 for chapter 2.
    insert_chapter(&ctx.pool, "bilibili", "mc-merge", 100, "mu", false).await;
    insert_chapter(&ctx.pool, "bilibili", "mc-merge", 200, "mu", false).await;
    insert_chapter(&ctx.pool, "bilibili", "mc-merge", 201, "bilibili", false).await;
    insert_chapter(&ctx.pool, "bilibili", "mc-merge", 202, "bilibili", false).await;

    insert_progress(&ctx.pool, user_id, "bilibili", "mc-merge", 100, true).await;
    insert_progress(&ctx.pool, user_id, "bilibili", "mc-merge", 200, true).await;

    let removed = merge_mu_duplicates(&ctx.pool, "mangaupdates", mu_id)
        .await
        .expect("merge");
    assert_eq!(removed, 1, "only the shadowed chapter-2 row folds");

    // The MU row for chapter 2 is gone; both parts now carry the read state.
    assert_eq!(
        progress_rows(&ctx.pool, user_id, "bilibili", "mc-merge").await,
        vec![(100, true), (201, true), (202, true)]
    );
    // Chapter 1's MU row stays: no official row shadows its base.
    assert_eq!(
        chapter_row_count(&ctx.pool, "bilibili", "mc-merge").await,
        3
    );

    // Idempotent: nothing left to fold.
    assert_eq!(
        merge_mu_duplicates(&ctx.pool, "mangaupdates", mu_id)
            .await
            .expect("merge again"),
        0
    );
}

#[tokio::test]
async fn merge_drops_unread_shadows_but_keeps_legacy_read_rows() {
    let (ctx, _user_id) = setup().await;
    let mu_id = "77002";
    add_media_item(&ctx.pool, mu_id, "manhua").await;
    bind(&ctx.pool, mu_id, "bilibili", "mc-merge2").await;

    // Unread shadowed MU row: no read state to carry, so it folds away.
    insert_chapter(&ctx.pool, "bilibili", "mc-merge2", 300, "mu", false).await;
    insert_chapter(&ctx.pool, "bilibili", "mc-merge2", 301, "bilibili", false).await;
    // Legacy `series_chapters.read` row without per-user progress: kept, since
    // its read state cannot be attributed to a user.
    sqlx::query(
        "INSERT INTO series_chapters \
             (provider, external_id, chapter_number, origin, read) \
         VALUES ('bilibili', 'mc-merge2', 400, 'mu', TRUE)",
    )
    .execute(&ctx.pool)
    .await
    .expect("legacy read row");
    insert_chapter(&ctx.pool, "bilibili", "mc-merge2", 401, "bilibili", false).await;

    let removed = merge_mu_duplicates(&ctx.pool, "mangaupdates", mu_id)
        .await
        .expect("merge");
    assert_eq!(
        removed, 1,
        "the unread shadow folds, the legacy-read row stays"
    );

    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT chapter_number, origin FROM series_chapters \
         WHERE provider = 'bilibili' AND external_id = 'mc-merge2' \
         ORDER BY chapter_number ASC",
    )
    .fetch_all(&ctx.pool)
    .await
    .expect("read rows");
    assert_eq!(
        rows,
        vec![
            (301, "bilibili".to_string()),
            (400, "mu".to_string()),
            (401, "bilibili".to_string())
        ]
    );
}

// --- resolve: a bound source that gained a fetch arm rekeys stranded rows ---

/// Never hits the network: the bound meta-capable binding short-circuits.
struct NoSearch;

impl mediatracker::services::official_resolve::OfficialSearch for NoSearch {
    fn search<'a>(
        &'a self,
        _source: &'a str,
        _query: &'a str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<
                        Vec<mediatracker::services::official_types::OfficialHit>,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[tokio::test]
async fn bound_meta_binding_rekeys_stranded_mu_rows() {
    let (ctx, user_id) = setup().await;
    let mu_id = "77003";
    add_media_item(&ctx.pool, mu_id, "manhwa").await;
    // Bound while kakao was still non-meta: rows and progress sit under the MU
    // identity key even though `chapter_key` now resolves to the kakao key.
    bind(&ctx.pool, mu_id, "kakao", "55989064").await;
    insert_chapter(&ctx.pool, "mangaupdates", mu_id, 100, "mu", false).await;
    insert_progress(&ctx.pool, user_id, "mangaupdates", mu_id, 100, true).await;

    let outcome = mediatracker::services::official_resolve::resolve_and_bind(
        &ctx.pool,
        &NoSearch,
        "mangaupdates",
        mu_id,
        "manhwa",
        "용사파티",
        &[],
    )
    .await;
    assert_eq!(
        outcome,
        Some(mediatracker::services::official_resolve::ResolveOutcome {
            source: "kakao".to_string(),
            source_id: "55989064".to_string(),
        })
    );
    // The stranded rows and read progress moved under the resolved key.
    assert_eq!(chapter_row_count(&ctx.pool, "kakao", "55989064").await, 1);
    assert_eq!(chapter_row_count(&ctx.pool, "mangaupdates", mu_id).await, 0);
    assert_eq!(
        progress_rows(&ctx.pool, user_id, "kakao", "55989064").await,
        vec![(100, true)]
    );
}
