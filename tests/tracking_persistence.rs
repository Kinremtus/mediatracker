// Persistence roundtrip tests for TrackingService::add_to_list.
//
// These tests catch drift between the fields declared on
// `CreateMediaItem` and the columns actually INSERTed in
// `add_to_list` — the kind of drift that produced the production
// bug "column 'mal_id' does not exist" after migration 010 added
// `shikimori_id` but not its sibling `mal_id`.
//
// They are #[ignore] by default because they require a live
// Postgres. Run with:
//
//   TEST_DATABASE_URL=postgres://Kin@localhost/tracker_test \
//     cargo test --test tracking_persistence -- --ignored
//
// The test DB is auto-migrated. A fixture user is created (and
// dropped) per test run, so existing data is not touched.

use mediatracker::models::media_item::CreateMediaItem;
use mediatracker::models::tracking_entry::UpdateTracking;
use mediatracker::services::tracking::TrackingService;
use sqlx::PgPool;
use uuid::Uuid;

const FIXTURE_USERNAME: &str = "test_persistence_user";
const FIXTURE_EMAIL: &str = "test_persistence@example.com";

fn require_db_url() -> String {
    std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        panic!(
            "TEST_DATABASE_URL not set. Run with: \
             TEST_DATABASE_URL=postgres://Kin@localhost/tracker_test \
             cargo test --test tracking_persistence -- --ignored"
        );
    })
}

/// Connect, run migrations, create a fixture user, return pool + user id.
async fn setup() -> (PgPool, Uuid) {
    let url = require_db_url();
    let pool = PgPool::connect(&url).await.expect("connect to TEST_DATABASE_URL");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");

    // ON DELETE CASCADE clears tracking_entries/activity_log for us.
    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .execute(&pool)
        .await
        .expect("delete fixture user");

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash_for_persistence_test', 'user') RETURNING id",
    )
    .bind(FIXTURE_USERNAME)
    .bind(FIXTURE_EMAIL)
    .fetch_one(&pool)
    .await
    .expect("create fixture user");

    (pool, user_id)
}

fn fixture_mal_anime() -> CreateMediaItem {
    CreateMediaItem {
        provider: "mal".to_string(),
        external_id: "21".to_string(),
        media_type: "anime".to_string(),
        title: "ONE PIECE".to_string(),
        title_english: Some("One Piece".to_string()),
        title_russian: Some("Ван Пис".to_string()),
        episodes: Some(1100),
        score: Some(9.5),
        mal_id: Some(21),
        shikimori_id: None,
        ..Default::default()
    }
}

/// Schema regression test: catches missing-column bugs at the
/// migration level. Runs first because persistence tests below
/// are meaningless if these columns don't exist.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn media_items_has_mal_id_and_shikimori_id_columns() {
    let (pool, _) = setup().await;

    let (mal_count, shiki_count): (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*)::bigint FROM information_schema.columns
              WHERE table_name = 'media_items' AND column_name = 'mal_id'),
            (SELECT COUNT(*)::bigint FROM information_schema.columns
              WHERE table_name = 'media_items' AND column_name = 'shikimori_id')",
    )
    .fetch_one(&pool)
    .await
    .expect("query information_schema");

    assert_eq!(
        mal_count, 1,
        "media_items.mal_id column is missing — apply migrations/011_add_mal_id_to_media_items.sql"
    );
    assert_eq!(
        shiki_count, 1,
        "media_items.shikimori_id column is missing — apply migrations/010_shikimori_id_and_episodes.sql"
    );
}

/// Drift test: the bug we keep hitting. Every field that
/// CreateMediaItem declares should reach the row in media_items.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn add_to_list_persists_mal_id_and_shikimori_id() {
    let (pool, user_id) = setup().await;
    let svc = TrackingService::new(pool.clone());
    let media = fixture_mal_anime();

    svc.add_to_list(user_id, &media, "in_progress")
        .await
        .expect("add_to_list must succeed — check that the INSERT in \
                 TrackingService::add_to_list lists every CreateMediaItem field");

    let (mal_id, shikimori_id, provider, external_id): (Option<i64>, Option<i64>, String, String) =
        sqlx::query_as(
            "SELECT mal_id, shikimori_id, provider, external_id \
             FROM media_items WHERE provider = $1 AND external_id = $2",
        )
        .bind(&media.provider)
        .bind(&media.external_id)
        .fetch_one(&pool)
        .await
        .expect("media_items row exists");

    assert_eq!(mal_id, Some(21), "mal_id not persisted by add_to_list");
    assert_eq!(shikimori_id, None, "shikimori_id should be None for MAL-sourced fixture");
    assert_eq!(provider, "mal");
    assert_eq!(external_id, "21");
}

