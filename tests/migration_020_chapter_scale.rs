//! Regression test for `migrations/020_chapter_scale_x100.sql`.
//!
//! Prod hit Postgres error 23505 on the unique
//! `(provider, external_id, chapter_number)` index: the original single-step
//! `chapter_number = chapter_number * 10` moved row 10 -> 100 while row 100
//! still existed untouched. This test recreates that data shape by applying
//! every embedded migration below version 20, seeding colliding rows, then
//! running the full migrator (including 020): the two-step shift must apply
//! cleanly and rescale every value to the *100 scale.

use std::borrow::Cow;

use sqlx::PgPool;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres as PostgresImage;

const PROVIDER: &str = "mangaupdates";
const EXTERNAL_ID: &str = "migration_020_regression";

/// All embedded migrations with `version < 20`. Running this first recreates
/// the pre-020 schema/data state prod was in when migration 020 failed.
fn pre_020_migrator() -> sqlx::migrate::Migrator {
    let all = sqlx::migrate!("./migrations");
    sqlx::migrate::Migrator {
        migrations: Cow::Owned(
            all.migrations
                .iter()
                .filter(|m| m.version < 20)
                .cloned()
                .collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    }
}

#[tokio::test]
async fn migration_020_rescales_colliding_rows() {
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

    pre_020_migrator()
        .run(&pool)
        .await
        .expect("pre-020 migrations must apply");

    let (user_id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO users (username, email, password_hash, role) \
         VALUES ('migration020', 'migration020@example.com', 'fakehash', 'user') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("insert user");

    for n in 1..=10i32 {
        let chapter = n * 10; // 10, 20, ..., 100 on the old *10 scale
        sqlx::query(
            "INSERT INTO series_chapters (provider, external_id, chapter_number) \
             VALUES ($1, $2, $3)",
        )
        .bind(PROVIDER)
        .bind(EXTERNAL_ID)
        .bind(chapter)
        .execute(&pool)
        .await
        .expect("seed series_chapters");

        sqlx::query(
            "INSERT INTO user_chapter_progress \
                 (user_id, provider, external_id, chapter_number, read) \
             VALUES ($1, $2, $3, $4, TRUE)",
        )
        .bind(user_id)
        .bind(PROVIDER)
        .bind(EXTERNAL_ID)
        .bind(chapter)
        .execute(&pool)
        .await
        .expect("seed user_chapter_progress");
    }

    // Full migrator, including 020. On the old single-step SQL this returns
    // a 23505 unique violation; the two-step shift must succeed.
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("migration 020 must apply without a unique violation");

    let expected: Vec<i32> = (1..=10i32).map(|n| n * 100).collect();

    let catalog: Vec<i32> = sqlx::query_scalar(
        "SELECT chapter_number FROM series_chapters \
         WHERE provider = $1 AND external_id = $2 ORDER BY chapter_number",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_all(&pool)
    .await
    .expect("read series_chapters");
    assert_eq!(
        catalog, expected,
        "series_chapters must be rescaled to the *100 scale"
    );

    let progress: Vec<i32> = sqlx::query_scalar(
        "SELECT chapter_number FROM user_chapter_progress \
         WHERE provider = $1 AND external_id = $2 ORDER BY chapter_number",
    )
    .bind(PROVIDER)
    .bind(EXTERNAL_ID)
    .fetch_all(&pool)
    .await
    .expect("read user_chapter_progress");
    assert_eq!(
        progress, expected,
        "user_chapter_progress must be rescaled to the *100 scale"
    );
}
