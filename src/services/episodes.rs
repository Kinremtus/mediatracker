use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::services::external::mal::JikanEpisode;
use crate::services::external::mal::MalService;

/// Episode as returned to the template layer.
#[derive(Debug, Clone)]
pub struct StoredEpisode {
    pub episode_number: i32,
    pub title_en: Option<String>,
    pub title_ru: Option<String>,
    pub title_jp: Option<String>,
    pub air_date: Option<chrono::NaiveDate>,
    pub duration_minutes: Option<i32>,
    pub watched: bool,
}

/// Batched episode insert row: `(episode_number, title_en, title_jp, air_date, duration_minutes)`.
type EpisodeInsertRow = (
    i32,
    Option<String>,
    Option<String>,
    Option<chrono::NaiveDate>,
    Option<i32>,
);

/// Insert or update the *catalog* episodes for one anime. UNIQUE
/// (provider, external_id, episode_number) makes the operation
/// idempotent — re-fetching the same anime just refreshes titles and
/// air dates in place.
///
/// Catalog data is user-agnostic: `watched` lives in
/// `user_episode_progress`, not here. Stores under `provider = "mal"`,
/// `external_id = mal_id.to_string()`.
///
/// The whole refresh (batched episode upsert + `media_items.episodes`
/// denominator) runs in a single transaction: a failure mid-way can no
/// longer leave a partially written catalog.
pub async fn store_episodes_mal(
    pool: &PgPool,
    mal_id: i64,
    episodes: &[JikanEpisode],
) -> Result<(), sqlx::Error> {
    if episodes.is_empty() {
        return Ok(());
    }

    // Dedup by episode number (last write wins) so a single batched
    // INSERT ... ON CONFLICT cannot touch the same target row twice.
    let mut index: HashMap<i32, usize> = HashMap::with_capacity(episodes.len());
    let mut rows: Vec<EpisodeInsertRow> = Vec::with_capacity(episodes.len());
    for ep in episodes {
        let air_date = ep
            .aired
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.naive_utc().date());
        let duration_minutes = ep
            .duration
            .as_deref()
            .and_then(crate::services::external::mal::parse_duration_to_minutes);
        let record = (
            ep.mal_id,
            ep.title.clone(),
            ep.title_japanese.clone(),
            air_date,
            duration_minutes,
        );
        match index.get(&ep.mal_id) {
            Some(&i) => rows[i] = record,
            None => {
                index.insert(ep.mal_id, rows.len());
                rows.push(record);
            }
        }
    }

    let episode_numbers: Vec<i32> = rows.iter().map(|r| r.0).collect();
    let titles_en: Vec<Option<String>> = rows.iter().map(|r| r.1.clone()).collect();
    let titles_jp: Vec<Option<String>> = rows.iter().map(|r| r.2.clone()).collect();
    let air_dates: Vec<Option<chrono::NaiveDate>> = rows.iter().map(|r| r.3).collect();
    let durations: Vec<Option<i32>> = rows.iter().map(|r| r.4).collect();

    let external_id = mal_id.to_string();
    let mut tx = pool.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO anime_episodes
            (provider, external_id, episode_number, title_en, title_ru, title_jp, air_date, duration_minutes)
        SELECT 'mal', $1, u.episode_number, u.title_en, NULL, u.title_jp, u.air_date, u.duration_minutes
        FROM UNNEST($2::int[], $3::text[], $4::text[], $5::date[], $6::int[])
            AS u(episode_number, title_en, title_jp, air_date, duration_minutes)
        ON CONFLICT (provider, external_id, episode_number) DO UPDATE
        SET title_en = EXCLUDED.title_en,
            title_jp = EXCLUDED.title_jp,
            air_date = EXCLUDED.air_date,
            duration_minutes = EXCLUDED.duration_minutes,
            fetched_at = NOW()
        "#,
    )
    .bind(&external_id)
    .bind(&episode_numbers)
    .bind(&titles_en)
    .bind(&titles_jp)
    .bind(&air_dates)
    .bind(&durations)
    .execute(&mut *tx)
    .await?;

    // Sync media_items.episodes = MAX(episode_number) of what we just
    // stored, so the tracking card "X / Y эп." denominator matches
    // reality. Jikan's /anime/{id}/episodes endpoint doesn't return the
    // total; using the actual max from our own table is exact and
    // doesn't need an extra Jikan round-trip. If we wrote zero episodes
    // (e.g. movie), we returned early above and leave the column
    // untouched — Shikimori/TMDB may have populated it correctly on add.
    sqlx::query(
        r#"
        UPDATE media_items
        SET episodes = sub.max_ep
        FROM (
            SELECT MAX(episode_number) AS max_ep
            FROM anime_episodes
            WHERE provider = 'mal' AND external_id = $1
        ) AS sub
        WHERE media_items.provider = 'mal'
          AND media_items.external_id = $1
        "#,
    )
    .bind(&external_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

/// Read all episodes for one anime, sorted by number ascending, with
/// the calling user's own `watched` flag joined in. Catalog rows without
/// a progress row are reported as not watched.
pub async fn get_episodes(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
    user_id: Uuid,
    progress_provider: &str,
    progress_external_id: &str,
) -> Result<Vec<StoredEpisode>, sqlx::Error> {
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        i32,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        Option<i32>,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT ae.episode_number, ae.title_en, ae.title_ru, ae.title_jp,
               ae.air_date, ae.duration_minutes,
               COALESCE(up.watched, FALSE) AS watched
        FROM anime_episodes ae
        LEFT JOIN user_episode_progress up
               ON up.user_id = $3
              AND up.provider = $4
              AND up.external_id = $5
              AND up.episode_number = ae.episode_number
        WHERE ae.provider = $1 AND ae.external_id = $2
        ORDER BY ae.episode_number ASC
        "#,
    )
    .bind(provider)
    .bind(external_id)
    .bind(user_id)
    .bind(progress_provider)
    .bind(progress_external_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(
            |(
                episode_number,
                title_en,
                title_ru,
                title_jp,
                air_date,
                duration_minutes,
                watched,
            )| {
                StoredEpisode {
                    episode_number,
                    title_en,
                    title_ru,
                    title_jp,
                    air_date,
                    duration_minutes,
                    watched,
                }
            },
        )
        .collect())
}