/// Same drift test for a Shikimori-sourced fixture: shikimori_id
/// is Some, mal_id is None (we don't always have MAL id back).
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn add_to_list_persists_shikimori_id_when_present() {
    let (pool, user_id) = setup().await;
    let svc = TrackingService::new(pool.clone());

    let media = CreateMediaItem {
        provider: "shikimori".to_string(),
        external_id: "21".to_string(),
        media_type: "anime".to_string(),
        title: "ONE PIECE".to_string(),
        mal_id: Some(21),
        shikimori_id: Some(21),
        ..Default::default()
    };

    svc.add_to_list(user_id, &media, "planned")
        .await
        .expect("add_to_list");

    let (mal_id, shikimori_id): (Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT mal_id, shikimori_id FROM media_items \
         WHERE provider = 'shikimori' AND external_id = '21'",
    )
    .fetch_one(&pool)
    .await
    .expect("media_items row exists");

    assert_eq!(mal_id, Some(21));
    assert_eq!(shikimori_id, Some(21));
}

/// Regression test: tracking_entries.rating is NUMERIC(2,1) in the
/// schema (so SQLx without the `bigdecimal` feature can't decode it
/// into Rust `f64` directly). update_entry used to do `RETURNING *`
/// which made `fetch_one::<TrackingEntry>` fail with
/// "mismatched types: ... Option<f64> ... not compatible with NUMERIC"
/// the moment any row had a rating set — even though the UPDATE
/// itself succeeded. The 500 made htmx_update_tracking fall through
/// to a 303 Redirect, leaving UI and DB inconsistent and producing
/// the "press + on a card and the page breaks" bug. Fix: cast
/// `rating::double precision AS rating` in the RETURNING list, same
/// as get_user_entries already does.
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn update_entry_decodes_numeric_rating_to_f64() {
    let (pool, user_id) = setup().await;
    let svc = TrackingService::new(pool.clone());

    // Add the entry, then set a rating via direct SQL (mimics the
    // user clicking a star in the drawer).
    let media = fixture_mal_anime();
    svc.add_to_list(user_id, &media, "in_progress")
        .await
        .expect("add_to_list");
    let entry_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM tracking_entries WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("tracking entry exists");
    sqlx::query("UPDATE tracking_entries SET rating = 9.0 WHERE id = $1")
        .bind(entry_id)
        .execute(&pool)
        .await
        .expect("set rating");

    // Now bump progress — this is exactly what the + button does.
    // Pre-fix: fetch_one fails on the NUMERIC rating column.
    let update = UpdateTracking {
        status: None,
        rating: None,
        progress: Some(1),
    };
    let returned = svc
        .update_entry(entry_id, user_id, &update)
        .await
        .expect("update_entry must decode NUMERIC rating to Option<f64>");

    assert_eq!(returned.progress, 1, "progress must be applied");
    assert_eq!(
        returned.rating,
        Some(9.0),
        "rating must be preserved (not reset, not corrupted) by update_entry"
    );

    // Verify in DB too — guards against update_entry silently
    // overwriting rating because of a wrong UPDATE column list.
    // Read rating as text to avoid pulling in the `bigdecimal` feature.
    let (db_progress, db_rating): (i32, Option<String>) = sqlx::query_as(
        "SELECT progress, rating::text FROM tracking_entries WHERE id = $1",
    )
    .bind(entry_id)
    .fetch_one(&pool)
    .await
    .expect("read back");
    assert_eq!(db_progress, 1);
    assert_eq!(
        db_rating.expect("rating set"),
        "9.0",
        "rating must be unchanged in DB"
    );
}

/// Companion test: when no rating is set, update_entry still works
/// (regression check for the cast-not-breaking-Option path).
#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL"]
async fn update_entry_with_null_rating_succeeds() {
    let (pool, user_id) = setup().await;
    let svc = TrackingService::new(pool.clone());

    svc.add_to_list(user_id, &fixture_mal_anime(), "planned")
        .await
        .expect("add_to_list");
    let entry_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM tracking_entries WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("entry exists");

    let update = UpdateTracking {
        status: Some("in_progress".to_string()),
        rating: None,
        progress: Some(5),
    };
    let returned = svc
        .update_entry(entry_id, user_id, &update)
        .await
        .expect("update_entry with null rating must succeed");
    assert_eq!(returned.progress, 5);
    assert_eq!(returned.rating, None);
    assert_eq!(returned.status, "in_progress");
}
