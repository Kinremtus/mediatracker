//! Integration test for `migrations/028_chapter_ownership.sql`.
//!
//! The migration (1) adds `series_chapters.is_bonus` / `.origin`, (2) swaps the
//! full `UNIQUE (provider, external_id, chapter_number)` of 012 for a partial
//! unique index over main rows only (`WHERE NOT is_bonus`) and (3) re-keys
//! `series_chapters` **and** `user_chapter_progress` from the MangaUpdates
//! identity key to the key of the winning official binding.
//!
//! `sqlx::migrate!()` alone is not enough here: at startup the tables are empty,
//! so the re-key is a no-op. The test therefore seeds prod-shaped data and then
//! replays the file statement by statement, which also proves idempotency. The
//! file must stay free of functions/triggers and of semicolons inside comments
//! and literals - `statements()` below relies on that.

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres as PostgresImage;

const MIGRATION_028: &str = include_str!("../migrations/028_chapter_ownership.sql");

/// PostgreSQL default name for the inline `UNIQUE` of `012_series_chapters.sql`,
/// asserted against `pg_constraint` before 028 runs.
const FULL_UNIQUE_CONSTRAINT: &str = "series_chapters_provider_external_id_chapter_number_key";
/// Partial unique index created by 028; writers must pass
/// `ON CONFLICT (provider, external_id, chapter_number) WHERE NOT is_bonus`.
const MAIN_UNIQUE_INDEX: &str = "series_chapters_main_unique";

/// Title WITH a supported binding -> chapters must end up under ('bilibili', 'mc32437').
const MU_BOUND: &str = "12600089717";
/// Title WITHOUT any binding -> must stay under MangaUpdates.
const MU_UNBOUND: &str = "12600089999";