/// Fetch episodes from Jikan and persist them (catalog only).
/// Returns the number of episodes stored (0 on failure).
pub async fn fetch_and_store_mal(
    pool: PgPool,
    service: &MalService,
    mal_id: i64,
) -> Result<usize, anyhow::Error> {
    let episodes = service.fetch_episodes(mal_id).await?;
    let count = episodes.len();
    store_episodes_mal(&pool, mal_id, &episodes).await?;
    Ok(count)
}

/// Look up `mal_id` for a media item. Required because we store
/// episodes keyed on MAL id (under `provider = "mal"`) regardless
/// of which provider the user originally added the anime with.
///
/// Returns `None` for anime that don't have a known MAL id
/// (rare for Shikimori-only entries).
pub async fn lookup_mal_id(
    pool: &PgPool,
    provider: &str,
    external_id: &str,
) -> Result<Option<i64>, sqlx::Error> {
    let row: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT mal_id FROM media_items WHERE provider = $1 AND external_id = $2")
            .bind(provider)
            .bind(external_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|(v,)| v))
}

/// Set the user's `watched` flag on one or more episodes.
///
/// All state lives in `user_episode_progress`; the catalog is only read
/// to discover which episode numbers exist and to preserve the
/// bulk-fill semantics relative to the catalog:
///
/// **Bulk-fill semantics on watch**: when `watched = true`, every
/// catalog episode with `episode_number <= episode_number` is marked
/// watched. This matches the standard "mark ep 200 watched → ep 1..200
/// watched" UX of MAL / AniList / Shikimori — users don't want to click
/// 200 checkboxes after binging a long series.
///
/// **Reverse bulk-fill on unwatch**: when `watched = false`, every
/// catalog episode with `episode_number >= episode_number` is marked
/// unwatched. Un-checking is the mirror of checking: if I "haven't seen
/// this one yet" I haven't seen anything past it either, so a single
/// click rolls progress back.
///
/// The write is a single `INSERT ... SELECT ... ON CONFLICT DO UPDATE`,
/// so repeated calls are idempotent (no duplicate rows) and there is no
/// SELECT-then-UPDATE race.
///
/// Returns `true` if at least one catalog row was touched (i.e. the
/// target episode exists in the DB for this MAL id), `false` otherwise.
pub async fn set_watched(
    pool: &PgPool,
    user_id: Uuid,
    progress_provider: &str,
    progress_external_id: &str,
    mal_id: i64,
    episode_number: i32,
    watched: bool,
) -> Result<bool, sqlx::Error> {
    let external_id = mal_id.to_string();
    let result = if watched {
        sqlx::query(
            r#"
            INSERT INTO user_episode_progress
                (user_id, provider, external_id, episode_number, watched, watched_at)
            SELECT $1, $2, $3, ae.episode_number, TRUE, NOW()
            FROM anime_episodes ae
            WHERE ae.provider = 'mal'
              AND ae.external_id = $4
              AND ae.episode_number <= $5
            ON CONFLICT (user_id, provider, external_id, episode_number) DO UPDATE
            SET watched = TRUE,
                watched_at = NOW()
            "#,
        )
        .bind(user_id)
        .bind(progress_provider)
        .bind(progress_external_id)
        .bind(&external_id)
        .bind(episode_number)
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            r#"
            UPDATE user_episode_progress
            SET watched = FALSE,
                watched_at = NULL
            WHERE user_id = $1
              AND provider = $2
              AND external_id = $3
              AND episode_number >= $4
            "#,
        )
        .bind(user_id)
        .bind(progress_provider)
        .bind(progress_external_id)
        .bind(episode_number)
        .execute(pool)
        .await?
    };
    Ok(result.rows_affected() > 0)
}

