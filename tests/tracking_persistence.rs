mod common;

use mediatracker::models::media_item::CreateMediaItem;
use mediatracker::models::tracking_entry::UpdateTracking;
use mediatracker::services::tracking::TrackingService;
use uuid::Uuid;

const FIXTURE_USERNAME: &str = "test_persistence_user";
const FIXTURE_EMAIL: &str = "test@example.com";

async fn setup() -> (common::TestContext, Uuid) {
    let ctx = common::TestContext::new().await;

    sqlx::query("DELETE FROM users WHERE username = $1")
        .bind(FIXTURE_USERNAME)
        .execute(&ctx.pool)
        .await
        .expect("delete fixture user");

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ($1, $2, 'fakehash_for_persistence_test', 'user') RETURNING id",
    )
    .bind(FIXTURE_USERNAME)
    .bind(FIXTURE_EMAIL)
    .fetch_one(&ctx.pool)
    .await
    .expect("create fixture user");

    (ctx, user_id)
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

#[tokio::test]
async fn media_items_has_mal_id_and_shikimori_id_columns() {
    let (ctx, _) = setup().await;

    let (mal_count, shiki_count): (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*)::bigint FROM information_schema.columns
              WHERE table_name = 'media_items' AND column_name = 'mal_id'),
            (SELECT COUNT(*)::bigint FROM information_schema.columns
              WHERE table_name = 'media_items' AND column_name = 'shikimori_id')",
    )
    .fetch_one(&ctx.pool)
    .await
    .expect("query information_schema");

    assert_eq!(
        mal_count, 1,
        "media_items.mal_id column is missing"
    );
    assert_eq!(
        shiki_count, 1,
        "media_items.shikimori_id column is missing"
    );
}

#[tokio::test]
async fn add_to_list_persists_mal_id_and_shikimori_id() {
    let (ctx, user_id) = setup().await;
    let svc = TrackingService::new(ctx.pool.clone());
    let media = fixture_mal_anime();

    svc.add_to_list(user_id, &media, "in_progress")
        .await
        .expect("add_to_list must succeed");

    let (mal_id, shikimori_id, provider, external_id): (Option<i64>, Option<i64>, String, String) =
        sqlx::query_as(
            "SELECT mal_id, shikimori_id, provider, external_id \
             FROM media_items WHERE provider = $1 AND external_id = $2",
        )
        .bind(&media.provider)
        .bind(&media.external_id)
        .fetch_one(&ctx.pool)
        .await
        .expect("media_items row exists");

    assert_eq!(mal_id, Some(21), "mal_id not persisted by add_to_list");
    assert_eq!(shikimori_id, None, "shikimori_id should be None for MAL-sourced fixture");
    assert_eq!(provider, "mal");
    assert_eq!(external_id, "21");
}

#[tokio::test]
async fn add_to_list_persists_shikimori_id_when_present() {
    let (ctx, user_id) = setup().await;
    let svc = TrackingService::new(ctx.pool.clone());

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
    .fetch_one(&ctx.pool)
    .await
    .expect("media_items row exists");

    assert_eq!(mal_id, Some(21));
    assert_eq!(shikimori_id, Some(21));
}

#[tokio::test]
async fn update_entry_decodes_numeric_rating_to_f64() {
    let (ctx, user_id) = setup().await;
    let svc = TrackingService::new(ctx.pool.clone());

    let media = fixture_mal_anime();
    svc.add_to_list(user_id, &media, "in_progress")
        .await
        .expect("add_to_list");
    let entry_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM tracking_entries WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("tracking entry exists");
    sqlx::query("UPDATE tracking_entries SET rating = 9.0 WHERE id = $1")
        .bind(entry_id)
        .execute(&ctx.pool)
        .await
        .expect("set rating");

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
        "rating must be preserved by update_entry"
    );

    let (db_progress, db_rating): (i32, Option<String>) = sqlx::query_as(
        "SELECT progress, rating::text FROM tracking_entries WHERE id = $1",
    )
    .bind(entry_id)
    .fetch_one(&ctx.pool)
    .await
    .expect("read back");
    assert_eq!(db_progress, 1);
    assert_eq!(
        db_rating.expect("rating set"),
        "9.0",
        "rating must be unchanged in DB"
    );
}

#[tokio::test]
async fn update_entry_with_null_rating_succeeds() {
    let (ctx, user_id) = setup().await;
    let svc = TrackingService::new(ctx.pool.clone());

    svc.add_to_list(user_id, &fixture_mal_anime(), "planned")
        .await
        .expect("add_to_list");
    let entry_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM tracking_entries WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&ctx.pool)
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