/// All embedded migrations below version 28 - the schema prod had before 028.
fn pre_028_migrator() -> sqlx::migrate::Migrator {
    let all = sqlx::migrate!("./migrations");
    sqlx::migrate::Migrator {
        migrations: Cow::Owned(
            all.migrations
                .iter()
                .filter(|m| m.version < 28)
                .cloned()
                .collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    }
}

async fn start_container() -> (testcontainers::ContainerAsync<PostgresImage>, PgPool) {
    let container = PostgresImage::default()
        .with_user("test")
        .with_password("test")
        .with_db_name("test")
        .start()
        .await
        .expect("Failed to start postgres container");

    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://test:test@{host}:{port}/test");

    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to postgres");

    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgcrypto")
        .execute(&pool)
        .await
        .expect("Failed to create pgcrypto extension");

    (container, pool)
}

/// Fresh pg17 container with the full migrator applied (028 included, no-op on
/// empty tables) - the replay tests seed after this.
async fn new_pool() -> (testcontainers::ContainerAsync<PostgresImage>, PgPool) {
    let (container, pool) = start_container().await;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    (container, pool)
}

/// Fresh pg17 container stopped right before 028, so the pre-migration schema
/// (full UNIQUE, no is_bonus/origin) can be asserted and the file replayed.
async fn new_pool_pre_028() -> (testcontainers::ContainerAsync<PostgresImage>, PgPool) {
    let (container, pool) = start_container().await;
    pre_028_migrator()
        .run(&pool)
        .await
        .expect("pre-028 migrations must apply");
    (container, pool)
}

/// Split the migration file into statements. Comments-only chunks are dropped,
/// so an executable chunk is never pure SQL comments.
fn statements(file: &str) -> Vec<String> {
    file.split(';')
        .map(|chunk| {
            chunk
                .lines()
                .filter(|line| !line.trim_start().starts_with("--"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .map(|chunk| chunk.trim().to_string())
        .filter(|chunk| !chunk.is_empty())
        .collect()
}

async fn apply_migration(pool: &PgPool) -> usize {
    let stmts = statements(MIGRATION_028);
    let count = stmts.len();
    for stmt in &stmts {
        sqlx::query(stmt)
            .execute(pool)
            .await
            .expect("028 statement must apply");
    }
    count
}

async fn seed_user(pool: &PgPool) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ('migration028', '<SECRET: MailDetector>', 'fakehash', 'user') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("insert user")
}

async fn seed_media(pool: &PgPool, external_id: &str, media_type: &str) {
    sqlx::query(
        "INSERT INTO media_items (provider, external_id, media_type, title, chapters) \
         VALUES ('mangaupdates', $1, $2, 'Yongsa Party', 271)",
    )
    .bind(external_id)
    .bind(media_type)
    .execute(pool)
    .await
    .expect("seed media_items");
}

async fn seed_binding(pool: &PgPool, external_id: &str, source: &str, source_id: &str) {
    sqlx::query(
        "INSERT INTO media_source_ids (provider, external_id, source, source_id) \
         VALUES ('mangaupdates', $1, $2, $3)",
    )
    .bind(external_id)
    .bind(source)
    .bind(source_id)
    .execute(pool)
    .await
    .expect("seed media_source_ids");
}

/// One chapter row under the MangaUpdates key; `read` rows also carry `read_at`
/// so the test can prove the flag travels with the re-keyed row.
async fn seed_chapter(pool: &PgPool, external_id: &str, chapter: i32, read: bool) {
    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number, read, read_at) \
         VALUES ('mangaupdates', $1, $2, $3, CASE WHEN $3 THEN NOW() ELSE NULL END)",
    )
    .bind(external_id)
    .bind(chapter)
    .bind(read)
    .execute(pool)
    .await
    .expect("seed series_chapters");
}

async fn seed_progress(pool: &PgPool, user_id: uuid::Uuid, external_id: &str, chapter: i32) {
    sqlx::query(
        "INSERT INTO user_chapter_progress \
             (user_id, provider, external_id, chapter_number, read, read_at) \
         VALUES ($1, 'mangaupdates', $2, $3, TRUE, NOW())",
    )
    .bind(user_id)
    .bind(external_id)
    .bind(chapter)
    .execute(pool)
    .await
    .expect("seed user_chapter_progress");
}

/// id + key + read + origin + is_bonus of every chapter row, in a stable order.
type ChapterSnapshot = Vec<(
    i64,
    String,
    String,
    i32,
    bool,
    Option<DateTime<Utc>>,
    String,
    bool,
)>;

async fn chapter_snapshot(pool: &PgPool) -> ChapterSnapshot {
    sqlx::query_as(
        "SELECT id, provider, external_id, chapter_number, read, read_at, origin, is_bonus \
         FROM series_chapters ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("read series_chapters")
}

/// id + key + read of every chapter row - readable before 028 added the two
/// new columns, so the pre/post comparison spans the whole migration.
type KeySnapshot = Vec<(i64, String, String, i32, bool, Option<DateTime<Utc>>)>;

async fn chapter_keys(pool: &PgPool) -> KeySnapshot {
    sqlx::query_as(
        "SELECT id, provider, external_id, chapter_number, read, read_at \
         FROM series_chapters ORDER BY id",
    )
    .fetch_all(pool)
    .await
    .expect("read series_chapters keys")
}

async fn progress_snapshot(pool: &PgPool) -> Vec<(String, String, i32, bool)> {
    sqlx::query_as(
        "SELECT provider, external_id, chapter_number, read \
         FROM user_chapter_progress ORDER BY provider, external_id, chapter_number",
    )
    .fetch_all(pool)
    .await
    .expect("read user_chapter_progress")
}

#[tokio::test]
async fn rekey_moves_bound_title_and_progress_and_is_idempotent() {
    let (_container, pool) = new_pool_pre_028().await;

    // --- pre-028 schema, so the DROP name is verified factually ------------
    let full_unique: String = sqlx::query_scalar(
        "SELECT conname FROM pg_constraint \
         WHERE conrelid = 'series_chapters'::regclass AND contype = 'u'",
    )
    .fetch_one(&pool)
    .await
    .expect("the 012 UNIQUE constraint must exist before 028");
    assert_eq!(
        full_unique, FULL_UNIQUE_CONSTRAINT,
        "028 drops the constraint by this name - a rename in 012 would silently skip the DROP"
    );

    // --- seed prod-shaped data under MangaUpdates keys ---------------------
    let user_id = seed_user(&pool).await;
    seed_media(&pool, MU_BOUND, "manhua").await;
    seed_media(&pool, MU_UNBOUND, "manhwa").await;
    seed_binding(&pool, MU_BOUND, "bilibili", "mc32437").await;

    seed_chapter(&pool, MU_BOUND, 100, true).await;
    seed_chapter(&pool, MU_BOUND, 200, false).await;
    seed_chapter(&pool, MU_BOUND, 300, true).await;
    seed_chapter(&pool, MU_UNBOUND, 100, false).await;
    seed_progress(&pool, user_id, MU_BOUND, 100).await;
    seed_progress(&pool, user_id, MU_BOUND, 300).await;

    let before = chapter_keys(&pool).await;
    assert_eq!(before.len(), 4);
    assert!(before.iter().all(|row| row.1 == "mangaupdates"));

    // --- run 1: the migration itself ---------------------------------------
    assert!(
        apply_migration(&pool).await >= 6,
        "028 must contain ALTERs, index DDL and both re-key UPDATEs"
    );

    let full_unique_left: i64 = sqlx::query_scalar(
        "SELECT count(*)::bigint FROM pg_constraint \
         WHERE conrelid = 'series_chapters'::regclass AND contype = 'u'",
    )
    .fetch_one(&pool)
    .await
    .expect("read pg_constraint");
    assert_eq!(full_unique_left, 0, "the full UNIQUE must be gone");
    let indexdef: String = sqlx::query_scalar(
        "SELECT indexdef FROM pg_indexes \
         WHERE schemaname = current_schema() AND indexname = $1",
    )
    .bind(MAIN_UNIQUE_INDEX)
    .fetch_one(&pool)
    .await
    .expect("028 partial unique index must exist");
    assert!(
        indexdef.contains("NOT is_bonus"),
        "the index must cover main rows only, got: {indexdef}"
    );

    let after = chapter_snapshot(&pool).await;
    let after_keys: Vec<_> = after
        .iter()
        .map(|row| (row.0, row.1.clone(), row.2.clone(), row.3, row.4, row.5))
        .collect();
    assert_ne!(before, after_keys, "the first run must move bound rows");

    let moved: Vec<_> = after
        .iter()
        .filter(|row| row.1 == "bilibili" && row.2 == "mc32437")
        .collect();
    assert_eq!(
        moved.len(),
        3,
        "all three chapters of the bound title moved"
    );
    assert_eq!(
        moved.iter().map(|row| row.3).collect::<Vec<_>>(),
        vec![100, 200, 300],
        "chapter numbers must not change"
    );
    assert_eq!(
        moved.iter().map(|row| row.4).collect::<Vec<_>>(),
        vec![true, false, true],
        "read flags must travel with the row"
    );
    for row in &moved {
        assert_eq!(row.6, "mu", "re-keyed rows keep their MU provenance");
        assert!(!row.7, "re-keyed rows are main chapters, never bonus");
    }
    assert!(
        moved.iter().any(|row| row.5.is_some()),
        "read_at must travel with the row"
    );
    assert!(
        !after
            .iter()
            .any(|row| row.1 == "mangaupdates" && row.2 == MU_BOUND),
        "no chapter row may stay under the bound MangaUpdates key"
    );
    let stayed: Vec<_> = after
        .iter()
        .filter(|row| row.1 == "mangaupdates" && row.2 == MU_UNBOUND)
        .collect();
    assert_eq!(
        stayed.len(),
        1,
        "a title without binding keeps its MangaUpdates key"
    );

    let after_progress = progress_snapshot(&pool).await;
    assert_eq!(
        after_progress,
        vec![
            ("bilibili".to_string(), "mc32437".to_string(), 100, true),
            ("bilibili".to_string(), "mc32437".to_string(), 300, true),
        ],
        "user_chapter_progress must move under the same predicate, or count_read joins to nothing"
    );

    // --- run 2: idempotency -------------------------------------------------
    apply_migration(&pool).await;
    assert_eq!(
        chapter_snapshot(&pool).await,
        after,
        "a second run of the file must change nothing"
    );
    assert_eq!(
        progress_snapshot(&pool).await,
        after_progress,
        "a second run of the file must not change user_chapter_progress"
    );

    // --- run 3: idempotency -------------------------------------------------
    apply_migration(&pool).await;
    assert_eq!(
        chapter_snapshot(&pool).await,
        after,
        "a third run of the file must change nothing"
    );
    assert_eq!(
        progress_snapshot(&pool).await,
        after_progress,
        "a third run of the file must not change user_chapter_progress"
    );

    // --- partial unique index contract -------------------------------------
    // A bonus episode may repeat a number a main chapter already uses
    // (mc32437: 297 episodes / 281 unique main numbers).
    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number, is_bonus) \
         VALUES ('bilibili', 'mc32437', 100, TRUE)",
    )
    .execute(&pool)
    .await
    .expect("a bonus row must be allowed to duplicate a main chapter number");

    // A second main row with the same number must still fail.
    let dup = sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number) \
         VALUES ('bilibili', 'mc32437', 100)",
    )
    .execute(&pool)
    .await;
    assert!(
        dup.is_err(),
        "main rows must stay unique - the partial index is the only guard left"
    );

    // Writers must name the predicate to infer the partial index.
    sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number) \
         VALUES ('bilibili', 'mc32437', 100) \
         ON CONFLICT (provider, external_id, chapter_number) WHERE NOT is_bonus DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect("ON CONFLICT with the partial predicate must be inferred");

    // Without the predicate Postgres cannot infer any unique index (42P10):
    // batches 5-6 must add `WHERE NOT is_bonus` to every series_chapters
    // ON CONFLICT (chapter_meta_enrich.rs, chapters.rs).
    let bare = sqlx::query(
        "INSERT INTO series_chapters (provider, external_id, chapter_number) \
         VALUES ('bilibili', 'mc32437', 400) \
         ON CONFLICT (provider, external_id, chapter_number) DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect_err("bare ON CONFLICT must not silently keep working after 028");
    assert_eq!(
        bare.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("42P10"),
        "expected 'no unique or exclusion constraint matching the ON CONFLICT specification'"
    );
}

