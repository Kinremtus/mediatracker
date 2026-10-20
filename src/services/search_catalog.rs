//! Local search catalog: the DB-backed cache behind `/search`.
//!
//! A search is served from here when the same `(query_norm, media_type)` was
//! answered recently (a fresh `search_queries` row); otherwise the caller runs
//! the live search and calls [`store`]. The catalog never deletes rows by age:
//! a title that disappears from its source keeps its last known data.
//!
//! Only NON-empty live answers are recorded: if every upstream failed
//! transiently, caching the empty answer would hide the title for a whole TTL
//! window, so an empty result is simply not written and the next search tries
//! live again. The refresh worker only sees queries that once had results.

use std::time::Duration;

use sqlx::PgPool;
use tokio_util::sync::CancellationToken;

use crate::app_state::AppState;
use crate::models::media_item::CreateMediaItem;
use crate::services::official_resolve::official_source_for_media_type;
use crate::services::official_title::normalize_title;
use crate::services::release_schedule;

/// How long a recorded query is served from the catalog before the live
/// sources are consulted again (the refresh worker re-runs stale queries).
pub const QUERY_TTL_HOURS: i32 = 24;

/// How often the refresh loop wakes up.
const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Queries refreshed per pass (bounded so a big catalog never fires hundreds
/// of upstream searches at once).
const REFRESH_LIMIT: i64 = 50;

/// Advisory-lock key of the catalog refresh: its own key (the counts /
/// release-schedule cycle uses 42) so the two background passes never starve
/// each other.
const CATALOG_LOCK_ID: i64 = 43;

/// Defensive cap on one catalog answer (the grid paginates at 24 per page).
const CATALOG_LIMIT: i64 = 200;

/// One row of the query log, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct StaleQuery {
    pub query_text: String,
    pub media_type: String,
}

/// The catalog answer for a fresh query, or `None` when the query must run
/// live (never searched, stale, or the lookup itself failed upstream).
pub async fn lookup(
    db: &PgPool,
    media_type: &str,
    query: &str,
) -> Result<Option<Vec<CreateMediaItem>>, sqlx::Error> {
    let query_norm = normalize_title(query);
    if query_norm.is_empty() {
        // Nothing searchable in the query (punctuation only): an empty answer
        // without a live round-trip.
        return Ok(Some(Vec::new()));
    }
    let fresh: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM search_queries \
         WHERE query_norm = $1 AND media_type = $2 \
           AND fetched_at > NOW() - make_interval(hours => $3))",
    )
    .bind(&query_norm)
    .bind(media_type)
    .bind(QUERY_TTL_HOURS)
    .fetch_one(db)
    .await?;
    if !fresh {
        return Ok(None);
    }
    // Official hits first, then by score/name; a malformed payload row is
    // skipped instead of failing the whole answer.
    let rows: Vec<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM search_catalog \
         WHERE media_type = $1 AND title_norm LIKE '%' || $2 || '%' \
         ORDER BY is_official DESC, score DESC NULLS LAST, title ASC \
         LIMIT $3",
    )
    .bind(media_type)
    .bind(&query_norm)
    .bind(CATALOG_LIMIT)
    .fetch_all(db)
    .await?;
    let items = rows
        .into_iter()
        .filter_map(|(payload,)| match serde_json::from_value(payload) {
            Ok(item) => Some(item),
            Err(e) => {
                tracing::warn!(media_type, error = %e, "search catalog: unreadable payload row skipped");
                None
            }
        })
        .collect();
    Ok(Some(items))
}

/// One catalog row as a `CreateMediaItem`. The drawer renders official-source
/// previews from here: those providers have no details endpoint, so the
/// search-answer payload is the only card data there is.
pub async fn find_item(
    db: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Option<CreateMediaItem>, sqlx::Error> {
    let row: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT payload FROM search_catalog WHERE provider = $1 AND external_id = $2",
    )
    .bind(provider)
    .bind(external_id)
    .fetch_optional(db)
    .await?;
    Ok(row.and_then(|(payload,)| serde_json::from_value(payload).ok()))
}

