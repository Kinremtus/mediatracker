//! MAL -> Shikimori identity resolver.
//!
//! Anime added through MAL have `provider = 'mal'` and `shikimori_id IS NULL`,
//! so they never match `release_schedule` rows (which are Shikimori-only).
//! This module finds tracked anime that have a MAL id but no Shikimori id and
//! fills in `media_items.shikimori_id`.
//!
//! The SQL finder is separated from the HTTP orchestration so the finder can be
//! integration-tested against a real Postgres without any network, and the
//! candidate-pick policy is a pure function that is unit-tested.

use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::services::external::shikimori::{ShikimoriIdCandidate, ShikimoriService};

/// Maximum number of tracked anime resolved per refresh pass. Shikimori's API
/// budget is ~5 rps / 90 rpm; a small batch leaves headroom for the calendar
/// fetch and the next pass retries the rest.
pub const RESOLVER_BATCH: i64 = 10;

/// Pause between candidates so a full batch stays well under the rate limit.
const PACING: Duration = Duration::from_millis(1000);

/// One unresolved tracked anime row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedAnime {
    pub id: Uuid,
    pub mal_id: i64,
    pub title: String,
}

/// SQL-only finder: tracked anime that have a MAL id but no Shikimori id yet.
pub async fn find_unresolved_tracked_anime(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<UnresolvedAnime>, sqlx::Error> {
    let rows: Vec<(Uuid, i64, String)> = sqlx::query_as(
        r#"
        SELECT DISTINCT m.id, m.mal_id, m.title
        FROM media_items m
        JOIN tracking_entries t ON t.media_id = m.id
        WHERE m.media_type = 'anime'
          AND m.mal_id IS NOT NULL
          AND m.shikimori_id IS NULL
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, mal_id, title)| UnresolvedAnime { id, mal_id, title })
        .collect())
}

/// Pure candidate pick: the first Shikimori id whose GraphQL `malId` equals
/// the target MAL id *exactly* (no fuzzy matching).
pub fn pick_candidate_by_mal_id(candidates: &[ShikimoriIdCandidate], mal_id: i64) -> Option<i64> {
    candidates
        .iter()
        .find(|c| c.mal_id == Some(mal_id))
        .map(|c| c.shikimori_id)
}

/// Resolve up to [`RESOLVER_BATCH`] tracked MAL anime, best effort.
///
/// For each candidate:
/// 1. Probe `GET /api/animes/{mal_id}` — if the returned anime's `mal_id`
///    equals our MAL id, the Shikimori and MAL ids coincide; store it.
/// 2. Otherwise GraphQL-search by title and strictly match `malId`.
///
/// Per-item failures are logged and skipped; the next pass retries.
pub async fn resolve_tracked_anime(
    pool: &PgPool,
    shikimori: &ShikimoriService,
) -> Result<u32, anyhow::Error> {
    let candidates = find_unresolved_tracked_anime(pool, RESOLVER_BATCH).await?;
    if candidates.is_empty() {
        return Ok(0);
    }

    let mut resolved = 0u32;
    for candidate in &candidates {
        let shikimori_id = match probe_same_id(shikimori, candidate).await {
            Ok(Some(id)) => Some(id),
            Ok(None) => match shikimori
                .graphql_search_with_mal(&candidate.title, 10)
                .await
            {
                Ok(found) => pick_candidate_by_mal_id(&found, candidate.mal_id),
                Err(error) => {
                    tracing::warn!(
                        mal_id = candidate.mal_id,
                        error = %error,
                        "anime_identity: GraphQL search failed"
                    );
                    None
                }
            },
            Err(error) => {
                tracing::warn!(
                    mal_id = candidate.mal_id,
                    error = %error,
                    "anime_identity: direct probe failed"
                );
                None
            }
        };

        if let Some(shikimori_id) = shikimori_id {
            match sqlx::query("UPDATE media_items SET shikimori_id = $1 WHERE id = $2")
                .bind(shikimori_id)
                .bind(candidate.id)
                .execute(pool)
                .await
            {
                Ok(_) => {
                    resolved += 1;
                    tracing::info!(
                        mal_id = candidate.mal_id,
                        shikimori_id,
                        "anime_identity: resolved MAL -> Shikimori"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        mal_id = candidate.mal_id,
                        error = %error,
                        "anime_identity: failed to persist shikimori_id"
                    );
                }
            }
        }

        tokio::time::sleep(PACING).await;
    }

    Ok(resolved)
}

/// Direct probe: try `candidate.mal_id` as a Shikimori id and confirm the
/// returned anime points back at the same MAL id.
async fn probe_same_id(
    shikimori: &ShikimoriService,
    candidate: &UnresolvedAnime,
) -> Result<Option<i64>, anyhow::Error> {
    match shikimori
        .fetch_mal_id_by_shikimori_id(candidate.mal_id)
        .await?
    {
        Some(returned) if returned == candidate.mal_id => Ok(Some(candidate.mal_id)),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(shikimori_id: i64, mal_id: Option<i64>) -> ShikimoriIdCandidate {
        ShikimoriIdCandidate {
            shikimori_id,
            mal_id,
        }
    }

    #[test]
    fn picks_exact_mal_id_match() {
        let candidates = vec![cand(10, Some(99)), cand(20, Some(21)), cand(30, None)];
        assert_eq!(pick_candidate_by_mal_id(&candidates, 21), Some(20));
    }

    #[test]
    fn returns_none_when_no_exact_match() {
        let candidates = vec![cand(10, Some(99)), cand(20, None)];
        assert_eq!(pick_candidate_by_mal_id(&candidates, 21), None);
    }

    #[test]
    fn ignores_null_mal_id() {
        let candidates = vec![cand(10, None)];
        assert_eq!(pick_candidate_by_mal_id(&candidates, 21), None);
    }
}