#[tokio::test]
async fn non_meta_binding_keeps_mangaupdates_key() {
    let (_container, pool) = new_pool().await;

    let user_id = seed_user(&pool).await;
    seed_media(&pool, MU_BOUND, "manhua").await;
    // kuaikan has no arm in fetch_source -> Ok(OfficialMeta::default()), so it
    // must not own the chapter rows (design :84-86).
    seed_binding(&pool, MU_BOUND, "kuaikan", "mc60544").await;

    seed_chapter(&pool, MU_BOUND, 100, true).await;
    seed_chapter(&pool, MU_BOUND, 200, false).await;
    seed_progress(&pool, user_id, MU_BOUND, 100).await;

    apply_migration(&pool).await;

    let rows = chapter_snapshot(&pool).await;
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.1 == "mangaupdates" && row.2 == MU_BOUND),
        "a binding to a source without meta support leaves the rows on the MU key"
    );
    assert_eq!(
        progress_snapshot(&pool).await,
        vec![("mangaupdates".to_string(), MU_BOUND.to_string(), 100, true)],
        "progress must not move either"
    );
}

#[tokio::test]
async fn two_bindings_pick_supports_meta_source() {
    let (_container, pool) = new_pool().await;

    let user_id = seed_user(&pool).await;
    seed_media(&pool, MU_BOUND, "manhua").await;
    // manhua chain is ["kuaikan", "bilibili"]; kuaikan loses on supports_meta.
    seed_binding(&pool, MU_BOUND, "kuaikan", "mc60544").await;
    seed_binding(&pool, MU_BOUND, "bilibili", "mc32437").await;

    seed_chapter(&pool, MU_BOUND, 100, false).await;
    seed_progress(&pool, user_id, MU_BOUND, 100).await;

    apply_migration(&pool).await;

    let rows = chapter_snapshot(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "bilibili");
    assert_eq!(rows[0].2, "mc32437");
    assert_eq!(
        progress_snapshot(&pool).await,
        vec![("bilibili".to_string(), "mc32437".to_string(), 100, true)],
        "the same winner must be used for progress"
    );
}

#[tokio::test]
async fn binding_without_media_item_uses_global_fallback() {
    let (_container, pool) = new_pool().await;

    let user_id = seed_user(&pool).await;
    // No media_items row at all (drawer preview before the title is added):
    // the chain lookup has nothing to work with, so the global rank decides.
    seed_binding(&pool, MU_BOUND, "daum", "1234567").await;
    seed_chapter(&pool, MU_BOUND, 100, false).await;
    seed_progress(&pool, user_id, MU_BOUND, 100).await;

    apply_migration(&pool).await;

    let rows = chapter_snapshot(&pool).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].1, "daum");
    assert_eq!(rows[0].2, "1234567");
    assert_eq!(
        progress_snapshot(&pool).await,
        vec![("daum".to_string(), "1234567".to_string(), 100, true)]
    );
}