/// Persist one live answer: upsert every hit and record the query as fresh.
///
/// Row identity is `(provider, external_id)` (migration 030): a provider id
/// denotes one work, so `media_type` is refreshed as display metadata on each
/// upsert instead of being part of the key. A type-scoped lookup can therefore
/// miss an item last seen under a different type — accepted: providers
/// classify an id once in practice, and the next same-type search re-upserts
/// the row.
/// An empty answer is not recorded (see the module docs). A failed row is
/// logged and skipped — the catalog must not lose the whole answer because one
/// payload could not be serialized.
pub async fn store(
    db: &PgPool,
    media_type: &str,
    query: &str,
    items: &[CreateMediaItem],
) -> Result<(), sqlx::Error> {
    if items.is_empty() {
        return Ok(());
    }
    let query_norm = normalize_title(query);
    if query_norm.is_empty() {
        return Ok(());
    }
    let officials = official_source_for_media_type(media_type);
    let mut tx = db.begin().await?;
    for item in items {
        let Ok(payload) = serde_json::to_value(item) else {
            tracing::warn!(
                provider = %item.provider,
                external_id = %item.external_id,
                "search catalog: payload serialization failed, row skipped"
            );
            continue;
        };
        let is_official = officials.contains(&item.provider.as_str());
        sqlx::query(
            "INSERT INTO search_catalog \
                (provider, external_id, media_type, title, title_norm, poster_url, \
                 year, score, is_official, payload, last_query, fetched_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW()) \
             ON CONFLICT (provider, external_id) DO UPDATE SET \
                media_type = EXCLUDED.media_type, \
                title = EXCLUDED.title, \
                title_norm = EXCLUDED.title_norm, \
                poster_url = EXCLUDED.poster_url, \
                year = EXCLUDED.year, \
                score = EXCLUDED.score, \
                is_official = EXCLUDED.is_official, \
                payload = EXCLUDED.payload, \
                last_query = EXCLUDED.last_query, \
                fetched_at = NOW()",
        )
        .bind(&item.provider)
        .bind(&item.external_id)
        .bind(media_type)
        .bind(&item.title)
        .bind(normalize_title(&item.title))
        .bind(item.poster_url.as_deref())
        .bind(item.year)
        .bind(item.score)
        .bind(is_official)
        .bind(&payload)
        .bind(&query_norm)
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO search_queries (query_norm, media_type, query_text, fetched_at) \
         VALUES ($1, $2, $3, NOW()) \
         ON CONFLICT (query_norm, media_type) DO UPDATE SET \
            query_text = EXCLUDED.query_text, fetched_at = NOW()",
    )
    .bind(&query_norm)
    .bind(media_type)
    .bind(query.trim())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The oldest recorded queries whose data is older than [`QUERY_TTL_HOURS`],
/// at most `limit` of them — the refresh worker's worklist.
pub async fn stale_queries(db: &PgPool, limit: i64) -> Result<Vec<StaleQuery>, sqlx::Error> {
    sqlx::query_as(
        "SELECT query_text, media_type FROM search_queries \
         WHERE fetched_at < NOW() - make_interval(hours => $1) \
         ORDER BY fetched_at ASC \
         LIMIT $2",
    )
    .bind(QUERY_TTL_HOURS)
    .bind(limit)
    .fetch_all(db)
    .await
}

/// Mark a refreshed query as fresh again (the refresh worker always bumps it,
/// even when the sources answered nothing, so a dead query cannot occupy the
/// oldest slot forever).
pub async fn touch_query(db: &PgPool, media_type: &str, query: &str) -> Result<(), sqlx::Error> {
    let query_norm = normalize_title(query);
    if query_norm.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE search_queries SET fetched_at = NOW() \
         WHERE query_norm = $1 AND media_type = $2",
    )
    .bind(&query_norm)
    .bind(media_type)
    .execute(db)
    .await
    .map(|_| ())
}

/// Apply one refreshed answer: upsert the non-empty hits and ALWAYS bump the
/// query — a query whose sources now answer nothing must not occupy the
/// oldest refresh slot forever. The catalog itself is never pruned.
pub async fn apply_refresh(
    db: &PgPool,
    media_type: &str,
    query_text: &str,
    items: &[CreateMediaItem],
) -> anyhow::Result<()> {
    if !items.is_empty() {
        store(db, media_type, query_text, items).await?;
    }
    touch_query(db, media_type, query_text).await?;
    Ok(())
}

/// Background refresh of the search catalog: every [`REFRESH_INTERVAL`] the
/// loop re-runs up to [`REFRESH_LIMIT`] stale queries (older than
/// [`QUERY_TTL_HOURS`]) through the live search, upserts their answers and
/// bumps them. Nothing is ever deleted. One replica runs the pass: it is
/// guarded by its own advisory lock ([`CATALOG_LOCK_ID`]).
pub async fn run_refresh_loop(state: AppState, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(REFRESH_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Consume the immediate first tick: the first pass runs after a full
    // period (startup already has enough to do).
    interval.tick().await;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("search catalog: refresh loop cancelled");
                break;
            }
            _ = interval.tick() => {
                let state = state.clone();
                let db = state.db.clone();
                let pass = release_schedule::with_advisory_lock(
                    &db,
                    CATALOG_LOCK_ID,
                    move || async move { refresh_stale(&state).await },
                )
                .await;
                match pass {
                    Ok(Some(count)) => {
                        tracing::info!(refreshed = count, "search catalog: refresh pass done");
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!(error = %e, "search catalog: refresh pass failed"),
                }
            }
        }
    }
}

/// One refresh pass over the stale queries (network I/O). Returns how many
/// queries were processed.
async fn refresh_stale(state: &AppState) -> anyhow::Result<usize> {
    let stale = stale_queries(&state.db, REFRESH_LIMIT).await?;
    if stale.is_empty() {
        return Ok(0);
    }
    tracing::info!(
        count = stale.len(),
        "search catalog: refreshing stale queries"
    );
    let mut refreshed = 0usize;
    for query in &stale {
        let items =
            crate::services::search::live_search(state, &query.media_type, &query.query_text).await;
        if let Err(e) = apply_refresh(&state.db, &query.media_type, &query.query_text, &items).await
        {
            tracing::warn!(
                error = %e,
                query = %query.query_text,
                "search catalog: refresh apply failed"
            );
        }
        refreshed += 1;
    }
    Ok(refreshed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_normalization_is_shared_with_the_title_index() {
        // The lookup builds `title_norm LIKE %query_norm%`, so both sides must
        // normalize the same way.
        assert_eq!(normalize_title("Lore Olympus"), "loreolympus");
        assert_eq!(normalize_title("  Lore   Olympus! "), "loreolympus");
        assert_eq!(normalize_title("進撃の巨人"), "進撃の巨人");
    }
}
