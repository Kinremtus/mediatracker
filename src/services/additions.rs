//! Lazy DB-first cache of game DLC / expansions ("additions").
//!
//! Mirrors the manga chapter pattern: the drawer section requests additions on
//! reveal; the handler reads `game_additions` first and only calls the game
//! provider when the cache is empty. See `routes::media::get_game_additions`.

use crate::services::external::AdditionRow;
use crate::services::external::igdb::IgdbService;
use crate::services::external::rawg::RawgService;
use sqlx::PgPool;

/// Read cached additions for one game, ordered by release date (oldest first,
/// undated last) then by name.
pub async fn get_additions(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Vec<AdditionRow>, sqlx::Error> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(String, String, String, Option<chrono::NaiveDate>)> = sqlx::query_as(
        r#"
        SELECT addition_external_id, name, kind, released
        FROM game_additions
        WHERE provider = $1 AND external_id = $2
        ORDER BY released ASC NULLS LAST, name ASC
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(addition_external_id, name, kind, released)| AdditionRow {
            addition_external_id,
            name,
            kind,
            released,
        })
        .collect())
}

/// Idempotent batch upsert. `(provider, external_id, addition_external_id)` is
/// the conflict key; name/kind/released are refreshed, and rows that vanished
/// from the provider are intentionally left in place.
pub async fn upsert_additions(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    rows: &[AdditionRow],
) -> Result<usize, sqlx::Error> {
    if rows.is_empty() {
        return Ok(0);
    }
    let addition_ids: Vec<String> = rows
        .iter()
        .map(|r| r.addition_external_id.clone())
        .collect();
    let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
    let kinds: Vec<String> = rows.iter().map(|r| r.kind.clone()).collect();
    let released: Vec<Option<chrono::NaiveDate>> = rows.iter().map(|r| r.released).collect();

    let result = sqlx::query(
        r#"
        INSERT INTO game_additions
            (provider, external_id, addition_external_id, name, kind, released)
        SELECT $1, $2, u.addition_external_id, u.name, u.kind, u.released
        FROM UNNEST($3::text[], $4::text[], $5::text[], $6::date[])
            AS u(addition_external_id, name, kind, released)
        ON CONFLICT (provider, external_id, addition_external_id) DO UPDATE
        SET name = EXCLUDED.name,
            kind = EXCLUDED.kind,
            released = EXCLUDED.released
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(&addition_ids)
    .bind(&names)
    .bind(&kinds)
    .bind(&released)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() as usize)
}

/// Fetch additions from the provider matching `provider` and cache them.
///
/// Unknown providers and provider/network failures yield `Ok(0)` with a
/// `warn!` log, so the drawer never degrades to an error response.
pub async fn fetch_and_store(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    rawg: &RawgService,
    igdb: &IgdbService,
) -> Result<usize, anyhow::Error> {
    let rows = match provider {
        "rawg" => match rawg.get_additions(external_id).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(external_id, error = %e, "RAWG additions fetch failed");
                return Ok(0);
            }
        },
        "igdb" => match igdb.get_additions(external_id).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(external_id, error = %e, "IGDB additions fetch failed");
                return Ok(0);
            }
        },
        _ => return Ok(0),
    };

    let count = upsert_additions(pool, provider, external_id, &rows).await?;
    Ok(count)
}