/// Returns `(episode_number, watched)` pairs for every catalog episode
/// of an anime, ordered by `episode_number` ASC, using this user's own
/// progress. Used by the toggle endpoint to broadcast authoritative
/// state to the drawer so all visible checkboxes stay in sync with the
/// DB (bulk-fill on watch can flip many rows in one go).
pub async fn get_episode_states(
    pool: &PgPool,
    user_id: Uuid,
    progress_provider: &str,
    progress_external_id: &str,
    mal_id: i64,
) -> Result<Vec<(i32, bool)>, sqlx::Error> {
    let rows: Vec<(i32, bool)> = sqlx::query_as(
        r#"
        SELECT ae.episode_number, COALESCE(up.watched, FALSE) AS watched
        FROM anime_episodes ae
        LEFT JOIN user_episode_progress up
               ON up.user_id = $1
              AND up.provider = $2
              AND up.external_id = $3
              AND up.episode_number = ae.episode_number
        WHERE ae.provider = 'mal'
          AND ae.external_id = $4
        ORDER BY ae.episode_number ASC
        "#,
    )
    .bind(user_id)
    .bind(progress_provider)
    .bind(progress_external_id)
    .bind(mal_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Highest episode number currently marked watched by this user for an
/// anime. Returns 0 if nothing is watched (or no progress exists).
pub async fn count_watched(
    pool: &PgPool,
    user_id: Uuid,
    progress_provider: &str,
    progress_external_id: &str,
    mal_id: i64,
) -> Result<i32, sqlx::Error> {
    let _ = mal_id;
    let row: (Option<i32>,) = sqlx::query_as(
        r#"
        SELECT MAX(episode_number)
        FROM user_episode_progress
        WHERE user_id = $1
          AND provider = $2
          AND external_id = $3
          AND watched = TRUE
        "#,
    )
    .bind(user_id)
    .bind(progress_provider)
    .bind(progress_external_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0.unwrap_or(0))
}

/// Bumps `tracking_entries.progress` to at least `watched_count`.
/// Uses `GREATEST(progress, $1)` so it never regresses — un-checking
/// the highest episode doesn't drop your progress, you'd have to do
/// that manually with the +1/-1 buttons.
pub async fn update_progress_from_watched(
    pool: &PgPool,
    user_id: Uuid,
    media_id: Uuid,
    watched_count: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE tracking_entries
        SET progress = GREATEST(progress, $1),
            updated_at = NOW()
        WHERE user_id = $2
          AND media_id = $3
        "#,
    )
    .bind(watched_count)
    .bind(user_id)
    .bind(media_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read a single episode by (mal_id, episode_number) with this user's
/// own `watched` flag. Returns `None` if the catalog row doesn't exist.
/// Used by the toggle endpoint to render the updated row HTML.
pub async fn get_episode(
    pool: &PgPool,
    mal_id: i64,
    episode_number: i32,
    user_id: Uuid,
    progress_provider: &str,
    progress_external_id: &str,
) -> Result<Option<StoredEpisode>, sqlx::Error> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        i32,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<chrono::NaiveDate>,
        Option<i32>,
        bool,
    )> = sqlx::query_as(
        r#"
        SELECT ae.episode_number, ae.title_en, ae.title_ru, ae.title_jp,
               ae.air_date, ae.duration_minutes,
               COALESCE(up.watched, FALSE) AS watched
        FROM anime_episodes ae
        LEFT JOIN user_episode_progress up
               ON up.user_id = $3
              AND up.provider = $4
              AND up.external_id = $5
              AND up.episode_number = ae.episode_number
        WHERE ae.provider = 'mal'
          AND ae.external_id = $1
          AND ae.episode_number = $2
        "#,
    )
    .bind(mal_id.to_string())
    .bind(episode_number)
    .bind(user_id)
    .bind(progress_provider)
    .bind(progress_external_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(
        |(episode_number, title_en, title_ru, title_jp, air_date, duration_minutes, watched)| {
            StoredEpisode {
                episode_number,
                title_en,
                title_ru,
                title_jp,
                air_date,
                duration_minutes,
                watched,
            }
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_jikan_episode_json() {
        // Real response shape from Jikan v4 /anime/{id}/episodes.
        let json = r#"{
            "data": [{
                "mal_id": 1,
                "url": "https://myanimelist.net/anime/21/One_Piece/episode/1",
                "title": "I'm Luffy! The Man Who's Gonna Be King of the Pirates!",
                "title_japanese": "俺はルフィ！海賊王になる男だ！",
                "aired": "1999-10-20T00:00:00+00:00",
                "score": 4.1,
                "filler": false,
                "recap": false,
                "forum_url": "https://myanimelist.net/forum/?topicid=43183"
            }],
            "pagination": {
                "last_visible_page": 12,
                "has_next_page": true
            }
        }"#;
        #[derive(serde::Deserialize)]
        struct Resp {
            data: Vec<JikanEpisode>,
        }
        let resp: Resp = serde_json::from_str(json).unwrap();
        assert_eq!(resp.data.len(), 1);
        let ep = &resp.data[0];
        assert_eq!(ep.mal_id, 1);
        assert!(ep.title.as_deref().unwrap().starts_with("I'm Luffy"));
        assert!(ep.aired.is_some());
        assert!(
            ep.duration.is_none(),
            "Jikan episodes list has no duration field"
        );
    }
}
